//! UI owned by the WM during voluntary shutdown; never by a concurrent painter.
use core::sync::atomic::{AtomicBool, Ordering};
use crate::gui::framebuffer as fb;
static ACTIVE: AtomicBool = AtomicBool::new(false);
pub fn begin() {
    ACTIVE.store(true, Ordering::Release);
    fb::reset_clip();
    fb::fill_rect_rgb(0, 0, fb::width(), fb::height(), 0x0d1117);
    let title = "BOUCHAUD OS";
    let x = fb::width().saturating_sub(fb::text_width(title, 30.0, true))/2;
    let y = fb::height()/2;
    fb::draw_text_prop(x, y.saturating_sub(72), title, 0xeff3f8, 30.0, true);
    fb::present();
    progress("Fermeture des services", 0);
}
pub fn progress(label: &str, phase: usize) {
    if !ACTIVE.load(Ordering::Acquire) { return; }
    let x = fb::width().saturating_sub(420)/2;
    let y = fb::height()/2;
    fb::reset_clip();
    fb::fill_rect_rgb(x, y, 420, 78, 0x0d1117);
    let label_x = x + 210usize.saturating_sub(fb::text_width(label, 15.0, false)/2);
    fb::draw_text_prop(label_x, y, label, 0x9da8b8, 15.0, false);
    for i in 0..8 {
        fb::fill_rect_rgb(x+146+i*16, y+38, 8, 4, if i == phase%8 { 0x44a8ff } else { 0x313a48 });
    }
    fb::present_rect(x, y, 420, 78);
}
pub fn finish(ok: bool) {
    progress(if ok { "Journaux sauvegardes. Extinction..." } else { "Sauvegarde incomplete : consulter les logs" }, 7);
    // An error must remain readable before power is removed.
    if !ok && ACTIVE.load(Ordering::Acquire) && crate::kernel::task::try_current().is_some() {
        crate::kernel::task::sleep_ticks(2_000);
    }
}
