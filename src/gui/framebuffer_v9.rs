// V9 wrapper around `gui/framebuffer.rs`.
//
// All drawing primitives keep the legacy API. Only `present()` and
// `present_rect()` are replaced: the physical framebuffer copy is desktop-local
// work and can run outside the global kernel lock.

#[path = "framebuffer.rs"]
mod legacy;

#[allow(unused_imports)]
pub use legacy::{
    draw_text,
    draw_text_scaled,
    draw_text_rgb,
    draw_text_prop,
    text_width,
    fill_rect_rgb,
    blit_rgb,
    blend_rgb,
    blend_span,
    blit_argb_span,
    pixel_rgb,
    get_pixel_rgb,
    ligne_mut,
    enter,
    fill_rect,
    handoff_to_userland,
    leave,
    pixel,
    rect,
    resume_from_userland,
    userland_owns_display,
    set_clip,
    reset_clip,
    pixels_dessines,
    pixels_texte,
    clip_rect,
    decoupe_touche,
    note_pixels_dessines,
    dernier_present_rect,
    lfb_present_generation,
    trace_present,
    HEIGHT,
    WIDTH,
    C_BLACK,
    C_BLUE,
    C_CYAN,
    C_DKGRAY,
    C_GRAY,
    C_GREEN,
    C_RED,
    C_TITLE,
    C_WHITE,
    C_YELLOW,
};

#[inline]
pub fn present() {
    crate::kernel::task::stall_site_set(750, 0);
    crate::gui::desktop_bkl::sans_bkl(
        crate::gui::desktop_bkl::Site::Present,
        crate::drivers::gfx::present,
    );
    crate::kernel::task::stall_site_set(751, 0);
}

#[inline]
pub fn present_rect(x: usize, y: usize, width: usize, height: usize) {
    crate::kernel::task::stall_site_set(752, ((width as u64) << 32) | height as u64);
    crate::gui::desktop_bkl::sans_bkl(
        crate::gui::desktop_bkl::Site::PresentRect,
        || crate::drivers::gfx::present_rect(x, y, width, height),
    );
    crate::kernel::task::stall_site_set(753, 0);
}
