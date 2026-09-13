//! Etat observe des processus, sans confondre presence et disponibilite IPC.
use crate::gui::{framebuffer as fb, services};
use alloc::format;
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
        fb::draw_text(bx+x+10, by+22, label, fb::C_WHITE);
    }
    let root = services::racine();
    let processes = crate::kernel::task::processes();
    let mut y = by + 60;
    let status = if services::en_echec() { "Echec du lancement" }
        else if root == 0 { "Session arretee" } else { "Session geree par le bureau" };
    fb::draw_text(bx+12, y, status, fb::C_CYAN); y += 24;
    for name in ["BouchaudBrowserHost", "WebContent", "RequestServer", "ImageDecoder", "Compositor", "WebWorker"] {
        if y + 14 > by + bh { break; }
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
        fb::draw_text(bx+12, y, crate::gui::window::clip(&line, bw.saturating_sub(24)/8), fb::C_WHITE);
        y += 24;
    }
    if y+32 < by+bh {
        fb::draw_text(bx+12, y+8, "En cours = processus vivant, pas une page chargee.", fb::C_GRAY);
        let net = format!("Reseau : {}", crate::net::nom_verdict());
        fb::draw_text(bx+12, y+24, &net, fb::C_CYAN);
    }
}
