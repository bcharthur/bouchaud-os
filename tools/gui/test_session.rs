//! Regles du bureau testees sur le meme module Rust que le noyau.
#[path = "../../src/gui/atelier/session.rs"]
mod session;
use session::{action_echap, EchapSansMenu};
#[test]
fn echap_bureau_vide_ne_quitte_pas() {
    assert_eq!(action_echap(false, false), EchapSansMenu::Ignorer);
}
#[test]
fn menu_contextuel_prioritaire() {
    assert_eq!(action_echap(true, true), EchapSansMenu::PrisParApplication);
    assert_eq!(action_echap(true, false), EchapSansMenu::PrisParApplication);
}
#[test]
fn fenetre_normale_peut_se_fermer() {
    assert_eq!(action_echap(false, true), EchapSansMenu::FermerFenetre);
}
