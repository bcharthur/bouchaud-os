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

/// Dessine le navigateur (barre d'adresse + contenu).
pub(crate) fn draw(url: &str, input: &str, content: &[String], bx: usize, by: usize, bw: usize, bh: usize) {
    let cols = bw / 8;
    fb::fill_rect(bx, by, bw, 9, fb::C_WHITE);
    let shown = if input == url { input.to_string() } else { format!("{}_", input) };
    fb::draw_text(bx + 1, by + 1, clip(&shown, cols), fb::C_BLACK);
    let mut yy = by + 12;
    for l in content {
        if yy + 8 > by + bh { break; }
        fb::draw_text(bx, yy, clip(l, cols), fb::C_WHITE);
        yy += 8;
    }
}
