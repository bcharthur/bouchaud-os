//! Application Moniteur (infos systeme en direct).

use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::arch::x86_64::rtc;
use crate::fs::ramfs;
use crate::kernel::timer;
use alloc::format;

/// Dessine les informations systeme (rafraichies a chaque frame).
pub(crate) fn draw(bx: usize, by: usize, bw: usize, _bh: usize) {
    let cols = bw / 8;
    let dt = rtc::now();
    let (used, free, total) = crate::kernel::heap::stats();
    let fs = ramfs::fs();
    let mut yy = by;
    let mut put = |s: &str, c: u8| { fb::draw_text(bx, yy, clip(s, cols), c); yy += 10; };
    put(&format!("Bouchaud OS {}", crate::VERSION), fb::C_YELLOW);
    put(&format!("Heure  {:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second), fb::C_GREEN);
    put(&format!("Uptime {} s", timer::seconds()), fb::C_WHITE);
    put(&format!("Heap   {}/{} o", used, total), fb::C_WHITE);
    put(&format!("Libre  {} o", free), fb::C_WHITE);
    put(&format!("PCI    {} dev", crate::arch::x86_64::pci::count()), fb::C_WHITE);
    put(&format!("Procs  {}", crate::kernel::process::count()), fb::C_WHITE);
    put(&format!("RAMFS  {} inodes", fs.used_nodes()), fb::C_WHITE);
}
