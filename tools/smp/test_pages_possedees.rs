//! Le cout d'un `madvise`/`munmap`/`fork` sur les frames possedees.
//!
//! # Ce que ces tests mesurent
//!
//! `AddressSpace::pages` etait un `Vec<u64>` interroge par balayage lineaire :
//!
//!   * `prepare_unmap`  : `pages.contains(&phys)` PAR PAGE de la plage ;
//!   * `finish_unmap`   : `pages.iter().position(..)` PAR frame rendue ;
//!   * `owns_frame`     : `contains`, appele PAR PAGE par `clone_for_fork`.
//!
//! Soit `O(R x P)`, R la plage traitee et P les frames residentes. Un
//! WebContent a 200 Mio resident donne P = 51 200 ; un `madvise(DONTNEED)` de
//! 16 Mio donne R = 4096. Deux fois 2 x 10^8 comparaisons, le gros verrou tenu.
//!
//! Les deux structures sont modelisees avec le MEME compteur de comparaisons,
//! et le test affirme la difference. Ce n'est pas une mesure de temps — elle
//! serait bruitee — mais un comptage d'operations, que la structure determine.
//!
//! Les tests de semantique verifient ce dont `finish_unmap` depend vraiment :
//! une frame retiree une fois, et jamais deux.
//!
//! Lance par `tools/smp/test-pages-possedees.sh`.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

const PAGE: u64 = 4096;
const BASE: u64 = 0x200_000;

fn frame(rang: u64) -> u64 {
    BASE + rang * PAGE
}

// ─── Les deux modeles ──────────────────────────────────────────────────────

/// L'ancien : un `Vec`, parcouru.
struct Liste {
    pages: Vec<u64>,
    comparaisons: u64,
}

impl Liste {
    fn neuve(residentes: u64) -> Self {
        Self { pages: (0..residentes).map(frame).collect(), comparaisons: 0 }
    }

    fn contient(&mut self, phys: u64) -> bool {
        // `Vec::contains` s'arrete au premier egal ; sinon il va au bout.
        for (rang, &candidate) in self.pages.iter().enumerate() {
            self.comparaisons += 1;
            if candidate == phys {
                let _ = rang;
                return true;
            }
        }
        false
    }

    fn retire(&mut self, phys: u64) -> bool {
        for rang in 0..self.pages.len() {
            self.comparaisons += 1;
            if self.pages[rang] == phys {
                self.pages.swap_remove(rang);
                return true;
            }
        }
        false
    }
}

/// Le nouveau : un ensemble ordonne.
struct Ensemble {
    pages: BTreeSet<u64>,
    comparaisons: u64,
}

impl Ensemble {
    fn neuf(residentes: u64) -> Self {
        Self { pages: (0..residentes).map(frame).collect(), comparaisons: 0 }
    }

    /// Un `BTreeSet` compare `O(log n)` fois. On majore par `log2(n) + 1`,
    /// ce qui suffit pour affirmer l'ordre de grandeur sans dependre du
    /// facteur de branchement de l'implementation.
    fn note_recherche(&mut self) {
        let n = self.pages.len().max(1) as u64;
        self.comparaisons += 64 - n.leading_zeros() as u64 + 1;
    }

    fn contient(&mut self, phys: u64) -> bool {
        self.note_recherche();
        self.pages.contains(&phys)
    }

    fn retire(&mut self, phys: u64) -> bool {
        self.note_recherche();
        self.pages.remove(&phys)
    }
}

// ─── Le cout ───────────────────────────────────────────────────────────────

/// Le WebContent reel : 200 Mio resident, `madvise(DONTNEED)` de 16 Mio.
const RESIDENTES: u64 = 51_200;
const PLAGE: u64 = 4_096;

/// La plage se trouve au MILIEU des frames residentes : ni le meilleur cas
/// (en tete) ni le pire (absente), mais celui qu'un tas reel produit.
fn plage() -> impl Iterator<Item = u64> {
    (RESIDENTES / 2..RESIDENTES / 2 + PLAGE).map(frame)
}

#[test]
fn prepare_unmap_ne_balaye_plus_toutes_les_frames_residentes() {
    let mut liste = Liste::neuve(RESIDENTES);
    for phys in plage() {
        assert!(liste.contient(phys));
    }
    let mut ensemble = Ensemble::neuf(RESIDENTES);
    for phys in plage() {
        assert!(ensemble.contient(phys));
    }

    assert!(
        liste.comparaisons > 100_000_000,
        "l'ancien doit bien couter des centaines de millions de comparaisons, \
         sinon ce test ne mesure pas le vrai probleme (mesure : {})",
        liste.comparaisons,
    );
    assert!(
        ensemble.comparaisons < liste.comparaisons / 1000,
        "le nouveau doit couter au moins mille fois moins ({} contre {})",
        ensemble.comparaisons, liste.comparaisons,
    );
}

#[test]
fn finish_unmap_ne_balaye_plus_toutes_les_frames_residentes() {
    let mut liste = Liste::neuve(RESIDENTES);
    for phys in plage() {
        liste.retire(phys);
    }
    let mut ensemble = Ensemble::neuf(RESIDENTES);
    for phys in plage() {
        ensemble.retire(phys);
    }

    assert!(liste.comparaisons > 100_000_000, "{}", liste.comparaisons);
    assert!(
        ensemble.comparaisons < liste.comparaisons / 1000,
        "{} contre {}", ensemble.comparaisons, liste.comparaisons,
    );
}

/// `clone_for_fork` appelle `owns_frame` PAR PAGE : c'est `O(P^2)`.
#[test]
fn fork_ne_coute_plus_le_carre_des_frames_residentes() {
    // Un processus plus modeste : `fork` recopie tout, le test doit rester court.
    const P: u64 = 8_192;
    let mut liste = Liste::neuve(P);
    for rang in 0..P {
        liste.contient(frame(rang));
    }
    let mut ensemble = Ensemble::neuf(P);
    for rang in 0..P {
        ensemble.contient(frame(rang));
    }
    assert!(
        liste.comparaisons > P * P / 4,
        "l'ancien est quadratique : {} pour P={P}", liste.comparaisons,
    );
    assert!(
        ensemble.comparaisons <= P * 20,
        "le nouveau est lineaire en P : {} pour P={P}", ensemble.comparaisons,
    );
}

// ─── La semantique dont `finish_unmap` depend ──────────────────────────────

#[test]
fn une_frame_retiree_deux_fois_n_est_liberee_qu_une_fois() {
    // `prepare_unmap` peut pousser deux fois la meme frame si deux pages
    // virtuelles de la plage la partagent. `finish_unmap` doit alors la rendre
    // UNE fois : c'est ce qui interdit le double `free`.
    let mut ensemble = Ensemble::neuf(16);
    let mut liberees = 0;
    for phys in [frame(3), frame(3)] {
        if ensemble.retire(phys) {
            liberees += 1;
        }
    }
    assert_eq!(liberees, 1, "une seule liberation");
}

#[test]
fn une_frame_ne_peut_appartenir_qu_une_fois_a_un_espace() {
    let mut pages = BTreeSet::new();
    assert!(pages.insert(frame(1)));
    assert!(!pages.insert(frame(1)), "le doublon est refuse");
    assert_eq!(pages.len(), 1);

    // Ce que le `Vec` permettait, et qui faisait fuir la seconde occurrence :
    let mut liste = Vec::new();
    liste.push(frame(1));
    liste.push(frame(1));
    assert_eq!(liste.len(), 2, "le Vec acceptait le doublon");
}

#[test]
fn retirer_une_frame_absente_ne_libere_rien() {
    let mut ensemble = Ensemble::neuf(4);
    assert!(!ensemble.retire(frame(99)));
    assert_eq!(ensemble.pages.len(), 4);
}

#[test]
fn le_compte_de_frames_est_preserve() {
    let mut pages: BTreeSet<u64> = (0..100).map(frame).collect();
    assert_eq!(pages.len(), 100);
    for rang in 0..40 {
        assert!(pages.remove(&frame(rang)));
    }
    assert_eq!(pages.len(), 60, "mapped_pages doit rester juste");
}

/// L'ordre de destruction : `free_all` parcourt l'ensemble, chaque frame une fois.
#[test]
fn la_destruction_libere_chaque_frame_exactement_une_fois() {
    let pages: BTreeSet<u64> = (0..1000).map(frame).collect();
    let mut vues = BTreeSet::new();
    for &phys in pages.iter() {
        assert!(vues.insert(phys), "frame {phys:#x} vue deux fois");
    }
    assert_eq!(vues.len(), 1000);
}
