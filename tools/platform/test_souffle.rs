//! L'enregistreur de vol doit pouvoir raconter sa propre mort.
//!
//! Ces tests existent parce que l'archive du 16 septembre 2026 ne le pouvait
//! pas : elle s'arrete a 7,30 s d'une session de vingt minutes, et le dernier
//! echantillon qu'elle contient affiche `bb_failures=0`. Le silence d'une
//! instrumentation morte est indistinguable de celui d'une machine saine tant
//! que rien ne tient le compte HORS du support qui a laché.

#[path = "../../src/kernel/debug/souffle.rs"]
mod souffle;

use souffle::{silencieux, Souffle};

const MS: u64 = 1_000_000;

#[test]
fn au_depart_rien_a_expliquer() {
    let s = Souffle::neuf();
    let e = s.etat(9_999 * MS);
    assert_eq!(e.poses, 0);
    assert_eq!(e.echecs, 0);
    // Pas de dernier succes => pas de silence a imputer a qui que ce soit.
    assert_eq!(e.silence_ms, 0);
    assert!(!silencieux(&e, 1));
}

#[test]
fn un_succes_date_le_dernier_ok() {
    let s = Souffle::neuf();
    s.succes(1_000 * MS);
    let e = s.etat(1_250 * MS);
    assert_eq!(e.poses, 1);
    assert_eq!(e.silence_ms, 250);
    assert_eq!(e.serie, 0);
}

#[test]
fn un_echec_isole_ne_declare_pas_la_mort() {
    let s = Souffle::neuf();
    s.succes(1_000 * MS);
    s.echec(1_010 * MS, 2, 53);
    let e = s.etat(1_020 * MS);
    assert_eq!(e.serie, 1);
    assert_eq!(e.silence_ms, 20);
    // Vingt millisecondes de silence ne sont pas une panne : le systeme de
    // fichiers tient la cle par intermittence, c'est normal.
    assert!(!silencieux(&e, 5_000));
}

#[test]
fn un_silence_qui_dure_avec_des_echecs_est_une_mort() {
    let s = Souffle::neuf();
    s.succes(7_047 * MS);
    for seq in 0..4_821u64 {
        s.echec((7_100 + seq) * MS, 2, 54 + seq);
    }
    let e = s.etat(1_223_000 * MS);
    assert_eq!(e.poses, 1);
    assert_eq!(e.echecs, 4_821);
    assert_eq!(e.serie, 4_821);
    assert_eq!(e.dernier_ok_ns, 7_047 * MS);
    assert!(silencieux(&e, 5_000));
}

#[test]
fn le_premier_echec_date_le_debut_du_silence() {
    let s = Souffle::neuf();
    s.succes(1_000 * MS);
    s.echec(1_100 * MS, 2, 10);
    s.echec(1_200 * MS, 2, 11);
    s.echec(1_300 * MS, 2, 12);
    // C'est le PREMIER qui date l'evenement declencheur, pas le dernier.
    assert_eq!(s.etat(2_000 * MS).premier_echec_ns, 1_100 * MS);
}

#[test]
fn un_succes_remet_la_serie_a_zero_mais_pas_la_pire() {
    let s = Souffle::neuf();
    s.succes(1_000 * MS);
    for i in 0..9u64 {
        s.echec((1_100 + i) * MS, 1, i);
    }
    s.succes(2_000 * MS);
    let e = s.etat(2_000 * MS);
    assert_eq!(e.serie, 0, "la serie en cours repart");
    assert_eq!(e.pire_serie, 9, "la pire serie ne recule jamais");
    assert_eq!(e.premier_echec_ns, 0, "le debut de silence est efface");
    assert_eq!(e.echecs, 9, "le total, lui, ne bouge pas");
}

#[test]
fn une_accalmie_ne_maquille_pas_une_longue_panne() {
    let s = Souffle::neuf();
    s.succes(1_000 * MS);
    for i in 0..1_000u64 {
        s.echec((1_001 + i) * MS, 1, i);
    }
    s.succes(9_000 * MS);            // un enregistrement repasse
    s.echec(9_001 * MS, 1, 1_000);   // puis ca recommence
    let e = s.etat(9_002 * MS);
    assert_eq!(e.serie, 1);
    // Sans `pire_serie`, cette session paraitrait n'avoir eu qu'un seul
    // echec en cours et le millier precedent serait invisible.
    assert_eq!(e.pire_serie, 1_000);
}

#[test]
fn le_dernier_perdu_est_nomme() {
    let s = Souffle::neuf();
    s.echec(500 * MS, 7, 1234);
    let e = s.etat(500 * MS);
    assert_eq!(e.dernier_genre, 7);
    assert_eq!(e.derniere_seq, 1234);
}

#[test]
fn une_machine_au_repos_n_est_pas_declaree_morte() {
    let s = Souffle::neuf();
    s.succes(1_000 * MS);
    // Une heure sans rien ecrire, mais AUCUN echec : il ne s'est simplement
    // rien passe. Exiger `serie > 0` est ce qui evite ce faux positif.
    let e = s.etat(3_601_000 * MS);
    assert_eq!(e.silence_ms, 3_600_000);
    assert_eq!(e.serie, 0);
    assert!(!silencieux(&e, 5_000));
}

#[test]
fn le_silence_ne_recule_pas_sur_une_horloge_qui_saute() {
    let s = Souffle::neuf();
    s.succes(5_000 * MS);
    // Une horloge lue avant le dernier succes donnerait un silence negatif ;
    // `saturating_sub` rend zero plutot qu'un nombre absurde.
    assert_eq!(s.etat(4_000 * MS).silence_ms, 0);
}
