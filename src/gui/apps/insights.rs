//! Bouchaud Insights M1 — vue de diagnostic native sans fausses metriques.
//! Toutes les valeurs proviennent de compteurs runtime; pas de pourcentages
//! CPU inventes ni de promesse de GPU ou de persistance.
use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use alloc::format;

pub(crate) fn draw(bx: usize, by: usize, bw: usize, bh: usize) {
    if bw < 70 || bh < 90 { return; }
    fb::fill_rect_rgb(bx, by, bw, bh, 0x101827);
    fb::fill_rect_rgb(bx, by, bw, 56, 0x1c2a3d);
    fb::draw_text_prop(bx + 18, by + 12, "Bouchaud Insights", 0xf7c569, 23.0, true);
    fb::draw_text_prop(bx + 18, by + 40, "Etat direct du noyau — mesures disponibles", 0x9db0c9, 12.0, false);

    let (used, free, total) = crate::kernel::heap::stats();
    let (user_keys, desktop_keys, fullscreen) = crate::gui::window_manager::stats_entree();
    let services = crate::kernel::process::count();
    let nodes = crate::fs::ramfs::used_nodes_relaxed();
    let pci = crate::arch::x86_64::pci::count();
    let seconds = crate::kernel::timer::seconds();

    let lines = [
        format!("Uptime                     {} s", seconds),
        format!("Processus vivants           {}", services),
        format!("Heap utilise                {} KiB", used / 1024),
        format!("Heap libre                  {} KiB", free / 1024),
        format!("Heap total                  {} KiB", total / 1024),
        format!("Noeuds RAMFS                {}", nodes),
        format!("Peripheriques PCI           {}", pci),
        format!("Touches vers client GUI     {}", user_keys),
        format!("Touches vers bureau         {}", desktop_keys),
        format!("Invalidations plein ecran   {}", fullscreen),
    ];
    let mut y = by + 75;
    for (i, line) in lines.iter().enumerate() {
        if y + 20 > by + bh.saturating_sub(48) { break; }
        if i % 2 == 0 { fb::fill_rect_rgb(bx + 10, y.saturating_sub(3), bw.saturating_sub(20), 24, 0x182438); }
        fb::draw_text_prop(bx + 18, y, clip(line, bw.saturating_sub(38) / 7), 0xe2e8f0, 14.0, false);
        y += 29;
    }
    let foot = by + bh - 40;
    fb::fill_rect_rgb(bx, foot, bw, 40, 0x1c2a3d);
    fb::draw_text_prop(bx + 12, foot + 7,
        clip("Sources directes. CPU/GPU/stockage : non instrumentes dans cette vue.", bw.saturating_sub(24) / 7),
        0x9db0c9, 12.0, false);
}
