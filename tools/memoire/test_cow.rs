//! BOUCHAUD_COW_V1 -- la table des frames partagees, contredite sur l'hote.
//!
//! Ce que ces epreuves defendent : une frame partagee par `fork` n'est rendue
//! a l'allocateur qu'une fois, par son DERNIER proprietaire, et une frame
//! redevenue exclusive quitte la table (son proprietaire peut ecrire sans
//! copier).

extern crate alloc;

#[path = "../../src/kernel/memory/cow.rs"]
mod cow;

use cow::{Rendu, TableCow};

const F: u64 = 0x1234_5000;

#[test]
fn une_frame_exclusive_se_libere_aussitot() {
    let mut t = TableCow::neuve();
    assert_eq!(t.proprietaires(F), 1);
    assert_eq!(t.rend(F), Rendu::Liberer);
}

#[test]
fn fork_puis_deux_morts_une_seule_liberation() {
    let mut t = TableCow::neuve();
    assert_eq!(t.partage(F), 2);
    assert!(t.est_partagee(F));
    assert_eq!(t.rend(F), Rendu::Garder);
    // Redevenue exclusive : hors table, son proprietaire peut la promouvoir.
    assert!(!t.est_partagee(F));
    assert_eq!(t.proprietaires(F), 1);
    assert_eq!(t.rend(F), Rendu::Liberer);
}

#[test]
fn forks_en_chaine() {
    let mut t = TableCow::neuve();
    t.partage(F); // pere + fils
    assert_eq!(t.partage(F), 3); // le fils forke a son tour
    let liberations = (0..3).filter(|_| t.rend(F) == Rendu::Liberer).count();
    assert_eq!(liberations, 1);
    assert_eq!(t.frames_partagees(), 0);
}

#[test]
fn les_frames_sont_independantes() {
    let mut t = TableCow::neuve();
    t.partage(F);
    t.partage(F + 0x1000);
    assert_eq!(t.rend(F), Rendu::Garder);
    assert_eq!(t.rend(F + 0x1000), Rendu::Garder);
    assert_eq!(t.rend(F), Rendu::Liberer);
    assert_eq!(t.rend(F + 0x1000), Rendu::Liberer);
    assert_eq!(t.cumuls(), (2, 2));
}

#[test]
fn copie_a_l_ecriture_du_pere_puis_mort_du_fils() {
    // Le pere copie (il rend l'ancienne frame et en possede une neuve) ; le
    // fils reste seul proprietaire de l'ancienne, puis meurt : liberee.
    let mut t = TableCow::neuve();
    t.partage(F);
    assert_eq!(t.rend(F), Rendu::Garder); // copie du pere
    assert_eq!(t.rend(F), Rendu::Liberer); // mort du fils
}
