//! Etat observe des processus, sans confondre presence et disponibilite IPC.
use crate::gui::{framebuffer as fb, services};
use alloc::format;

#[inline]
fn texte(x: usize, y: usize, valeur: &str, couleur: u32, gras: bool) {
    fb::draw_text_prop(x, y, valeur, couleur, 14.0, gras);
}

pub(crate) fn click(x: i32, y: i32) {
    if (12..42).contains(&y) {
        if (12..182).contains(&x) { services::demande(services::DEMARRER); }
        if (198..368).contains(&x) { services::demande(services::ARRETER); }
    }
}
pub(crate) fn draw(bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect_rgb(bx, by, bw, bh, 0x111827);
    for (x, label, color) in [(12, "Demarrer Ladybird", 0x2563eb), (198, "Arreter Ladybird", 0x374151)] {
        fb::fill_rect_rgb(bx+x, by+12, 170, 30, color);
        texte(bx+x+10, by+20, label, 0xeff3f8, true);
    }
    let root = services::racine();
    let processes = crate::kernel::task::processes();
    let mut y = by + 60;
    let status = if services::en_echec() { "Echec du lancement" }
        else if root == 0 { "Session arretee" } else { "Session geree par le bureau" };
    texte(bx+12, y, status, 0x67d5e8, true); y += 24;
    for name in ["BouchaudBrowserHost", "WebContent", "RequestServer", "ImageDecoder", "Compositor", "WebWorker"] {
        if y + 16 > by + bh { break; }
        let mut pids = alloc::vec::Vec::new();
        for p in &processes {
            if root == 0 || (p.resource_group_id != root && p.pid != root) { continue; }
            if p.lifecycle.lock().zombie { continue; }
            let meta = p.metadata.lock();
            let base = meta.name.rsplit('/').next().unwrap_or(&meta.name);
            if base == name || (name == "BouchaudBrowserHost" && p.pid == root) { pids.push(p.pid); }
        }
        let line = if pids.is_empty() { format!("{:<20} Arrete / a la demande", name) }
            else { format!("{:<20} En cours   PID {:?}", name, pids) };
        texte(bx+12, y, crate::gui::window::clip(&line, bw.saturating_sub(24)/7), 0xeff3f8, false);
        y += 24;
    }
    if y+72 < by+bh {
        texte(bx+12, y+8, "En cours = processus vivant, pas une page chargee.", 0x9da8b8, false);
        let net = format!("Reseau : {}", crate::net::nom_verdict());
        texte(bx+12, y+28, &net, 0x67d5e8, false);
        let (cpu, rss, sampled) = services::usage();
        let stats = if sampled == 0 || root == 0 { alloc::string::String::from("Mesures : en attente / session arretee") }
            else { format!("CPU {}% (100%=1 coeur)  Somme RSS {} Mio", cpu, rss/(1024*1024)) };
        texte(bx+12, y+48, &stats, 0xeff3f8, false);
        texte(bx+12, y+68, "Releve 5 s ; RSS partagee comptee par processus.", 0x9da8b8, false);
    }
}
