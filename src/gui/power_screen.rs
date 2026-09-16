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
    progress(if ok { "Journaux sauvegardes. Extinction..." } else { "Sauvegarde incomplete" }, 7);
    // An error must remain readable before power is removed.
    if !ok && ACTIVE.load(Ordering::Acquire) && crate::kernel::task::try_current().is_some() {
        crate::kernel::task::sleep_ticks(2_000);
    }
}

// ===========================================================================
// BOUCHAUD_ECHEC_QUI_SE_LIT_V1 : « consulter les logs » etait un piege
// ===========================================================================
//
// L'ecran disait « Sauvegarde incomplete : consulter les logs ». Or les logs
// en question sont EXACTEMENT ceux qui viennent de ne pas etre sauves. Le
// message renvoyait donc l'utilisateur vers le fichier dont il annoncait
// l'absence, et la machine s'eteignait sur cette phrase.
//
// Ce qui manquait n'etait pas un fichier, c'etait la raison. Elle tient en
// quelques lignes, elle est connue au moment ou l'on echoue, et l'ecran est
// le SEUL endroit qui reste quand le support ne repond plus. Elle s'affiche
// donc la : la couche, l'etape, l'etat du support, le dernier enregistrement
// reellement pose et depuis quand plus rien ne passe.

/// Affiche le detail d'un echec de sauvegarde. Les lignes sont deja formatees
/// par l'appelant, qui seul connait les compteurs de sa couche.
pub fn detail_echec(titre: &str, lignes: &[&str]) {
    if !ACTIVE.load(Ordering::Acquire) { return; }
    let largeur = fb::width();
    let hauteur = fb::height();
    let x = largeur.saturating_sub(820) / 2;
    let mut y = hauteur / 2 + 96;
    // Tout ce qui suit doit tenir a l'ecran : une ligne peinte hors du cadre
    // est une ligne perdue, et c'est celle qu'on cherchait.
    let fin = hauteur.saturating_sub(24);
    if y + 24 >= fin { return; }
    fb::reset_clip();
    fb::fill_rect_rgb(x, y, 820.min(largeur.saturating_sub(x)), fin.saturating_sub(y), 0x0d1117);
    fb::draw_text_prop(x, y, titre, 0xff7b72, 17.0, true);
    y += 30;
    for ligne in lignes {
        if y + 20 >= fin { break; }
        fb::draw_text_prop(x, y, ligne, 0x9da8b8, 14.0, false);
        y += 20;
    }
    fb::present();
}
