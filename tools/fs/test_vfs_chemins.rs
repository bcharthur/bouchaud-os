//! Exécute le module VFS pur du noyau avec rustc --test.
extern crate alloc;
#[path = "../../src/fs/vfs_chemins.rs"]
mod vfs_chemins;
use vfs_chemins::{normalise_absolu as n, ErreurChemin as E};

#[test]
fn normalise_separateurs_et_points() {
    assert_eq!(n("/persist//home/./user/notes.txt").unwrap(), "/persist/home/user/notes.txt");
    assert_eq!(n("/persist/home/../Documents").unwrap(), "/persist/Documents");
    assert_eq!(n("/").unwrap(), "/");
    assert_eq!(n("//a///b/").unwrap(), "/a/b");
}
#[test]
fn refuse_sortie_de_racine_et_relatifs() {
    assert_eq!(n("../../tmp").unwrap_err(), E::Relatif);
    assert_eq!(n("/a/../../etc").unwrap_err(), E::SortieRacine);
    assert_eq!(n("/../etc").unwrap_err(), E::SortieRacine);
}
#[test]
fn refuse_controles_et_limites_de_longueur() {
    assert_eq!(n("/ok/\u{0}").unwrap_err(), E::ControleInterdit);
    assert_eq!(n("/ok\n").unwrap_err(), E::ControleInterdit);
    assert_eq!(n(&format!("/{}", "a".repeat(256))).unwrap_err(), E::ComposantTropLong);
    assert_eq!(n(&format!("/{}", "a/".repeat(3000))).unwrap_err(), E::TropLong);
}
#[test]
fn utf8_conserve_et_composants_255_octets() {
    assert_eq!(n("/persist/Téléchargements/été.txt").unwrap(), "/persist/Téléchargements/été.txt");
    assert!(n(&format!("/{}", "é".repeat(128))).is_err());
    assert!(n(&format!("/{}", "a".repeat(255))).is_ok());
}
