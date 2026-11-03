//! Host proof of the loader -> kernel handoff and of the health commit
//! decision (BOUCHAUD_BOOT_HANDOFF_V1).
//!
//!   rustc --edition 2021 --test tools/update/test_passage.rs -o /tmp/tp && /tmp/tp
//!
//! Production modules included unchanged: system/update-core/src/{ab,passage}.rs.

#![allow(dead_code)]

#[path = "../../system/update-core/src/ab.rs"]
mod ab;
#[path = "../../system/update-core/src/passage.rs"]
mod passage;

pub use ab::*;
use passage::{commit_sante, Passage, Raison, RefusCommit};

const V1: [u8; 32] = [0x11; 32];
const V2: [u8; 32] = [0x22; 32];

/// What the loader (tools/reference/uefi-preboot-probe/src/ab.rs) hands over,
/// given the record it read and whether persisting the consumed attempt
/// succeeded. Returns (handoff, record now on disk).
fn loader(e: &Enregistrement, persistance_ok: bool) -> (Passage, Enregistrement) {
    let (slot, apres) = e.choisit_boot();
    let (choisi, raison, disque) = if apres == *e {
        (slot, Raison::Actif, *e)
    } else if persistance_ok {
        (slot, if slot != e.actif { Raison::CandidatEssaiConsomme } else { Raison::RollbackCandidatEpuise }, apres)
    } else if slot != e.actif {
        (e.actif, Raison::PersistanceImpossibleReplieActif, *e)
    } else {
        (slot, Raison::RollbackNonPersiste, *e)
    };
    let essais = match disque.slot(choisi).etat { Etat::Candidat { essais } => essais, _ => 0 };
    (Passage { slot: choisi, raison, essais_restants: essais, sequence_meta: disque.sequence, boot_id: 7, meta_unique: [0x5a; 16] }, disque)
}

fn candidat_b(essais: u8) -> Enregistrement {
    let e = Enregistrement::initial(Slot::A, V1);
    let e = e.commence_installation(Slot::B, Slot::A).unwrap();
    e.marque_candidat(Slot::B, V2, V2, essais).unwrap()
}

#[test]
fn encodage_bijectif_et_chaque_bit_compte() {
    for slot in [Slot::A, Slot::B] {
        for raison in [Raison::Actif, Raison::CandidatEssaiConsomme, Raison::RollbackCandidatEpuise,
                       Raison::RollbackNonPersiste, Raison::PersistanceImpossibleReplieActif] {
            let p = Passage { slot, raison, essais_restants: 2, sequence_meta: 0x0102_0304_0506_0708, boot_id: 0xdead_beef, meta_unique: [0xa5; 16] };
            let o = passage::encode(&p);
            assert_eq!(passage::decode(&o), Some(p));
            for bit in 0..passage::OCTETS * 8 {
                let mut x = o;
                x[bit / 8] ^= 1 << (bit % 8);
                assert_eq!(passage::decode(&x), None, "bit {bit} flipped still decodes");
            }
        }
    }
    assert_eq!(passage::decode(&[0u8; passage::OCTETS]), None);
    assert_eq!(passage::decode(&[0u8; 10]), None);
}

#[test]
fn candidat_booste_puis_sain_devient_actif() {
    let e = candidat_b(3);
    let (p, disque) = loader(&e, true);
    assert_eq!((p.slot, p.raison, p.essais_restants), (Slot::B, Raison::CandidatEssaiConsomme, 2));
    let n = commit_sante(&p, &disque).unwrap().expect("a commit");
    assert_eq!(n.actif, Slot::B);
    assert_eq!(n.slot(Slot::B).etat, Etat::Bon);
    assert_eq!(n.slot(Slot::A).etat, Etat::Bon, "the previous version stays available for revient()");
    // Next boot: B is the good active slot, nothing is consumed any more.
    let (p2, d2) = loader(&n, true);
    assert_eq!((p2.slot, p2.raison), (Slot::B, Raison::Actif));
    assert_eq!(commit_sante(&p2, &d2), Ok(None));
}

#[test]
fn repli_sur_l_actif_ne_committe_jamais_le_candidat() {
    // The loader could not make the attempt durable and booted A. The record
    // still says "B candidate". Deducing the running slot from it would
    // commit an image that never ran: the handoff says A, nothing is written.
    let e = candidat_b(3);
    let (p, disque) = loader(&e, false);
    assert_eq!((p.slot, p.raison), (Slot::A, Raison::PersistanceImpossibleReplieActif));
    assert_eq!(commit_sante(&p, &disque), Ok(None));
    assert_eq!(disque.slot(Slot::B).etat, Etat::Candidat { essais: 3 });
}

#[test]
fn meta_modifiee_depuis_le_boot_refuse() {
    let e = candidat_b(3);
    let (p, disque) = loader(&e, true);
    let ailleurs = disque.sante_ko(Slot::B);
    assert_eq!(commit_sante(&p, &ailleurs), Err(RefusCommit::MetaChangee));
    // Replaying the same handoff after a commit writes nothing: B is now the
    // good active slot.
    let n = commit_sante(&p, &disque).unwrap().unwrap();
    assert_eq!(commit_sante(&p, &n), Ok(None));
}

#[test]
fn installation_pendant_le_boot_n_empeche_pas_de_confirmer_l_actif() {
    // Regression (QEMU bench mise-a-jour): A runs, updated begins installing
    // into B -> META rewritten (sequence +1). Confirming A must still succeed
    // and write nothing; promoting a candidate on a changed record must not.
    let e = Enregistrement::initial(Slot::A, V1);
    let (p, disque) = loader(&e, true);
    assert_eq!((p.slot, p.raison), (Slot::A, Raison::Actif));
    let pendant = disque.commence_installation(Slot::B, Slot::A).unwrap();
    assert_ne!(pendant.sequence, p.sequence_meta);
    assert_eq!(commit_sante(&p, &pendant), Ok(None));
    let apres = pendant.marque_candidat(Slot::B, V2, V2, 3).unwrap();
    assert_eq!(commit_sante(&p, &apres), Ok(None), "A stays the good slot; B stays Candidate");
    assert_eq!(apres.slot(Slot::B).etat, Etat::Candidat { essais: 3 });
}

#[test]
fn passage_forge_vers_un_slot_mauvais_ou_vide_refuse() {
    let e = candidat_b(3);
    let mauvais = e.sante_ko(Slot::B);
    let p = Passage { slot: Slot::B, raison: Raison::CandidatEssaiConsomme, essais_restants: 0,
                      sequence_meta: mauvais.sequence, boot_id: 1, meta_unique: [1; 16] };
    assert_eq!(commit_sante(&p, &mauvais), Err(RefusCommit::SlotNiCandidatNiActif));
    let vide = Enregistrement::initial(Slot::A, V1).commence_installation(Slot::B, Slot::A).unwrap();
    let p = Passage { sequence_meta: vide.sequence, ..p };
    assert_eq!(commit_sante(&p, &vide), Err(RefusCommit::SlotNiCandidatNiActif));
    // A candidate the loader did NOT boot as a candidate (raison Actif) is
    // never committed, even if the handoff names it.
    let p = Passage { slot: Slot::B, raison: Raison::Actif, essais_restants: 3, sequence_meta: e.sequence, boot_id: 1, meta_unique: [1; 16] };
    assert_eq!(commit_sante(&p, &e), Err(RefusCommit::SlotNiCandidatNiActif));
}

#[test]
fn candidat_jamais_sain_rollback_apres_n_boots() {
    let mut disque = candidat_b(3);
    let mut slots = Vec::new();
    for _ in 0..6 {
        let (p, d) = loader(&disque, true);
        slots.push(p.slot);
        disque = d; // never healthy: no commit
    }
    assert_eq!(slots, [Slot::B, Slot::B, Slot::B, Slot::A, Slot::A, Slot::A]);
    assert_eq!(disque.slot(Slot::B).etat, Etat::Mauvais);
}

#[test]
fn exploration_le_commit_ne_produit_jamais_un_actif_non_bon() {
    // Every record reachable from a fresh install by install/candidate/boot/
    // commit/ko/revient, every loader outcome: a commit only ever promotes
    // the slot the loader booted as a consumed candidate, and the result is
    // always bootable (active slot Good).
    use std::collections::{HashSet, VecDeque};
    let depart = Enregistrement::initial(Slot::A, V1);
    let mut vus = HashSet::new();
    let mut file = VecDeque::from([depart]);
    let mut commits = 0;
    while let Some(e) = file.pop_front() {
        if !vus.insert(encode(&e)) || vus.len() > 5000 { continue; }
        let mut suivants = Vec::new();
        for cible in [Slot::A, Slot::B] {
            for en_cours in [Slot::A, Slot::B] {
                if let Ok(n) = e.commence_installation(cible, en_cours) {
                    suivants.push(n);
                    if let Ok(c) = n.marque_candidat(cible, V2, V2, 2) { suivants.push(c); }
                }
            }
        }
        if let Ok(r) = e.revient() { suivants.push(r); }
        for ok in [true, false] {
            let (p, d) = loader(&e, ok);
            suivants.push(d);
            match commit_sante(&p, &d) {
                Ok(Some(n)) => {
                    commits += 1;
                    assert_eq!(p.raison, Raison::CandidatEssaiConsomme);
                    assert_eq!(n.actif, p.slot);
                    assert_eq!(n.slot(n.actif).etat, Etat::Bon);
                    suivants.push(n);
                }
                Ok(None) => assert!(d.actif == p.slot && d.slot(p.slot).etat == Etat::Bon),
                Err(_) => panic!("loader handoff on its own record refused: {p:?} {d:?}"),
            }
        }
        file.extend(suivants);
    }
    assert!(commits > 0 && vus.len() > 10, "exploration too small: commits={commits} etats={}", vus.len());
    println!("etats={} commits={commits}", vus.len());
}

