//! Application Moniteur (infos systeme en direct).

use crate::arch::x86_64::rtc;
use crate::fs::ramfs;
use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::kernel::timer;
use alloc::format;

/// Dessine les informations systeme (rafraichies a chaque frame).
pub(crate) fn draw(bx: usize, by: usize, bw: usize, _bh: usize) {
    let cols = bw / 7;
    let dt = rtc::now();
    let (used, free, total) = crate::kernel::heap::stats();
    // ORDRE DES VERROUS : le RAMFS (rang 50) ne se tient PAS pendant qu'on
    // prend la table des processus (rang 20). Le garde etait retenu jusqu'a la
    // fin de la fonction, et `process::count()` le prenait dessous : LOCKDEP
    // paniquait des la premiere peinture du Moniteur. On lit le compte, et le
    // garde tombe a la fin de l'instruction.
    let inodes = ramfs::fs().used_nodes();
    let procs = crate::kernel::process::count();
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
    put(&format!("PCI    {} dev", crate::arch::x86_64::pci::count()), 0xeff3f8, false);
    put(&format!("Procs  {}", procs), 0xeff3f8, false);
    put(&format!("RAMFS  {} inodes", inodes), 0xeff3f8, false);
}
