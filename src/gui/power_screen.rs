//! UI owned by the WM during voluntary shutdown; never by a concurrent painter.
use core::sync::atomic::{AtomicBool, Ordering};
use crate::gui::framebuffer as fb;
static ACTIVE: AtomicBool = AtomicBool::new(false);
pub fn begin() {
    ACTIVE.store(true, Ordering::Release);
    fb::reset_clip();
    fb::fill_rect_rgb(0, 0, fb::width(), fb::height(), 0x0d1117);
    let x = fb::width().saturating_sub(160)/2;
    let y = fb::height()/2;
    fb::draw_text_rgb(x, y.saturating_sub(60), "BOUCHAUD OS", 0xeff3f8, 2);
    fb::present();
    progress("Fermeture des services", 0);
}
pub fn progress(label: &str, phase: usize) {
    if !ACTIVE.load(Ordering::Acquire) { return; }
    let x = fb::width().saturating_sub(360)/2;
    let y = fb::height()/2;
    fb::reset_clip();
    fb::fill_rect_rgb(x, y, 360, 65, 0x0d1117);
    fb::draw_text_rgb(x+8, y, label, 0x9da8b8, 1);
    for i in 0..8 {
        fb::fill_rect_rgb(x+116+i*16, y+30, 8, 4, if i == phase%8 { 0x44a8ff } else { 0x313a48 });
    }
    fb::present_rect(x, y, 360, 65);
}
pub fn finish(ok: bool) {
    progress(if ok { "Journaux sauvegardes. Extinction..." } else { "Sauvegarde incomplete : consulter les logs" }, 7);
}
