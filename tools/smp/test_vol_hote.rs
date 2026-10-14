//! La lecture du temps vole publie par KVM (BOUCHAUD_PREUVE_HOTE_V1) :
//! jamais une valeur lue pendant que l'hote l'ecrit.

#[path = "../../src/kernel/time/vol_hote_lecture.rs"]
mod lecture;

use lecture::lecture_versionnee;
use std::cell::Cell;

#[test]
fn une_lecture_stable_rend_le_compteur() {
    assert_eq!(lecture_versionnee(|| (4, 1234), || 4, 8), Some(1234));
}

#[test]
fn une_version_impaire_est_une_ecriture_en_cours() {
    // L'hote ecrit (version 5), puis publie (version 6).
    let tour = Cell::new(0);
    let r = lecture_versionnee(
        || {
            tour.set(tour.get() + 1);
            if tour.get() == 1 { (5, 999_999) } else { (6, 2_000) }
        },
        || 6,
        8,
    );
    assert_eq!(r, Some(2_000));
}

#[test]
fn une_version_qui_change_pendant_la_lecture_fait_recommencer() {
    // Premiere lecture : version 6, mais 8 apres -- le compteur lu peut
    // etre a moitie neuf. Seconde : 8 et 8.
    let tour = Cell::new(0);
    let r = lecture_versionnee(
        || {
            tour.set(tour.get() + 1);
            if tour.get() == 1 { (6, 1_000) } else { (8, 3_000) }
        },
        || if tour.get() == 1 { 8 } else { 8 },
        8,
    );
    assert_eq!(r, Some(3_000));
}

#[test]
fn un_hote_qui_ne_laisse_jamais_lire_ne_prouve_rien() {
    assert_eq!(lecture_versionnee(|| (7, 42), || 7, 8), None);
    let n = Cell::new(0u32);
    assert_eq!(lecture_versionnee(|| { n.set(n.get() + 2); (n.get(), 1) }, || n.get() + 2, 8), None);
}
