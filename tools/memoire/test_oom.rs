//! BOUCHAUD_OOM_CONTROLE_V1 -- la politique OOM, contredite sur l'hote.
//!
//! Ce que ces epreuves defendent : quand la memoire manque, c'est le GOUFFRE
//! qui meurt, pas le processus qui fautait au mauvais moment ; et l'on ne tue
//! pas deux fois pour la meme penurie tant que la premiere victime meurt.

#[path = "../../src/kernel/memory/oom.rs"]
mod oom;

use oom::{action, choisit_victime, verdict, Action, Candidat, Verdict, VictimeEnCours, DELAI_MORT_NS};

fn c(pid: u32, pages: u64) -> Candidat {
    Candidat { pid, pages, zombie: false, emprunteur: false, protege: false }
}

#[test]
fn une_reclamation_fructueuse_suffit() {
    assert_eq!(action(1, None, 0), Action::Reessaie);
    let v = VictimeEnCours { pid: 9, tuee_a_ns: 0, vivante: true };
    assert_eq!(action(3, Some(v), 1), Action::Reessaie);
}

#[test]
fn une_victime_qui_meurt_est_attendue_et_pas_doublee() {
    let v = VictimeEnCours { pid: 9, tuee_a_ns: 1_000, vivante: true };
    assert_eq!(action(0, Some(v), 1_000 + DELAI_MORT_NS - 1), Action::AttendsVictime);
}

#[test]
fn une_victime_morte_ou_trop_lente_laisse_choisir_la_suivante() {
    let morte = VictimeEnCours { pid: 9, tuee_a_ns: 0, vivante: false };
    assert_eq!(action(0, Some(morte), 10), Action::ChoisisVictime);
    let lente = VictimeEnCours { pid: 9, tuee_a_ns: 0, vivante: true };
    assert_eq!(action(0, Some(lente), DELAI_MORT_NS), Action::ChoisisVictime);
    assert_eq!(action(0, None, 0), Action::ChoisisVictime);
}

#[test]
fn le_gouffre_meurt_pas_le_fautif() {
    // Le bureau (pid 3) faute ; le gouffre (pid 40) tient tout.
    let candidats = [c(3, 2_000), c(40, 400_000), c(41, 10_000)];
    assert_eq!(verdict(3, &candidats), Verdict::Tue(40));
}

#[test]
fn le_fautif_qui_est_le_gouffre_meurt_seul() {
    let candidats = [c(3, 2_000), c(40, 400_000)];
    assert_eq!(verdict(40, &candidats), Verdict::Abandonne);
}

#[test]
fn ni_zombie_ni_emprunteur_ni_protege_ni_vide() {
    let candidats = [
        Candidat { zombie: true, ..c(10, 900_000) },
        Candidat { emprunteur: true, ..c(11, 800_000) },
        Candidat { protege: true, ..c(12, 700_000) },
        c(13, 0),
        c(14, 5),
    ];
    assert_eq!(choisit_victime(&candidats), Some(14));
    assert_eq!(choisit_victime(&candidats[..4]), None);
    assert_eq!(verdict(1, &candidats[..4]), Verdict::Abandonne);
}

#[test]
fn a_egalite_le_plus_jeune() {
    assert_eq!(choisit_victime(&[c(20, 100), c(25, 100), c(22, 100)]), Some(25));
}
