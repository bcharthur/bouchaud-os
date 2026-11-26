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

fn partage(t: &mut TableCow, frame: u64) -> u32 {
    assert!(t.partage_lot(&[frame]));
    t.proprietaires(frame)
}

#[test]
fn partage_par_lots_avec_croissance_et_tombes() {
    let mut t = TableCow::neuve();
    let frames: Vec<u64> = (0..1024).map(|n| F + n * 4096).collect();
    assert!(t.partage_lot(&frames[..256]));
    assert!(t.partage_lot(&frames[..128]));
    for &frame in &frames[128..256] {
        assert_eq!(t.rend(frame), Rendu::Garder);
    }
    assert!(t.partage_lot(&frames));
    assert_eq!(t.frames_partagees(), 1024);
    for (i, frame) in frames.into_iter().enumerate() {
        assert_eq!(t.proprietaires(frame), if i < 128 { 4 } else { 2 });
    }
}

#[test]
fn une_frame_exclusive_se_libere_aussitot() {
    let mut t = TableCow::neuve();
    assert_eq!(t.proprietaires(F), 1);
    assert_eq!(t.rend(F), Rendu::Liberer);
}

#[test]
fn fork_puis_deux_morts_une_seule_liberation() {
    let mut t = TableCow::neuve();
    assert_eq!(partage(&mut t, F), 2);
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
    partage(&mut t, F); // pere + fils
    assert_eq!(partage(&mut t, F), 3); // le fils forke a son tour
    let liberations = (0..3).filter(|_| t.rend(F) == Rendu::Liberer).count();
    assert_eq!(liberations, 1);
    assert_eq!(t.frames_partagees(), 0);
}

#[test]
fn les_frames_sont_independantes() {
    let mut t = TableCow::neuve();
    partage(&mut t, F);
    partage(&mut t, F + 0x1000);
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
    partage(&mut t, F);
    assert_eq!(t.rend(F), Rendu::Garder); // copie du pere
    assert_eq!(t.rend(F), Rendu::Liberer); // mort du fils
}

// BOUCHAUD_TABLE_COW_HACHEE_V1 : la table hachee contre un modele (BTreeMap),
// sur une suite pseudo-aleatoire de partages et de rendus, avec croissance et
// tombes reutilisees.
#[test]
fn la_table_hachee_suit_le_modele() {
    use std::collections::BTreeMap;
    let mut t = TableCow::neuve();
    let mut m: BTreeMap<u64, u32> = BTreeMap::new();
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    for _ in 0..200_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let frame = (x % 3000 + 1) << 12;
        if x & 0x10000 != 0 {
            let attendu = m.get(&frame).map_or(2, |c| c + 1);
            assert_eq!(partage(&mut t, frame), attendu);
            m.insert(frame, attendu);
        } else {
            let attendu = match m.get_mut(&frame) {
                None => Rendu::Liberer,
                Some(c) => {
                    *c -= 1;
                    if *c <= 1 {
                        m.remove(&frame);
                    }
                    Rendu::Garder
                }
            };
            assert_eq!(t.rend(frame), attendu);
        }
        if x % 1000 == 0 {
            assert_eq!(t.frames_partagees(), m.len());
            for (&f, &c) in m.iter().take(50) {
                assert_eq!(t.proprietaires(f), c);
                assert!(t.est_partagee(f));
            }
        }
    }
    assert_eq!(t.frames_partagees(), m.len());
}

#[test]
fn la_frame_zero_et_les_grandes_adresses_sont_des_cles_valides() {
    let mut t = TableCow::neuve();
    for f in [0u64, 0x1000, 0xF_FFFF_F000, 0x000F_FFFF_FFFF_F000] {
        assert_eq!(partage(&mut t, f), 2);
        assert_eq!(t.proprietaires(f), 2);
    }
    for f in [0u64, 0x1000, 0xF_FFFF_F000, 0x000F_FFFF_FFFF_F000] {
        assert_eq!(t.rend(f), Rendu::Garder);
        assert!(!t.est_partagee(f));
        assert_eq!(t.rend(f), Rendu::Liberer);
    }
    assert_eq!(t.frames_partagees(), 0);
}
