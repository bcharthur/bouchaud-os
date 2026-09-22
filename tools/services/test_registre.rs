//! Le registre des services dit-il la verite, et une seule fois ?
//!
//! # Les deux defauts que ces tests defendent
//!
//! 1. UNE SEULE SOURCE. Un compteur dans `netetat`, un autre dans une
//!    interface, un troisieme dans l'archive et un quatrieme dans l'ecran de
//!    demarrage divergent le jour ou ils comptent -- et c'est toujours ce
//!    jour-la qu'on les lit.
//!
//! 2. LE DIAGNOSTIC NE DOIT PAS EFFACER CE QU'IL EXPLIQUE. Le releve physique
//!    du 17 septembre a perdu tout son demarrage :
//!
//!    ```text
//!    tambour_reserves=12736 ecrases=4544 perdus=4209
//!    premier enregistrement survivant : seq=4546, t~287 s
//!    ```
//!
//!    Un service qui reste actif sans erreur pendant cinq minutes ne doit donc
//!    ecrire AUCUNE ligne. Seuls les changements d'etat comptent.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/services/registre.rs"]
mod registre;

use registre::{
    pic_a_enregistrer, Etat, Genre, Registre, ID_MAX, REPOS_PIC_NS, SERVICES_MAX,
    SEUIL_PIC_REVEIL_US,
};

const MS: u64 = 1_000_000;
const S: u64 = 1_000_000_000;

fn registre_reseau() -> Registre {
    let mut r = Registre::neuf();
    r.declare("net", "", Genre::Groupe);
    r.declare("net.rtl8168", "net", Genre::Pilote);
    r.declare("net.arp", "net", Genre::Protocole);
    r.declare("net.dhcp", "net", Genre::Protocole);
    r.declare("net.dns", "net", Genre::Protocole);
    r
}

// ===========================================================================
// L'arbre
// ===========================================================================

#[test]
fn un_service_declare_deux_fois_ne_se_duplique_pas() {
    let mut r = Registre::neuf();
    assert!(r.declare("net.dns", "net", Genre::Protocole));
    assert!(r.declare("net.dns", "net", Genre::Protocole));
    assert_eq!(r.entrees().len(), 1);
    assert_eq!(r.compteurs().enregistres, 1);
}

#[test]
fn l_arbre_se_parcourt_par_les_parents() {
    let r = registre_reseau();
    let enfants: Vec<&str> = r.enfants("net").map(|e| e.id.texte()).collect();
    assert_eq!(enfants, vec!["net.rtl8168", "net.arp", "net.dhcp", "net.dns"]);
    assert_eq!(r.enfants("net.dns").count(), 0);
}

#[test]
fn le_genre_distingue_un_protocole_d_un_processus() {
    // L'arbre represente la RESPONSABILITE : fabriquer un fil d'execution par
    // ligne d'arborescence serait une fiction couteuse.
    let r = registre_reseau();
    assert_eq!(r.lis("net.dns").unwrap().genre, Genre::Protocole);
    assert_eq!(r.lis("net.rtl8168").unwrap().genre, Genre::Pilote);
}

#[test]
fn un_registre_plein_refuse_et_le_compte() {
    // Une borne qui se tait est une borne qu'on decouvre trop tard : le
    // service qu'on cherche aurait disparu de l'arbre sans un mot.
    let mut r = Registre::neuf();
    for i in 0..SERVICES_MAX {
        let mut nom = String::from("s");
        nom.push_str(&i.to_string());
        assert!(r.declare(&nom, "", Genre::Service), "refus premature a {i}");
    }
    assert!(!r.declare("de.trop", "", Genre::Service));
    assert_eq!(r.compteurs().refuses, 1);
    assert_eq!(r.entrees().len(), SERVICES_MAX);
}

#[test]
fn les_pid_navigateur_morts_se_recyclent_quand_le_registre_est_plein() {
    // BOUCHAUD_V13_RECYCLAGE_PID : une longue session ne doit pas tuer
    // Services apres 128 onglets/workers historiques.
    let mut r = Registre::neuf();
    r.declare("browser.web_content", "browser", Genre::Processus);
    for i in 1..SERVICES_MAX {
        let id = format!("browser.web_content.{i}");
        assert!(r.declare(&id, "browser.web_content", Genre::Processus));
        r.etat(&id, Etat::Arrete, i as u64);
    }
    assert_eq!(r.entrees().len(), SERVICES_MAX);
    assert!(r.declare("browser.web_content.999999", "browser.web_content", Genre::Processus));
    assert!(r.lis("browser.web_content.999999").is_some());
    assert_eq!(r.compteurs().refuses, 0);
    assert_eq!(r.entrees().len(), SERVICES_MAX);
}

#[test]
fn un_identifiant_trop_long_est_tronque_sans_deborder() {
    let mut r = Registre::neuf();
    let long = "net.un.identifiant.vraiment.beaucoup.trop.long";
    assert!(r.declare(long, "net", Genre::Protocole));
    let entree = r.entrees()[0];
    assert_eq!(entree.id.texte().len(), ID_MAX);
    assert!(long.starts_with(entree.id.texte()));
}

// ===========================================================================
// Les etats, et ce qui merite d'etre ecrit
// ===========================================================================

#[test]
fn un_service_qui_ne_change_pas_n_ecrit_rien() {
    // LE DEFAUT bb(3), EN UNE ASSERTION. Cinq minutes d'activite paisible ne
    // doivent pas produire trois cents lignes.
    let mut r = registre_reseau();
    assert!(r.etat("net.dns", Etat::Demarrage, 10 * MS));
    assert!(r.etat("net.dns", Etat::Actif, 20 * MS));
    let mut ecritures = 0;
    for seconde in 1..=300u64 {
        if r.etat("net.dns", Etat::Actif, seconde * S) {
            ecritures += 1;
        }
    }
    assert_eq!(ecritures, 0, "{ecritures} lignes pour un service qui va bien");
    assert_eq!(r.compteurs().transitions, 2);
}

#[test]
fn une_degradation_s_ecrit_a_l_instant() {
    let mut r = registre_reseau();
    r.etat("net.rtl8168", Etat::Demarrage, MS);
    r.etat("net.rtl8168", Etat::Actif, 2 * MS);
    assert!(
        r.etat("net.rtl8168", Etat::Degrade, 3 * S),
        "une degradation passe inapercue"
    );
    // Et le retour a la normale aussi : sans lui, on ne saurait pas que c'est
    // fini.
    assert!(r.etat("net.rtl8168", Etat::Actif, 5 * S));
}

#[test]
fn la_reprise_se_compte() {
    let mut r = registre_reseau();
    r.etat("net.rtl8168", Etat::Actif, MS);
    r.etat("net.rtl8168", Etat::Degrade, S);
    r.etat("net.rtl8168", Etat::Reprise, 2 * S);
    r.etat("net.rtl8168", Etat::Actif, 3 * S);
    r.etat("net.rtl8168", Etat::Degrade, 10 * S);
    r.etat("net.rtl8168", Etat::Reprise, 11 * S);
    assert_eq!(r.lis("net.rtl8168").unwrap().reprises, 2);
}

#[test]
fn une_erreur_garde_sa_raison_et_sa_date() {
    let mut r = registre_reseau();
    r.etat("net.dns", Etat::Actif, MS);
    r.erreur("net.dns", "timeout", 4 * S);
    let e = r.lis("net.dns").unwrap();
    assert_eq!(e.erreurs, 1);
    assert_eq!(e.raison.texte(), "timeout");
    assert_eq!(e.derniere_erreur_ns, 4 * S);
    // Le succes suivant ne l'efface pas : « derniere erreur il y a 2,4 s » est
    // exactement ce qu'il faut afficher.
    r.succes("net.dns", 6 * S);
    let e = r.lis("net.dns").unwrap();
    assert_eq!(e.derniere_erreur_ns, 4 * S);
    assert_eq!(e.dernier_succes_ns, 6 * S);
}

// ===========================================================================
// Les phases de demarrage : la meme source, pas une deuxieme
// ===========================================================================

#[test]
fn la_duree_de_demarrage_se_lit_sur_le_registre() {
    // C'est le chiffre de l'ecran de demarrage -- « USB pret 210 ms » -- et il
    // vient du MEME registre que l'observatoire et l'archive.
    let mut r = Registre::neuf();
    r.declare("usb", "systeme", Genre::Noyau);
    assert_eq!(r.lis("usb").unwrap().duree_demarrage_ms(), None);
    r.etat("usb", Etat::Demarrage, 15 * MS);
    assert_eq!(
        r.lis("usb").unwrap().duree_demarrage_ms(),
        None,
        "une duree affichee avant d'etre prete est une duree fabriquee"
    );
    r.etat("usb", Etat::Actif, 225 * MS);
    assert_eq!(r.lis("usb").unwrap().duree_demarrage_ms(), Some(210));
}

#[test]
fn un_redemarrage_ne_cumule_pas_la_vie_precedente() {
    let mut r = registre_reseau();
    r.etat("net.dhcp", Etat::Demarrage, 100 * MS);
    r.etat("net.dhcp", Etat::Actif, 400 * MS);
    assert_eq!(r.lis("net.dhcp").unwrap().duree_demarrage_ms(), Some(300));

    // Le lien tombe, DHCP repart.
    r.etat("net.dhcp", Etat::Arrete, 10 * S);
    r.etat("net.dhcp", Etat::Demarrage, 20 * S);
    assert_eq!(
        r.lis("net.dhcp").unwrap().duree_demarrage_ms(),
        None,
        "la duree affichee contient encore le demarrage precedent"
    );
    r.etat("net.dhcp", Etat::Actif, 20 * S + 250 * MS);
    assert_eq!(r.lis("net.dhcp").unwrap().duree_demarrage_ms(), Some(250));
    assert_eq!(r.lis("net.dhcp").unwrap().redemarrages, 1);
}

#[test]
fn le_pire_service_se_voit_d_un_coup_d_oeil() {
    let mut r = registre_reseau();
    r.etat("net.rtl8168", Etat::Actif, MS);
    r.etat("net.arp", Etat::Actif, MS);
    r.etat("net.dns", Etat::Actif, MS);
    assert!(r.pire().is_none(), "rien ne va mal, et pourtant quelque chose est signale");

    r.etat("net.dns", Etat::Degrade, S);
    assert_eq!(r.pire().unwrap().id.texte(), "net.dns");

    // Une panne passe devant une degradation.
    r.etat("net.rtl8168", Etat::Panne, 2 * S);
    assert_eq!(r.pire().unwrap().id.texte(), "net.rtl8168");

    // Un service arrete proprement n'est pas une alarme.
    let mut propre = Registre::neuf();
    propre.declare("net.dhcp", "net", Genre::Protocole);
    propre.etat("net.dhcp", Etat::Arrete, S);
    assert!(propre.pire().is_none());
}

// ===========================================================================
// Les pics de latence
// ===========================================================================

#[test]
fn un_reveil_normal_ne_produit_aucun_evenement() {
    // « Ne logue PAS chaque reveil. »
    for us in [0u64, 100, 5_000, SEUIL_PIC_REVEIL_US - 1] {
        assert!(
            !pic_a_enregistrer(us, 0, 0, 10 * S, REPOS_PIC_NS),
            "{us} us enregistre comme un pic"
        );
    }
}

#[test]
fn le_premier_pic_s_enregistre_toujours() {
    assert!(pic_a_enregistrer(SEUIL_PIC_REVEIL_US, 0, 0, 10 * S, REPOS_PIC_NS));
    // Le releve physique : 12,18 secondes.
    assert!(pic_a_enregistrer(12_179_860, 0, 0, 10 * S, REPOS_PIC_NS));
}

#[test]
fn une_machine_qui_souffre_n_efface_pas_son_archive() {
    // Mille pics par seconde effaceraient ce qu'ils doivent expliquer. Apres
    // un premier pic, le suivant attend le repos.
    let premier = 10 * S;
    assert!(!pic_a_enregistrer(25_000, premier, 25_000, premier + S, REPOS_PIC_NS));
    assert!(!pic_a_enregistrer(30_000, premier, 25_000, premier + 4 * S, REPOS_PIC_NS));
    assert!(pic_a_enregistrer(30_000, premier, 25_000, premier + 5 * S, REPOS_PIC_NS));
}

#[test]
fn un_pic_nettement_pire_passe_avant_le_repos() {
    // Un pic dix fois plus grand que le precedent est une information neuve,
    // et l'attendre cinq secondes la perdrait.
    let premier = 10 * S;
    assert!(pic_a_enregistrer(500_000, premier, 25_000, premier + S, REPOS_PIC_NS));
    // Mais pas un pic a peine plus grand.
    assert!(!pic_a_enregistrer(26_000, premier, 25_000, premier + S, REPOS_PIC_NS));
}
