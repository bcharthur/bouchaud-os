//! Host proof of the anti-rollback floor and of the v2 manifest
//! (BOUCHAUD_ANTI_RETOUR_V1).
//!
//!   rustc --edition 2021 --test tools/update/test_anti_retour.rs -o /tmp/tar && /tmp/tar
//!
//! Production modules included unchanged: system/update-core/src/{ab,
//! anti_retour, manifeste, ed25519}.rs. Launched by tools/ci/run_host_tests.sh.

#![allow(dead_code)]

#[path = "../../system/update-core/src/ab.rs"]
mod ab;
#[path = "../../system/update-core/src/anti_retour.rs"]
mod anti_retour;
#[path = "../../system/update-core/src/ed25519.rs"]
mod ed25519;
#[path = "../../system/update-core/src/manifeste.rs"]
mod manifeste;

use ab::Slot;
use anti_retour::*;

fn bloc(r: Option<&Registre>) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    if let Some(r) = r {
        b[..OCTETS].copy_from_slice(&encode(r));
    }
    b
}

// --- The record ----------------------------------------------------------

#[test]
fn aller_retour_et_somme() {
    let r = Registre { sequence: 7, plancher: 42, slots: [41, 43] };
    assert_eq!(decode(&encode(&r)), Some(r));
    for i in 0..OCTETS {
        let mut o = encode(&r);
        o[i] ^= 0x01;
        assert_eq!(decode(&o), None, "octet {i} altere accepte");
    }
}

#[test]
fn neuf_valide_ou_illisible() {
    assert_eq!(lit(&bloc(None), &bloc(None)), Lecture::Neuf);
    let r = Registre { sequence: 1, plancher: 5, slots: [5, 0] };
    assert_eq!(lit(&bloc(Some(&r)), &bloc(None)), Lecture::Valide(r));
    assert_eq!(lit(&bloc(None), &bloc(Some(&r))), Lecture::Valide(r));
    // Garbage in both copies is NOT a fresh install: refusing is the only
    // safe reading (resetting to 0 would re-open every old release).
    let mut g = bloc(None);
    g[100] = 0xff;
    assert_eq!(lit(&g, &bloc(None)), Lecture::Illisible);
    let mut abime = bloc(Some(&r));
    abime[20] ^= 1;
    assert_eq!(lit(&abime, &abime.clone()), Lecture::Illisible);
}

#[test]
fn la_copie_la_plus_recente_gagne_et_n_est_jamais_ecrasee() {
    let a = Registre { sequence: 3, plancher: 10, slots: [10, 0] };
    let b = Registre { sequence: 4, plancher: 11, slots: [10, 11] };
    assert_eq!(lit(&bloc(Some(&a)), &bloc(Some(&b))), Lecture::Valide(b));
    assert_eq!(lit(&bloc(Some(&b)), &bloc(Some(&a))), Lecture::Valide(b));
    assert_eq!(copie_a_ecrire(&bloc(Some(&a)), &bloc(Some(&b))), 0);
    assert_eq!(copie_a_ecrire(&bloc(Some(&b)), &bloc(Some(&a))), 1);
    assert_eq!(copie_a_ecrire(&bloc(Some(&a)), &bloc(None)), 1);
    assert_eq!(copie_a_ecrire(&bloc(None), &bloc(None)), 0);
}

#[test]
fn coupure_a_chaque_octet_rend_l_ancien_ou_le_nouveau() {
    // The older copy is overwritten byte by byte; a power cut at any byte
    // leaves the previous floor or the new one, never a lower one.
    let ancien = Registre { sequence: 8, plancher: 20, slots: [20, 19] };
    let nouveau = ancien.installe(Slot::B, 21).confirme(Slot::B).unwrap();
    let (c0, c1) = (bloc(Some(&ancien)), bloc(Some(&Registre { sequence: 7, ..ancien })));
    let i = copie_a_ecrire(&c0, &c1);
    let cible = encode(&nouveau);
    for coupe in 0..=OCTETS {
        let mut ecrit = if i == 0 { c0.clone() } else { c1.clone() };
        ecrit[..coupe].copy_from_slice(&cible[..coupe]);
        let (x, y) = if i == 0 { (ecrit, c1.clone()) } else { (c0.clone(), ecrit) };
        match lit(&x, &y) {
            Lecture::Valide(r) => assert!(r == ancien || r == nouveau, "coupe {coupe}: {r:?}"),
            autre => panic!("coupe {coupe}: {autre:?}"),
        }
    }
}

// --- The policy ------------------------------------------------------------

#[test]
fn installation_neuve_accepte_tout() {
    assert_eq!(autorise(Lecture::Neuf, 0), Ok(Registre::NEUF), "manifeste v1 sans plancher");
    assert_eq!(autorise(Lecture::Neuf, 1), Ok(Registre::NEUF));
}

#[test]
fn retrograde_refuse_egal_accepte() {
    let r = Registre { sequence: 2, plancher: 10, slots: [10, 9] };
    assert_eq!(autorise(Lecture::Valide(r), 9), Err(Refus::Retrograde));
    assert_eq!(autorise(Lecture::Valide(r), 1), Err(Refus::Retrograde));
    assert_eq!(autorise(Lecture::Valide(r), 10), Ok(r), "reinstaller la meme release (reparation)");
    assert_eq!(autorise(Lecture::Valide(r), 11), Ok(r));
    assert_eq!(autorise(Lecture::Valide(r), 0), Err(Refus::SansSequence), "v1 rejoue apres un plancher");
    assert_eq!(autorise(Lecture::Illisible, 99), Err(Refus::RegistreIllisible));
}

#[test]
fn le_plancher_ne_monte_qu_au_commit_et_ne_descend_jamais() {
    let r = Registre { sequence: 1, plancher: 10, slots: [10, 0] };
    // finish: B holds release 12 -- the floor does not move yet.
    let r = r.installe(Slot::B, 12);
    assert_eq!(r.plancher, 10);
    // A candidate that never confirms leaves the floor alone: the old
    // release (A, 10) stays installable for a local rollback AND as a new
    // install (10 >= 10).
    assert_eq!(autorise(Lecture::Valide(r), 10), Ok(r));
    // Health commit of B: floor = 12.
    let r = r.confirme(Slot::B).unwrap();
    assert_eq!(r.plancher, 12);
    assert_eq!(autorise(Lecture::Valide(r), 10), Err(Refus::Retrograde));
    // Confirming A (local rollback to release 10) never lowers the floor.
    assert_eq!(r.confirme(Slot::A), None);
    // Idempotent: confirming B again changes nothing (no write).
    assert_eq!(r.confirme(Slot::B), None);
}

#[test]
fn coupure_entre_commit_ab_et_plancher_reparee_au_boot_suivant() {
    // The A/B commit happened, the floor write did not: the next confirmed
    // boot of the same slot raises it.
    let r = Registre { sequence: 5, plancher: 10, slots: [10, 12] };
    let apres = r.confirme(Slot::B).unwrap();
    assert_eq!(apres.plancher, 12);
    assert!(apres.sequence > r.sequence);
}

// --- Manifest v2 -------------------------------------------------------------

const H: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn v2(sequence: &str, materiel: &str) -> String {
    format!("bouchaud-manifeste-v2\nversion=2026.11.12-1\nsequence={sequence}\nmateriel={materiel}\nimage_sha256={H}\nimage_octets=4096\nessais=3\n")
}

#[test]
fn manifeste_v2_canonique() {
    let m = manifeste::analyse(v2("42", "x86_64-uefi").as_bytes()).unwrap();
    assert_eq!(m.sequence, 42);
    assert_eq!(m.materiel(), "x86_64-uefi");
    assert_eq!(m.version(), "2026.11.12-1");
    assert_eq!(m.octets, 4096);
    assert_eq!(m.essais, 3);
}

#[test]
fn manifeste_v1_toujours_lu_avec_sequence_zero() {
    let t = format!("bouchaud-manifeste-v1\nversion=bench-1\nimage_sha256={H}\nimage_octets=4096\nessais=3\n");
    let m = manifeste::analyse(t.as_bytes()).unwrap();
    assert_eq!(m.sequence, 0);
    assert_eq!(m.materiel(), "");
}

#[test]
fn manifeste_v2_non_canonique_refuse() {
    for (nom, t) in [
        ("sequence nulle", v2("0", "x86_64-uefi")),
        ("zero en tete", v2("042", "x86_64-uefi")),
        ("sequence trop longue", v2("12345678901234567890", "x86_64-uefi")),
        ("sequence non decimale", v2("4a", "x86_64-uefi")),
        ("materiel vide", v2("1", "")),
        ("materiel majuscule", v2("1", "X86")),
        ("materiel trop long", v2("1", &"a".repeat(33))),
        ("materiel espace", v2("1", "x86 64")),
        ("v1 avec champs v2", v2("1", "x86_64-uefi").replace("-v2\n", "-v1\n")),
        ("v2 sans sequence", format!("bouchaud-manifeste-v2\nversion=x\nmateriel=a\nimage_sha256={H}\nimage_octets=1\nessais=1\n")),
        ("ordre change", format!("bouchaud-manifeste-v2\nversion=x\nmateriel=a\nsequence=1\nimage_sha256={H}\nimage_octets=1\nessais=1\n")),
        ("ligne en trop", v2("1", "a") + "x=1\n"),
        ("en-tete v3", v2("1", "a").replace("-v2\n", "-v3\n")),
    ] {
        assert_eq!(manifeste::analyse(t.as_bytes()).err(), Some(manifeste::RefusManifeste::Format), "{nom}");
    }
}
