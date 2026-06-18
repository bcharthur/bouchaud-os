//! Abstraction d'affichage.
//!
//! Pilote l'ecran graphique : framebuffer lineaire HD truecolor via Bochs VBE
//! (BGA), 1280x720x32, expose par `gfx`. Le mode texte VGA 80x25 reste utilise
//! par le shell ; le bureau bascule en HD le temps de la session graphique.

use crate::drivers::gfx;

pub fn width() -> usize { gfx::WIDTH }
pub fn height() -> usize { gfx::HEIGHT }

/// Affiche l'etat de l'affichage (commande `display`/`devices`).
pub fn print_info() {
    crate::println!("display: framebuffer HD {}x{}x32 (Bochs VBE/BGA truecolor)", gfx::WIDTH, gfx::HEIGHT);
    crate::println!("  texte: VGA 80x25 ; bureau graphique en HD via la commande 'desktop'");
}
