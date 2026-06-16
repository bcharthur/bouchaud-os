//! Couche framebuffer du bureau : primitives de dessin.
//!
//! Abstraction au-dessus du pilote d'affichage (`drivers::gfx`, mode VGA 12h
//! 640x480). Le reste du GUI dessine via ce module, ce qui permettra de basculer
//! vers un framebuffer lineaire haute resolution sans toucher aux applications.

pub use crate::drivers::gfx::{
    clear, draw_text, enter, fill_rect, leave, pixel, present, rect, HEIGHT, WIDTH,
    C_BLACK, C_BLUE, C_CYAN, C_DESKTOP, C_DKBLUE, C_DKGRAY, C_GRAY, C_GREEN, C_RED, C_TITLE,
    C_WHITE, C_YELLOW,
};
