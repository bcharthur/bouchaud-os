// V15 facade around the historical widgets implementation.
//
// Le gros `widgets.rs` reste intact. Le chemin event-driven appelle
// `widgets::draw_barre_haute()` ; on reutilise donc son rendu, puis on ajoute un
// indicateur FPS discret avec la MEME police proportionnelle que le bureau.

#[path = "widgets.rs"]
mod legacy;

pub(crate) use legacy::*;

use alloc::format;
use crate::gui::framebuffer as fb;

/// Barre superieure + FPS utiles du compositeur.
pub(crate) fn draw_barre_haute() {
    legacy::draw_barre_haute();

    let snapshot = crate::gui::frame_clock::snapshot();
    let valeur = if snapshot.active {
        format!("FPS:{:3}", snapshot.fps_arrondi())
    } else {
        format!("FPS: --")
    };

    // A gauche de l'horloge. Pas de jauge rouge a 0 FPS : sur un bureau
    // immobile, zero trame utile est justement le comportement event-driven
    // desire. L'indicateur est donc neutre, purement metrique.
    const CORPS: f32 = 12.0;
    let largeur = fb::text_width(&valeur, CORPS, false);
    let marge_droite = 104usize;
    let x = fb::WIDTH.saturating_sub(marge_droite + largeur + 18);
    let y = 8usize;

    fb::fill_rect_rgb(
        x.saturating_sub(7),
        4,
        largeur + 14,
        22,
        crate::gui::theme::COLOR_SURFACE,
    );
    fb::draw_text_prop(
        x,
        y,
        &valeur,
        crate::gui::theme::COLOR_TEXT_SECONDARY,
        CORPS,
        false,
    );
}
