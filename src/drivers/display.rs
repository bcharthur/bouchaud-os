//! Abstraction d'affichage.
//!
//! Pilote l'ecran graphique (actuellement VGA mode 12h, 640x480x16, via `gfx`).
//! Couche destinee a accueillir un framebuffer lineaire haute resolution quand
//! le bootloader le fournira (migration bootloader 0.11).

use crate::drivers::gfx;

pub fn width() -> usize { gfx::WIDTH }
pub fn height() -> usize { gfx::HEIGHT }

/// Affiche l'etat de l'affichage (commande `display`/`devices`).
pub fn print_info() {
    crate::println!("display: VGA mode 12h {}x{}x16 (planaire)", gfx::WIDTH, gfx::HEIGHT);
    crate::println!("  texte: VGA 80x25 ; haute resolution truecolor = bootloader 0.11 (a venir)");
}
