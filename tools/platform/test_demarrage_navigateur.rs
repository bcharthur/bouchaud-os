//! Quand le navigateur part, et ce qu'il emporte.
//!
//! Le releve du 16 septembre : `bureau-premiere-trame` a 5799 ms, lien
//! Ethernet monte vers 6500 ms, `navigateur-demande` a 6799 ms avec
//! `resolveur=NON-CONFIGURE`. Trois cents millisecondes de trop.

#[path = "../../src/gui/demarrage_navigateur.rs"]
mod demarrage;

use demarrage::{decide, Decision, ATTENTE_MAXIMALE_MS, REPOS_BUREAU_MS};

fn d(ms: u64, bail_possible: bool, pret: bool) -> Decision {
    decide(ms, bail_possible, pret, REPOS_BUREAU_MS, ATTENTE_MAXIMALE_MS)
}

#[test]
fn le_bureau_se_pose_d_abord() {
    assert_eq!(d(0, true, true), Decision::LaisserLeBureauSePoser);
    assert_eq!(d(499, true, true), Decision::LaisserLeBureauSePoser);
}

#[test]
fn reseau_pret_on_part_tout_de_suite() {
    assert_eq!(d(500, true, true), Decision::LancerReseauPret);
    assert!(d(500, true, true).lance());
}

#[test]
fn sans_cable_on_n_attend_rien() {
    // Huit secondes d'attente pour un bail qui ne peut pas arriver seraient
    // huit secondes volees a une machine hors ligne, dont la page d'accueil
    // est justement locale.
    assert_eq!(d(500, false, false), Decision::LancerSansReseau);
    assert!(d(500, false, false).lance());
}

#[test]
fn lien_monte_bail_en_route_on_patiente() {
    assert_eq!(d(600, true, false), Decision::AttendreLeResolveur);
    assert!(!d(600, true, false).lance());
}

#[test]
fn le_cas_exact_du_seize_septembre() {
    // 6799 - 5799 = 1000 ms apres la premiere trame, lien monte, pas de bail.
    // L'ancien code lancait; celui-ci attend.
    assert_eq!(d(1_000, true, false), Decision::AttendreLeResolveur);
}

#[test]
fn un_reseau_sans_serveur_dhcp_ne_retient_pas_le_bureau() {
    assert_eq!(d(ATTENTE_MAXIMALE_MS, true, false), Decision::LancerDelaiEcoule);
    assert_eq!(d(60_000, true, false), Decision::LancerDelaiEcoule);
    assert!(d(60_000, true, false).lance());
}

#[test]
fn le_bail_qui_arrive_pendant_l_attente_declenche_le_depart() {
    assert_eq!(d(3_000, true, false), Decision::AttendreLeResolveur);
    assert_eq!(d(3_001, true, true), Decision::LancerReseauPret);
}

#[test]
fn l_ordre_des_tests_compte() {
    // Lien bas ET delai depasse : c'est « sans reseau » qu'il faut dire, pas
    // « delai ecoule ». Le second laisserait croire qu'on a attendu un bail.
    assert_eq!(d(60_000, false, false), Decision::LancerSansReseau);
}

#[test]
fn le_repos_du_bureau_prime_sur_tout() {
    // Meme sans lien, meme resolveur pret : la premiere trame doit pouvoir
    // s'afficher avant qu'on lance quoi que ce soit.
    assert_eq!(d(0, false, false), Decision::LaisserLeBureauSePoser);
    assert!(!d(0, false, false).lance());
}

#[test]
fn seuls_les_trois_etats_de_lancement_lancent() {
    assert!(!Decision::LaisserLeBureauSePoser.lance());
    assert!(!Decision::AttendreLeResolveur.lance());
    assert!(Decision::LancerReseauPret.lance());
    assert!(Decision::LancerSansReseau.lance());
    assert!(Decision::LancerDelaiEcoule.lance());
}

#[test]
fn les_noms_de_decision_sont_ceux_du_journal() {
    // Ces chaines sortent dans BOUCHAUD_NAVIGATEUR_DEPART decision=... et
    // distinguent « on a attendu le bail » de « il n'y avait pas de cable ».
    assert_eq!(Decision::LaisserLeBureauSePoser.nom(), "bureau-se-pose");
    assert_eq!(Decision::AttendreLeResolveur.nom(), "attente-resolveur");
    assert_eq!(Decision::LancerReseauPret.nom(), "reseau-pret");
    assert_eq!(Decision::LancerSansReseau.nom(), "sans-reseau");
    assert_eq!(Decision::LancerDelaiEcoule.nom(), "delai-ecoule");
}

#[test]
fn un_lien_qui_monte_encore_n_est_pas_une_absence_de_carte() {
    // LE DEFAUT DU 16 SEPTEMBRE 18:31, EN UNE LIGNE DE JOURNAL :
    //
    //     BOUCHAUD_NAVIGATEUR_DEPART decision=sans-reseau t_ms=1322 lien=0
    //
    // « Sans reseau » sur une machine dont le cable etait branche et dont le
    // lien est monte a 1 Gbit/s quelques secondes plus tard. Le parametre
    // valait `net::connecte()`, faux pendant les ~3 s d'autonegociation
    // cuivre -- et le lire comme « pas de cable » supprimait exactement
    // l'attente qu'on venait d'ajouter.
    //
    // Ce que le parametre signifie maintenant : un bail peut-il ENCORE
    // arriver. Carte presente, lien pas encore monte => oui.
    assert_eq!(d(1_322, true, false), Decision::AttendreLeResolveur);
    assert!(!d(1_322, true, false).lance());
}

#[test]
fn sans_carte_du_tout_on_ne_perd_pas_huit_secondes() {
    // Le seul cas ou plus aucun bail ne peut arriver.
    assert_eq!(d(600, false, false), Decision::LancerSansReseau);
    assert_eq!(d(60_000, false, false), Decision::LancerSansReseau);
}
