//! Bouchaud Browser : navigateur graphique.
//!
//! Recupere le HTML (HTTP/1.1 + TLS, gzip/deflate/brotli), le parse en DOM, le
//! met en page (moteur `gui::web`) et le peint dans le framebuffer HD avec
//! defilement. Pages internes : `about:bouchaud`, `about:system`, `file:/...`.

use crate::gui::framebuffer as fb;
use crate::gui::web::{self, Page};
use crate::gui::window::clip;
use crate::arch::x86_64::rtc;
use crate::fs::ramfs;
use crate::kernel::timer;
use crate::users;
use alloc::format;
use alloc::string::{String, ToString};

const ADDR_H: usize = 11;

// Echappe le texte brut pour l'injecter dans du HTML synthetise.
fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c { '&' => o.push_str("&amp;"), '<' => o.push_str("&lt;"), '>' => o.push_str("&gt;"), _ => o.push(c) }
    }
    o
}

fn page_from_html(html: &[u8], base: &str, width: i32) -> Page {
    let dom = web::parse(html);
    web::layout(&dom, base, width)
}

/// Charge une URL et renvoie la page mise en page pour la largeur donnee.
pub(crate) fn open(url: &str, width: i32) -> Page {
    let width = width.max(80);
    if url == "about:bouchaud" {
        let html = format!(
            "<h1>Bouchaud OS</h1><p>OS souverain francais experimental.</p>\
             <p>Version : {} &mdash; noyau Rust no_std, bureau HD, pile reseau TLS 1.3.</p>\
             <h3>Pages</h3><ul><li><a href=\"about:system\">about:system</a></li>\
             <li><a href=\"file:/readme.txt\">file:/readme.txt</a></li>\
             <li><a href=\"https://example.com/\">https://example.com/</a></li>\
             <li><a href=\"https://www.google.com/\">https://www.google.com/</a></li></ul>\
             <p>Tape une URL dans la barre d'adresse, ou un numero pour suivre un lien.</p>",
            crate::VERSION);
        return page_from_html(html.as_bytes(), url, width);
    }
    if url == "about:system" {
        let dt = rtc::now();
        let (used, free, total) = crate::kernel::heap::stats();
        let html = format!(
            "<h1>Systeme</h1><ul><li>Heure : {:02}:{:02}:{:02}</li><li>Uptime : {} s</li>\
             <li>Heap : {}/{} o (libre {})</li><li>PCI : {} peripheriques</li>\
             <li>User : {}</li></ul>",
            dt.hour, dt.minute, dt.second, timer::seconds(), used, total, free,
            crate::arch::x86_64::pci::count(), users::session().username());
        return page_from_html(html.as_bytes(), url, width);
    }
    if let Some(path) = url.strip_prefix("file:") {
        let p = path.trim_start_matches('/');
        let full = format!("/{}", p);
        let fs = ramfs::fs();
        let body = match fs.resolve_checked(&full, 0) {
            Ok(idx) if fs.nodes[idx].kind == ramfs::NodeKind::File && fs.can(idx, ramfs::PERM_R) => {
                let mut s = String::new();
                for k in 0..fs.nodes[idx].content_len { s.push(fs.nodes[idx].content[k] as char); }
                format!("<h2>file:{}</h2><pre>{}</pre>", full, esc(&s))
            }
            Ok(_) => format!("<h2>Erreur</h2><p>permission refusee : {}</p>", full),
            _ => format!("<h2>Erreur</h2><p>introuvable : {}</p>", full),
        };
        return page_from_html(body.as_bytes(), url, width);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let doc = crate::net::fetch_document(url);
        if doc.ok && !doc.body.is_empty() {
            if doc.is_html {
                return page_from_html(&doc.body, &doc.final_url, width);
            }
            // Contenu non-HTML : affichage texte brut.
            let mut s = String::new();
            for &b in doc.body.iter().take(40_000) {
                match b { b'\n' | b'\r' | b'\t' => s.push(b as char), 0x20..=0x7e => s.push(b as char), _ => s.push('.') }
            }
            let html = format!("<h3>{} ({} o)</h3><pre>{}</pre>", esc(&doc.content_type), doc.body.len(), esc(&s));
            return page_from_html(html.as_bytes(), &doc.final_url, width);
        }
        // Echec : affiche le diagnostic (TLS, statut, erreurs).
        let mut html = String::from("<h2>Echec du chargement</h2><pre>");
        for line in &doc.banner { html.push_str(&esc(line)); html.push('\n'); }
        html.push_str("</pre>");
        return page_from_html(html.as_bytes(), url, width);
    }
    let html = format!("<h2>Page inconnue</h2><p>{}</p><p>Essaie about:bouchaud, https://example.com/</p>", esc(url));
    page_from_html(html.as_bytes(), url, width)
}

/// Normalise l'entree de la barre d'adresse en URL. Un numero seul suit le lien
/// correspondant de la page courante.
pub(crate) fn resolve_input(input: &str, page: &Page) -> String {
    let t = input.trim();
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = t.parse::<usize>() {
            if n >= 1 && n <= page.links.len() {
                return page.links[n - 1].href.clone();
            }
        }
    }
    if t.contains("://") || t.starts_with("about:") || t.starts_with("file:") {
        return t.to_string();
    }
    if t.contains('.') && !t.contains(' ') {
        return format!("https://{}", t);
    }
    t.to_string()
}

/// Hauteur maximale de defilement pour la zone de contenu donnee.
pub(crate) fn max_scroll(page: &Page, bh: usize) -> i32 {
    let view = bh.saturating_sub(ADDR_H) as i32;
    (page.height - view).max(0)
}

/// Suit un clic dans la zone de contenu : renvoie l'URL du lien touche.
pub(crate) fn link_at(page: &Page, scroll: i32, rel_x: i32, rel_y: i32) -> Option<String> {
    let cy = rel_y - ADDR_H as i32 + scroll; // coordonnee dans le contenu
    let cx = rel_x;
    for lnk in &page.links {
        if cx >= lnk.x && cx < lnk.x + lnk.w && cy >= lnk.y && cy < lnk.y + lnk.h {
            return Some(lnk.href.clone());
        }
    }
    None
}

/// Dessine le navigateur : barre d'adresse + contenu peint.
pub(crate) fn draw(url: &str, input: &str, page: &Page, scroll: i32, bx: usize, by: usize, bw: usize, bh: usize) {
    // Barre d'adresse.
    fb::fill_rect(bx, by, bw, ADDR_H - 1, fb::C_WHITE);
    let shown = if input == url { input.to_string() } else { format!("{}_", input) };
    fb::draw_text(bx + 1, by + 1, clip(&shown, bw / 8), fb::C_BLACK);
    // Contenu.
    let cy = by + ADDR_H;
    let ch = bh.saturating_sub(ADDR_H);
    web::paint(page, scroll, bx, cy, bw, ch);
}
