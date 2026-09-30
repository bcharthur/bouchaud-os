//! Quand le navigateur part, et ce qu'il emporte.
//!
//! Le releve du 16 septembre : `bureau-premiere-trame` a 5799 ms, lien
//! Ethernet monte vers 6500 ms, `navigateur-demande` a 6799 ms avec
//! `resolveur=NON-CONFIGURE`. Trois cents millisecondes de trop.

#[path = "../../src/gui/demarrage_navigateur.rs"]
mod demarrage;

use demarrage::{decide, Commande, Decision, Portail, ATTENTE_MAXIMALE_MS, REPOS_BUREAU_MS};

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

// -- Le portail de la demande explicite (depuis p18) --------------------------
//
// Le bureau ne lance plus le navigateur tout seul ; il obeit a une demande.
// Le portail retient celle qui arrive avant que le reseau ait eu sa chance.

/// Un tour de bureau : `decide` puis le portail, dans l'ordre du
/// `window_manager`. Rend ce qui est execute, et l'etat ouvert du portail.
fn tour(
    portail: &mut Portail,
    ouvert: &mut bool,
    t_ms: u64,
    bail_possible: bool,
    pret: bool,
    commande: Commande,
) -> Commande {
    if !*ouvert && d(t_ms, bail_possible, pret).lance() {
        *ouvert = true;
    }
    portail.filtre(commande, *ouvert)
}

#[test]
fn sur_un_reseau_pret_la_demande_part_au_tour_meme() {
    // Ce que p18 a voulu et que le portail ne doit pas abimer : sur un
    // reseau qui marche, cliquer lance.
    let mut p = Portail::neuf();
    assert_eq!(p.filtre(Commande::Demarrer, true), Commande::Demarrer);
    assert!(!p.retient());
}

#[test]
fn le_portail_seul_ne_lance_jamais_rien() {
    // L'invariant de p18 : pas de demarrage automatique. S'ouvrir n'est pas
    // lancer ; seule une demande lance.
    let mut p = Portail::neuf();
    for _ in 0..3 {
        assert_eq!(p.filtre(Commande::Aucune, true), Commande::Aucune);
    }
}

#[test]
fn le_seize_septembre_rejoue_avec_un_clic() {
    // Premiere trame posee, clic a 1000 ms, lien monte mais pas de bail : la
    // demande attend. Le bail arrive a 3001 ms : elle part a ce tour-la, pas
    // avant, et une seule fois.
    let mut p = Portail::neuf();
    let mut ouvert = false;
    assert_eq!(tour(&mut p, &mut ouvert, 1_000, true, false, Commande::Demarrer), Commande::Aucune);
    assert!(p.retient());
    assert_eq!(tour(&mut p, &mut ouvert, 2_000, true, false, Commande::Aucune), Commande::Aucune);
    assert_eq!(tour(&mut p, &mut ouvert, 3_001, true, true, Commande::Aucune), Commande::Demarrer);
    assert!(!p.retient());
    assert_eq!(tour(&mut p, &mut ouvert, 4_000, true, true, Commande::Aucune), Commande::Aucune);
}

#[test]
fn sans_serveur_dhcp_la_demande_part_au_bout_de_l_attente_bornee() {
    let mut p = Portail::neuf();
    let mut ouvert = false;
    assert_eq!(tour(&mut p, &mut ouvert, 1_000, true, false, Commande::Demarrer), Commande::Aucune);
    assert_eq!(
        tour(&mut p, &mut ouvert, ATTENTE_MAXIMALE_MS - 1, true, false, Commande::Aucune),
        Commande::Aucune
    );
    assert_eq!(
        tour(&mut p, &mut ouvert, ATTENTE_MAXIMALE_MS, true, false, Commande::Aucune),
        Commande::Demarrer
    );
}

#[test]
fn sans_carte_la_demande_n_attend_pas() {
    let mut p = Portail::neuf();
    let mut ouvert = false;
    assert_eq!(
        tour(&mut p, &mut ouvert, REPOS_BUREAU_MS, false, false, Commande::Demarrer),
        Commande::Demarrer
    );
}

#[test]
fn une_fois_ouvert_le_portail_ne_se_referme_pas() {
    // L'arbitrage porte sur le PREMIER bail. Un bail perdu plus tard ne doit
    // pas retenir une relance -- la supervision la demande justement quand
    // quelque chose va mal.
    let mut p = Portail::neuf();
    let mut ouvert = false;
    assert_eq!(tour(&mut p, &mut ouvert, 600, true, true, Commande::Demarrer), Commande::Demarrer);
    assert_eq!(tour(&mut p, &mut ouvert, 900, true, false, Commande::Demarrer), Commande::Demarrer);
}

#[test]
fn un_arret_n_attend_jamais_et_annule_la_demande_retenue() {
    let mut p = Portail::neuf();
    assert_eq!(p.filtre(Commande::Demarrer, false), Commande::Aucune);
    assert_eq!(p.filtre(Commande::Arreter, false), Commande::Arreter);
    assert!(!p.retient());
    // Le reseau arrive ensuite : on ne relance pas ce qu'on a demande
    // d'arreter.
    assert_eq!(p.filtre(Commande::Aucune, true), Commande::Aucune);
}

#[test]
fn deux_clics_avant_le_bail_ne_lancent_qu_une_fois() {
    let mut p = Portail::neuf();
    assert_eq!(p.filtre(Commande::Demarrer, false), Commande::Aucune);
    assert_eq!(p.filtre(Commande::Demarrer, false), Commande::Aucune);
    assert_eq!(p.filtre(Commande::Aucune, true), Commande::Demarrer);
    assert_eq!(p.filtre(Commande::Aucune, true), Commande::Aucune);
}

#[test]
fn le_redemarrage_traverse_sans_toucher_la_retenue() {
    // Sa phase d'arret n'a rien a attendre ; sa phase de demarrage revient
    // au tour suivant comme un Demarrer et passe alors par le portail.
    let mut p = Portail::neuf();
    assert_eq!(p.filtre(Commande::Autre, false), Commande::Autre);
    assert!(!p.retient());
    assert_eq!(p.filtre(Commande::Demarrer, false), Commande::Aucune);
    assert_eq!(p.filtre(Commande::Autre, false), Commande::Autre);
    assert!(p.retient());
}
