//! Bouchaud Browser (navigateur natif minimal, "chromium stub").
//!
//! Pages internes : `about:bouchaud`, `about:system`, `file:/<chemin>` (lecture
//! RAMFS). Les URL `http(s)://` afficheront du contenu quand le reseau (e1000 +
//! TCP/HTTP) sera disponible.

use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::arch::x86_64::rtc;
use crate::fs::ramfs;
use crate::kernel::timer;
use crate::users;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Charge une "page" et renvoie ses lignes.
pub(crate) fn load_page(url: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if url == "about:bouchaud" {
        out.push("Bouchaud OS".to_string());
        out.push("OS souverain francais experimental".to_string());
        out.push("".to_string());
        out.push(format!("Version : {}", crate::VERSION));
        out.push("Kernel  : Rust no_std".to_string());
        out.push("GUI     : window manager VGA".to_string());
        out.push("Reseau  : loopback (e1000 a venir)".to_string());
        out.push("".to_string());
        out.push("Pages: about:system  file:/readme.txt".to_string());
    } else if url == "about:system" {
        let dt = rtc::now();
        let (used, free, total) = crate::kernel::heap::stats();
        out.push(format!("Heure  : {:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second));
        out.push(format!("Uptime : {} s", timer::seconds()));
        out.push(format!("Heap   : {}/{} o (libre {})", used, total, free));
        out.push(format!("PCI    : {} peripheriques", crate::arch::x86_64::pci::count()));
        out.push(format!("User   : {}", users::session().username()));
    } else if let Some(path) = url.strip_prefix("file:") {
        let p = path.trim_start_matches('/');
        let full = format!("/{}", p);
        let fs = ramfs::fs();
        match fs.resolve_checked(&full, 0) {
            Ok(idx) if fs.nodes[idx].kind == ramfs::NodeKind::File => {
                if fs.can(idx, ramfs::PERM_R) {
                    let mut s = String::new();
                    for k in 0..fs.nodes[idx].content_len { s.push(fs.nodes[idx].content[k] as char); }
                    for l in s.split('\n') { out.push(l.to_string()); }
                } else {
                    out.push("Erreur: permission denied".to_string());
                }
            }
            _ => out.push(format!("Erreur: introuvable {}", full)),
        }
    } else if url.starts_with("http://") || url.starts_with("https://") {
        // Recuperation HTTP/HTTPS reelle via e1000/TCP (+ TLS 1.3 pour https).
        return crate::net::http_get(url);
    } else {
        out.push(format!("Page inconnue: {}", url));
        out.push("Essaie: about:bouchaud, about:system, file:/readme.txt".to_string());
    }
    out
}

/// Resout l'entree de la barre d'adresse : si c'est un numero de lien affiche
/// (`[n] url`), renvoie l'URL correspondante ; sinon renvoie l'entree telle quelle.
/// Permet de "cliquer" un lien en tapant son numero.
pub(crate) fn resolve_link(input: &str, content: &[String]) -> String {
    let t = input.trim();
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        let prefix = format!("[{}] ", t);
        for line in content {
            if let Some(url) = line.strip_prefix(&prefix) {
                return url.trim().to_string();
            }
        }
    }
    t.to_string()
}

/// Decoupe une ligne logique en rangees de largeur <= `cols` (retour a la ligne
/// sur les espaces ; coupe les mots trop longs). Compte en caracteres pour ne
/// jamais couper au milieu d'un caractere multi-octet.
fn wrap_line(line: &str, cols: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    if cols == 0 { rows.push(line.to_string()); return rows; }
    let mut cur = String::new();
    let mut cur_len = 0usize;
    let push_word = |rows: &mut Vec<String>, cur: &mut String, cur_len: &mut usize, word: &str| {
        let wlen = word.chars().count();
        if *cur_len == 0 {
            if wlen <= cols { cur.push_str(word); *cur_len = wlen; return; }
        } else if *cur_len + 1 + wlen <= cols {
            cur.push(' '); cur.push_str(word); *cur_len += 1 + wlen; return;
        } else {
            rows.push(core::mem::take(cur)); *cur_len = 0;
            if wlen <= cols { cur.push_str(word); *cur_len = wlen; return; }
        }
        // Mot plus long que la largeur : coupe dur, caractere par caractere.
        for ch in word.chars() {
            cur.push(ch); *cur_len += 1;
            if *cur_len == cols { rows.push(core::mem::take(cur)); *cur_len = 0; }
        }
    };
    for word in line.split(' ') {
        push_word(&mut rows, &mut cur, &mut cur_len, word);
    }
    rows.push(cur);
    rows
}

/// Echelle du texte de contenu (lisibilite en HD 720p). Le chrome (barre
/// d'adresse) reste en 8 px.
const CONTENT_SCALE: usize = 2;

/// Dessine le navigateur (barre d'adresse + contenu).
pub(crate) fn draw(url: &str, input: &str, content: &[String], bx: usize, by: usize, bw: usize, bh: usize) {
    // Barre d'adresse : police 8 px sur bandeau blanc.
    let addr_cols = bw / 8;
    fb::fill_rect(bx, by, bw, 9, fb::C_WHITE);
    let shown = if input == url { input.to_string() } else { format!("{}_", input) };
    fb::draw_text(bx + 1, by + 1, clip(&shown, addr_cols), fb::C_BLACK);

    // Contenu : texte agrandi pour la lisibilite, retour a la ligne sur la
    // largeur disponible a cette echelle.
    let cell = 8 * CONTENT_SCALE;
    let cols = (bw / cell).max(1);
    let mut yy = by + 12;
    'outer: for l in content {
        for row in wrap_line(l, cols) {
            if yy + cell > by + bh { break 'outer; }
            fb::draw_text_scaled(bx, yy, &row, fb::C_WHITE, CONTENT_SCALE);
            yy += cell + 2;
        }
    }
}
