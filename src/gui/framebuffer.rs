//! Couche framebuffer du bureau : primitives de dessin.
//!
//! Abstraction au-dessus du pilote d'affichage (`drivers::gfx`, framebuffer HD
//! truecolor via Bochs VBE). Le reste du GUI dessine via ce module, ce qui
//! permet de changer de backend d'affichage sans toucher aux applications.

#[allow(unused_imports)]
pub use crate::drivers::gfx::{
    draw_text, draw_text_scaled, draw_text_rgb, draw_text_prop, text_width,
    fill_rect_rgb, blit_rgb, blend_rgb, pixel_rgb, get_pixel_rgb,
    enter, fill_rect, leave, pixel, present, rect,
    HEIGHT, WIDTH,
    C_BLACK, C_BLUE, C_CYAN, C_DKGRAY, C_GRAY, C_GREEN, C_RED, C_TITLE,
    C_WHITE, C_YELLOW,
};
