//! Ecriture differee et dossiers personnels : regles pures
//! (BOUCHAUD_ECRITURE_DIFFEREE_V1, BOUCHAUD_DOSSIERS_PERSONNELS_V1).

#![allow(dead_code)]
extern crate alloc;

#[path = "../../src/fs/ecriture_differee_politique.rs"]
mod politique;
#[path = "../../src/fs/dossiers_politique.rs"]
mod dossiers;

use politique::*;

/// Simule le fil : observe toutes les 250 ms de `de` a `a`, la generation
/// rendue par `g(t)`. Rend les instants ou il aurait ecrit.
fn simule(etat: &mut Etat, de: u64, a: u64, g: impl Fn(u64) -> u64, ok: bool) -> Vec<u64> {
    let mut ecrits = Vec::new();
    let mut t = de;
    while t <= a {
        let gen = g(t);
        if etat.decide(gen, t) {
            if ok { etat.reussie(gen); } else { etat.echouee(t); }
            ecrits.push(t);
        }
        t += 250;
    }
    ecrits
}

#[test]
fn rien_a_ecrire_rien_n_est_ecrit() {
    let mut e = Etat::neuf(7);
    assert!(simule(&mut e, 0, 600_000, |_| 7, true).is_empty(), "dix minutes sans modification");
}

#[test]
fn une_modification_isolee_est_ecrite_apres_le_calme_et_une_seule_fois() {
    let mut e = Etat::neuf(0);
    let ecrits = simule(&mut e, 0, 60_000, |t| if t >= 1_000 { 1 } else { 0 }, true);
    assert_eq!(ecrits.len(), 1);
    assert!(ecrits[0] >= 1_000 + CALME_MS && ecrits[0] <= 1_000 + CALME_MS + 250, "{ecrits:?}");
}

#[test]
fn une_rafale_ne_donne_qu_une_transaction() {
    let mut e = Etat::neuf(0);
    // 40 modifications en 1 seconde, puis plus rien.
    let ecrits = simule(&mut e, 0, 30_000, |t| (t.min(1_000) / 25), true);
    assert_eq!(ecrits.len(), 1, "{ecrits:?}");
}

#[test]
fn une_ecriture_continue_ne_repousse_pas_la_durabilite_au_dela_du_plafond() {
    let mut e = Etat::neuf(0);
    // Une modification toutes les 250 ms pendant 30 s : jamais de calme.
    let ecrits = simule(&mut e, 0, 30_000, |t| t / 250 + 1, true);
    assert!(!ecrits.is_empty());
    assert!(ecrits[0] <= PLAFOND_MS + 250, "premiere ecriture a {}", ecrits[0]);
    for w in ecrits.windows(2) {
        assert!(w[1] - w[0] <= PLAFOND_MS + 250, "ecart {}", w[1] - w[0]);
    }
}

#[test]
fn un_echec_est_retente_avec_un_delai_qui_double_et_reste_borne() {
    let mut e = Etat::neuf(0);
    let essais = simule(&mut e, 0, 600_000, |t| if t >= 100 { 1 } else { 0 }, false);
    assert!(essais.len() >= 3);
    let ecarts: Vec<u64> = essais.windows(2).map(|w| w[1] - w[0]).collect();
    for w in ecarts.windows(2) {
        assert!(w[1] >= w[0], "le delai ne diminue pas : {ecarts:?}");
    }
    assert!(*ecarts.last().unwrap() <= ECHEC_MAX_MS + 250, "borne : {ecarts:?}");
    assert!(essais.len() < 40, "pas de boucle : {} essais en dix minutes", essais.len());
}

#[test]
fn une_modification_pendant_la_transaction_reste_a_ecrire() {
    let mut e = Etat::neuf(0);
    assert!(!e.decide(1, 0));
    assert!(e.decide(1, CALME_MS));
    // La transaction a capture 1 ; entre-temps, 2.
    e.reussie(1);
    assert_eq!(e.ecrite, 1);
    assert!(!e.decide(2, CALME_MS + 10), "2 vient d'arriver : calme pas atteint");
    assert!(e.decide(2, 2 * CALME_MS + 20), "2 est ecrit a son tour");
}

#[test]
fn le_dossier_personnel_est_sous_persist_quand_il_y_a_une_zone() {
    assert_eq!(dossiers::chemin("arthur", true, "/home/arthur"), "/persist/home/arthur");
    assert_eq!(dossiers::chemin("root", true, "/"), "/persist/home/root");
    assert_eq!(dossiers::chemin("root", false, "/"), "/", "sans zone : l'accueil declare");
    assert_eq!(dossiers::chemin("../x", true, "/home/x"), "/home/x", "nom refuse : pas d'evasion");
    assert_eq!(dossiers::chemin("", true, "/"), "/");
}

#[test]
fn les_sous_dossiers_sont_ceux_de_la_barre_laterale() {
    assert_eq!(dossiers::SOUS_DOSSIERS, ["Documents", "Images", "T\u{e9}l\u{e9}chargements"]);
    assert_eq!(dossiers::MODE_PERSONNEL, 0o700, "prive");
}
