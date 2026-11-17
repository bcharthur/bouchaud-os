//! Application Moniteur (infos systeme en direct).

use crate::arch::x86_64::rtc;
use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::kernel::timer;
use alloc::format;

/// Dessine les informations systeme. Les compteurs sont releves au plus une
/// fois par seconde (BOUCHAUD_RELEVE_SECONDE_V1, `releve`), l'heure aussi
/// (BOUCHAUD_RTC_LISSE_V1) : une trame ne fait ni E/S ni verrou de comptage.
pub(crate) fn draw(bx: usize, by: usize, bw: usize, _bh: usize) {
    let cols = bw / 7;
    let dt = rtc::maintenant_lisse();
    let r = super::releve::courant();
    let (used, free, total) = (r.heap_utilise, r.heap_libre, r.heap_total);
    let (inodes, procs) = (r.inodes_ramfs, r.processus);
    let mut yy = by;
    let mut put = |s: &str, couleur: u32, gras: bool| {
        fb::draw_text_prop(bx, yy, clip(s, cols), couleur, 14.0, gras);
        yy += 20;
    };
    put(&format!("Bouchaud OS {}", crate::VERSION), 0xf5be4a, true);
    put(&format!("Heure  {:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second), 0x5bda8b, false);
    put(&format!("Uptime {} s", timer::seconds()), 0xeff3f8, false);
    put(&format!("Heap   {}/{} o", used, total), 0xeff3f8, false);
    put(&format!("Libre  {} o", free), 0xeff3f8, false);
    put(&format!("PCI    {} dev", r.pci), 0xeff3f8, false);
    put(&format!("Procs  {}", procs), 0xeff3f8, false);
    put(&format!("RAMFS  {} inodes", inodes), 0xeff3f8, false);
}
