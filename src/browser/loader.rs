//! Chargement d'URL pour Nautile.
//!
//! Inspiré du pipeline réseau de Firefox (Necko) et du gestionnaire de navigation
//! de Chromium (NavigationController). Principe : local-first souverain — le rendu
//! est assuré par le moteur web intégré de l'OS ; le mode compat (proxy Chromium
//! externe) n'est accessible que via le préfixe `compat:`.

use crate::gui::web::{Page, Session};
use crate::fs::ramfs;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Active le proxy Chromium pour TOUTES les URL http/https (non recommandé).
/// `false` = navigation souveraine par défaut. Le mode compat reste accessible
/// via le préfixe `compat:` sans modifier cette constante.
const ENABLE_COMPAT_PROXY: bool = false;

/// Hôte du service de rendu déporté (voir `tools/render-proxy`).
const PROXY_HOST: &str = "192.168.1.187:8080";

// ── API publique ──────────────────────────────────────────────────────────────

/// Charge une URL et renvoie (Session interactive, Page rendue).
/// C'est le point d'entrée unique du pipeline de chargement de Nautile.
pub fn open(url: &str, width: i32) -> (Session, Page) {
    let width = width.max(80);
    // Pages internes
    if url == "about:bouchaud" {
        return from_html(super::pages::bouchaud_home().as_bytes(), url, width);
    }
    // Page Google locale (logo + barre de recherche).
    if url == "https://google.com" || url == "https://www.google.com"
        || url == "http://google.com" || url == "http://www.google.com"
        || url == "google.com" || url == "www.google.com"
    {
        return from_html(super::pages::google_home().as_bytes(), url, width);
    }
    if url == "about:calc" {
        return from_html(super::pages::CALC_APP.as_bytes(), url, width);
    }
    if url == "about:wasm" {
        return from_html(super::pages::WASM_DEMO.as_bytes(), url, width);
    }
    if url == "about:system" {
        return from_html(super::pages::system_info().as_bytes(), url, width);
    }
    // Fichier local (RAMFS)
    if let Some(path) = url.strip_prefix("file:") {
        return load_file(path, url, width);
    }
    // Mode compat forcé par préfixe
    if let Some(rest) = url.strip_prefix("compat:") {
        return compat_render(rest, width);
    }
    // HTTP / HTTPS : rendu local (souverain) par défaut
    if url.starts_with("http://") || url.starts_with("https://") {
        if ENABLE_COMPAT_PROXY { return compat_render(url, width); }
        return local_render(url, width);
    }
    let html = format!(
        "<h2>Page inconnue</h2><p>{}</p>\
         <p>Essaie <a href=\"about:bouchaud\">about:bouchaud</a> \
         ou <a href=\"https://example.com/\">example.com</a>.</p>",
        esc(url)
    );
    from_html(html.as_bytes(), url, width)
}

/// Normalise la saisie de la barre d'adresse en URL.
/// Un nombre seul suit le lien correspondant dans la page courante.
pub fn resolve_input(input: &str, page: &Page) -> String {
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

// ── Rendu local (souverain) ───────────────────────────────────────────────────

fn local_render(url: &str, width: i32) -> (Session, Page) {
    let doc = crate::net::fetch_document(url);
    if doc.ok && doc.is_html && !doc.body.is_empty() {
        return from_html(&doc.body, &doc.final_url, width);
    }
    if doc.ok {
        // Document non-HTML : aperçu texte + métadonnées
        let mut preview = String::new();
        for &b in doc.body.iter().take(40_000) {
            match b {
                b'\n' | b'\r' | b'\t' => preview.push(b as char),
                0x20..=0x7e => preview.push(b as char),
                _ => preview.push('.'),
            }
        }
        let mut info = String::new();
        for line in doc.banner.iter().take(6) { info.push_str(&esc(line)); info.push('\n'); }
        let html = format!(
            "<h2>Document non-HTML</h2>\
             <ul><li>URL : {u}</li><li>Type : {ct}</li>\
             <li>Taille : {sz} octets</li></ul>\
             <pre>{info}</pre><hr><pre>{pv}</pre>",
            u = esc(&doc.final_url), ct = esc(&doc.content_type),
            sz = doc.body.len(), info = info, pv = esc(&preview)
        );
        return from_html(html.as_bytes(), &doc.final_url, width);
    }
    // Erreur réseau / DNS / TLS
    let mut info = String::new();
    for line in doc.banner.iter().take(12) { info.push_str(&esc(line)); info.push('\n'); }
    let html = format!(
        "<h2>Impossible de charger la page</h2>\
         <p>URL : {u}</p><pre>{info}</pre>\
         <p><a href=\"about:bouchaud\">← Accueil Nautile</a></p>",
        u = esc(url), info = info
    );
    from_html(html.as_bytes(), url, width)
}

// ── Mode COMPAT (optionnel, non souverain) ────────────────────────────────────

fn compat_render(url: &str, width: i32) -> (Session, Page) {
    let purl = format!("http://{}/render?url={}", PROXY_HOST, pct_encode(url));
    let doc  = crate::net::fetch_document(&purl);
    if doc.ok && !doc.body.is_empty() {
        if let Some(img) = crate::gui::image::decode(&doc.body) {
            let cw     = width.max(1) as usize;
            let scaled = crate::gui::image::downscale(&img, cw, 1_000_000);
            let iw = scaled.w as i32;
            let ih = scaled.h as i32;
            let page = Page {
                title: url.to_string(),
                items: alloc::vec![crate::gui::web::Item::Image { x: 0, y: 0, w: iw, h: ih, idx: 0 }],
                links: Vec::new(),
                images: alloc::vec![scaled],
                height: ih.max(1),
                bg: 0xffffff,
            };
            let (sess, _) = Session::open(b"", url, width);
            return (sess, page);
        }
    }
    let mut info = String::new();
    for line in doc.banner.iter().take(6) { info.push_str(&esc(line)); info.push('\n'); }
    let html = format!(
        "<h2>Mode compat indisponible</h2>\
         <p>Le proxy Chromium externe est injoignable ou n'a pas renvoyé d'image.</p>\
         <p>Lance le service :</p>\
         <pre>cd tools/render-proxy\nnpm run setup\nnpm start</pre>\
         <p>Proxy attendu : <b>http://{host}</b></p>\
         <p>URL : {u}</p><pre>{info}</pre>\
         <p><a href=\"about:bouchaud\">← Accueil</a></p>",
        host = PROXY_HOST, u = esc(url), info = info
    );
    from_html(html.as_bytes(), url, width)
}

// ── Fichier local ─────────────────────────────────────────────────────────────

fn load_file(path: &str, url: &str, width: i32) -> (Session, Page) {
    let p    = path.trim_start_matches('/');
    let full = format!("/{}", p);
    let fs   = ramfs::fs();
    let body = match fs.resolve_checked(&full, 0) {
        Ok(idx) if fs.nodes[idx].kind == ramfs::NodeKind::File && fs.can(idx, ramfs::PERM_R) => {
            let mut s = String::new();
            for k in 0..fs.nodes[idx].content_len { s.push(fs.nodes[idx].content[k] as char); }
            format!("<h2>file:{}</h2><pre>{}</pre>", full, esc(&s))
        }
        Ok(_) => format!("<h2>Permission refusée</h2><p>{}</p>", full),
        _     => format!("<h2>Fichier introuvable</h2><p>{}</p>", full),
    };
    from_html(body.as_bytes(), url, width)
}

// ── Utilitaires ───────────────────────────────────────────────────────────────

/// Parse HTML → Session + Page (plafonnée à 4 Mo pour éviter les OOM).
fn from_html(html: &[u8], base: &str, width: i32) -> (Session, Page) {
    let capped = &html[..html.len().min(4_000_000)];
    Session::open(capped, base, width)
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c { '&' => o.push_str("&amp;"), '<' => o.push_str("&lt;"), '>' => o.push_str("&gt;"), _ => o.push(c) }
    }
    o
}

fn pct_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { o.push(b as char); }
        else { o.push('%'); o.push(hexd(b >> 4)); o.push(hexd(b)); }
    }
    o
}

fn hexd(n: u8) -> char {
    core::char::from_digit((n & 0x0f) as u32, 16).unwrap_or('0').to_ascii_uppercase()
}
