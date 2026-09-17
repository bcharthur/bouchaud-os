//! La fenetre Services montre-t-elle VRAIMENT l'arbre de la pile ?
//!
//! # Le test d'acceptation qui etait rouge
//!
//! La photo physique du 17 septembre montre la fenetre Services affichant
//! encore six lignes Ladybird codees en dur :
//!
//! ```text
//! BouchaudBrowserHost   En cours PID [...]
//! WebContent            En cours PID [...]
//! RequestServer         En cours PID [...]
//! ImageDecoder          En cours PID [...]
//! Compositor            En cours PID [...]
//! WebWorker             Arrete / a la demande
//! Reseau : pret
//! ```
//!
//! Pas une ligne de Systeme, de Reseau, ni de Navigateur. Le registre central
//! existait ; la vue ne s'en servait pas.
//!
//! Ces tests portent sur le MODELE VISIBLE, pas sur le rendu : ils exigent que
//! l'arbre construit depuis le registre contienne ce que l'utilisateur doit
//! voir. Une mutation qui revient aux six lignes Ladybird les fait echouer.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/services"]
mod services {
    pub mod registre;
    pub mod vue;
}

use services::registre::{
    declare_topologie, Etat, Genre, Kpi, Registre, SERVICES_MAX, TOPOLOGIE,
};
use services::vue::{
    couleur_etat, etat_affiche, etat_agrege, gravite, lignes, porte_un_pid, repli_par_defaut,
    Ligne, Replies, RACINES,
};

fn registre_complet() -> Registre {
    let mut r = Registre::neuf();
    assert_eq!(declare_topologie(&mut r), 0);
    r
}

fn modele(r: &Registre, replies: &Replies) -> Vec<(String, u8)> {
    let mut tampon = vec![Ligne::vide(); SERVICES_MAX];
    let n = lignes(r.entrees(), replies, &mut tampon);
    tampon[..n]
        .iter()
        .map(|l| (l.entree.id.texte().to_string(), l.profondeur))
        .collect()
}

// ===========================================================================
// La topologie tient dans le registre
// ===========================================================================

#[test]
fn topologie_complete_enregistrable() {
    // LA BORNE NE DOIT PAS TRONQUER L'ARBRE EN SILENCE.
    //
    // Trente-deux entrees suffisaient a la poignee de services de la premiere
    // version ; la topologie reelle en compte soixante-sept. Un arbre tronque
    // fait disparaitre de la vue le service qu'on y cherche, sans un mot.
    let mut r = Registre::neuf();
    let refuses = declare_topologie(&mut r);
    assert_eq!(refuses, 0, "{refuses} service(s) refuse(s) : la borne est trop courte");
    assert_eq!(r.compteurs().refuses, 0);
    assert_eq!(r.entrees().len(), TOPOLOGIE.len());
    assert!(
        TOPOLOGIE.len() <= SERVICES_MAX,
        "la topologie ({}) depasse la borne ({SERVICES_MAX})",
        TOPOLOGIE.len()
    );
}

#[test]
fn chaque_parent_declare_existe() {
    // Un parent absent detache silencieusement tout son sous-arbre de la vue.
    let r = registre_complet();
    for entree in r.entrees() {
        if entree.parent.est_vide() {
            continue;
        }
        assert!(
            r.lis(entree.parent.texte()).is_some(),
            "{} est rattache a « {} », qui n'existe pas",
            entree.id.texte(),
            entree.parent.texte()
        );
    }
}

#[test]
fn les_racines_sont_exactement_systeme_reseau_navigateur() {
    assert_eq!(RACINES, ["sys", "net", "browser"]);
    let r = registre_complet();
    let racines: Vec<&str> = r
        .entrees()
        .iter()
        .filter(|e| e.parent.est_vide())
        .map(|e| e.id.texte())
        .collect();
    assert_eq!(racines, vec!["sys", "net", "browser"]);
}

// ===========================================================================
// Le modele visible
// ===========================================================================

#[test]
fn la_vue_montre_systeme_reseau_navigateur() {
    // LE TEST D'ACCEPTATION. Une vue qui revient aux six lignes Ladybird
    // echoue ici.
    let r = registre_complet();
    let vue = modele(&r, &Replies::neuf());
    let ids: Vec<&str> = vue.iter().map(|(id, _)| id.as_str()).collect();

    assert!(ids.contains(&"sys"), "« Systeme » absent de la vue");
    assert!(ids.contains(&"net"), "« Reseau » absent de la vue");
    assert!(ids.contains(&"browser"), "« Navigateur » absent de la vue");
    assert_eq!(vue[0].0, "sys");
    assert_eq!(vue[0].1, 0, "une racine ne s'indente pas");
}

#[test]
fn la_vue_contient_la_chaine_reseau_et_le_navigateur() {
    let r = registre_complet();
    let vue = modele(&r, &Replies::neuf());
    let ids: Vec<&str> = vue.iter().map(|(id, _)| id.as_str()).collect();

    // Ce que l'utilisateur doit pouvoir lire d'un coup d'oeil quand une page
    // echoue : la chaine entiere, de la carte jusqu'au moteur.
    for attendu in [
        "net.nic.rtl8168",
        "net.arp",
        "net.dhcp",
        "net.dns",
        "net.tcp",
        "net.tls",
        "net.http1",
        "browser.request_server",
        "browser.web_content",
        "browser.compositor",
        "browser.navigation",
    ] {
        assert!(ids.contains(&attendu), "« {attendu} » absent de la vue");
    }
}

#[test]
fn la_vue_compte_au_moins_vingt_lignes() {
    let r = registre_complet();
    let vue = modele(&r, &Replies::neuf());
    assert!(
        vue.len() >= 20,
        "{} lignes visibles : la vue est retombee a une poignee d'entrees",
        vue.len()
    );
    assert_eq!(vue.len(), TOPOLOGIE.len(), "tout deploye, tout doit etre visible");
}

#[test]
fn l_arbre_s_affiche_dans_l_ordre_de_declaration() {
    // Trier par nom mettrait `net.arp` avant `net.ethernet` et ferait
    // apparaitre ARP au-dessus de la couche qui le porte.
    let r = registre_complet();
    let vue = modele(&r, &Replies::neuf());
    let ids: Vec<&str> = vue.iter().map(|(id, _)| id.as_str()).collect();
    let ethernet = ids.iter().position(|i| *i == "net.ethernet").unwrap();
    let arp = ids.iter().position(|i| *i == "net.arp").unwrap();
    assert!(ethernet < arp, "ARP s'affiche au-dessus de la couche qui le porte");
}

#[test]
fn la_profondeur_suit_la_parente() {
    let r = registre_complet();
    let vue = modele(&r, &Replies::neuf());
    let profondeur = |cible: &str| {
        vue.iter().find(|(id, _)| id == cible).map(|(_, p)| *p).unwrap()
    };
    assert_eq!(profondeur("net"), 0);
    assert_eq!(profondeur("net.nic"), 1);
    assert_eq!(profondeur("net.nic.rtl8168"), 2);
    assert_eq!(profondeur("browser.navigation.layout"), 2);
}

// ===========================================================================
// Replier et deployer
// ===========================================================================

#[test]
fn tout_est_deploye_au_premier_affichage() {
    // Une vue qui s'ouvre repliee ne montre rien de ce qu'on vient d'y
    // chercher.
    let replies = Replies::neuf();
    assert!(replies.deploye("sys"));
    assert!(replies.deploye("net.l4"));
}

#[test]
fn replier_un_noeud_cache_ses_enfants_et_lui_seul() {
    let r = registre_complet();
    let mut replies = Replies::neuf();
    replies.replie("sys");
    let vue = modele(&r, &replies);
    let ids: Vec<&str> = vue.iter().map(|(id, _)| id.as_str()).collect();

    assert!(ids.contains(&"sys"), "le noeud replie disparait au lieu de se replier");
    assert!(!ids.contains(&"sys.scheduler"), "les enfants restent visibles");
    // Les autres racines ne bougent pas.
    assert!(ids.contains(&"net"));
    assert!(ids.contains(&"net.nic.rtl8168"));
    assert!(ids.contains(&"browser.web_content"));
}

#[test]
fn replier_puis_deployer_rend_l_arbre_intact() {
    let r = registre_complet();
    let mut replies = Replies::neuf();
    let complet = modele(&r, &replies).len();
    assert!(!replies.bascule("net"));
    assert!(modele(&r, &replies).len() < complet);
    assert!(replies.bascule("net"));
    assert_eq!(modele(&r, &replies).len(), complet);
}

#[test]
fn replier_un_sous_arbre_ne_cache_pas_les_petits_enfants_des_autres() {
    let r = registre_complet();
    let mut replies = Replies::neuf();
    replies.replie("net.application");
    let vue = modele(&r, &replies);
    let ids: Vec<&str> = vue.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"net.application"));
    assert!(!ids.contains(&"net.http1"));
    assert!(ids.contains(&"net.tls"), "un frere a ete replie par erreur");
}

// ===========================================================================
// Ce que chaque ligne montre
// ===========================================================================

#[test]
fn le_libelle_est_le_dernier_segment() {
    let r = registre_complet();
    let mut tampon = vec![
        Ligne::vide();
        SERVICES_MAX
    ];
    let n = lignes(r.entrees(), &Replies::neuf(), &mut tampon);
    let ligne = tampon[..n]
        .iter()
        .find(|l| l.entree.id.egale("net.nic.rtl8168"))
        .unwrap();
    // L'arbre porte deja le contexte ; le repeter volerait la largeur des
    // colonnes.
    assert_eq!(ligne.libelle(), "rtl8168");
    assert_eq!(ligne.profondeur, 2);
    assert!(!ligne.a_des_enfants);
}

#[test]
fn un_protocole_n_a_pas_de_pid() {
    // « Pour les protocoles : PAS de faux PID. » DNS n'est pas un processus,
    // et lui en inventer un ferait chercher un fil qui n'existe pas.
    assert!(!porte_un_pid(Genre::Protocole));
    assert!(!porte_un_pid(Genre::Groupe));
    assert!(!porte_un_pid(Genre::Etape));
    assert!(porte_un_pid(Genre::Processus));
}

#[test]
fn une_mesure_absente_s_affiche_n_a_et_jamais_zero() {
    let mut r = registre_complet();
    // Un protocole ne publie ni memoire residente ni PID.
    let dns = r.lis("net.dns").unwrap();
    assert_eq!(dns.kpi.rss_octets, None);
    assert_eq!(dns.kpi.pid, None);
    assert_eq!(etat_affiche(Etat::Inconnu), "N/A");

    // Publier une latence n'invente pas une memoire.
    r.kpi(
        "net.dns",
        Kpi { latence_us: Some(2_050_000), operations: Some(7), ..Kpi::default() },
        1_000,
    );
    let dns = r.lis("net.dns").unwrap();
    assert_eq!(dns.kpi.latence_us, Some(2_050_000));
    assert_eq!(dns.kpi.rss_octets, None, "une mesure absente est devenue zero");
}

#[test]
fn les_etats_degrades_se_distinguent_visuellement() {
    // Sobre, mais distinct : un etat degrade doit sauter aux yeux sans que
    // tout le tableau soit en couleur.
    assert_ne!(couleur_etat(Etat::Actif), couleur_etat(Etat::Degrade));
    assert_ne!(couleur_etat(Etat::Degrade), couleur_etat(Etat::Panne));
    assert_ne!(couleur_etat(Etat::Actif), couleur_etat(Etat::Panne));
    assert_eq!(etat_affiche(Etat::Degrade), "Degrade");
}

#[test]
fn un_service_inactif_reste_dans_l_arbre() {
    // « WebWorker arrete, DHCP repos, TLS attente, HTTP/2 N/A. Il ne disparait
    // pas de l'arbre. »
    let mut r = registre_complet();
    r.etat("browser.web_worker", Etat::Arrete, 1_000);
    r.etat("net.dhcp", Etat::Repos, 1_000);
    r.etat("net.tls", Etat::Attente, 1_000);
    let vue = modele(&r, &Replies::neuf());
    let ids: Vec<&str> = vue.iter().map(|(id, _)| id.as_str()).collect();
    for inactif in ["browser.web_worker", "net.dhcp", "net.tls", "net.http2"] {
        assert!(ids.contains(&inactif), "« {inactif} » a disparu parce qu'il se tait");
    }
    assert_eq!(r.lis("net.http2").unwrap().etat, Etat::Inconnu);
}

// ===========================================================================
// Le meme registre pour les trois consommateurs
// ===========================================================================

#[test]
fn un_identifiant_de_la_blackbox_se_retrouve_dans_la_vue() {
    // « Je veux prendre service_id=net.dns dans la blackbox et retrouver
    // net.dns dans l'application Services. Meme identifiant, meme etat, meme
    // compteur d'erreurs. »
    let mut r = registre_complet();
    r.etat("net.dns", Etat::Actif, 1_000);
    r.erreur("net.dns", "timeout", 2_000);

    let vue = modele(&r, &Replies::neuf());
    assert!(vue.iter().any(|(id, _)| id == "net.dns"));

    let depuis_registre = r.lis("net.dns").unwrap();
    assert_eq!(depuis_registre.etat, Etat::Degrade);
    assert_eq!(depuis_registre.erreurs, 1);
    assert_eq!(depuis_registre.raison.texte(), "timeout");
}

// ===========================================================================
// Le repliage du premier affichage
//
// La premiere capture QEMU montrait « sys » et « net » -- et rien d'autre :
// `net` deployait vingt-quatre lignes et poussait `browser` sous le bord de
// la fenetre. Une fenetre censee montrer toute la pile en cachait un tiers.
// ===========================================================================

/// Lignes tenant dans le corps de la fenetre : (560 - 54 barre - 20 en-tete
/// - 22 pied - 24 decor) / 18. Mesure sur la fenetre reelle, pas devinee.
const LIGNES_A_L_ECRAN: usize = 24;

fn vue_par_defaut() -> Vec<(String, u8)> {
    let r = registre_complet();
    let mut replies = Replies::neuf();
    repli_par_defaut(r.entrees(), &mut replies, LIGNES_A_L_ECRAN);
    modele(&r, &replies)
}

#[test]
fn le_premier_affichage_montre_les_trois_racines_sans_defiler() {
    let vue = vue_par_defaut();
    let ids: Vec<&str> = vue.iter().map(|(id, _)| id.as_str()).collect();
    for racine in RACINES {
        let rang = ids
            .iter()
            .position(|id| *id == racine)
            .unwrap_or_else(|| panic!("« {racine} » absent du premier affichage"));
        assert!(
            rang < LIGNES_A_L_ECRAN,
            "« {racine} » est a la ligne {rang} : hors de l'ecran sans defiler, \
             exactement le defaut de la premiere capture"
        );
    }
}

#[test]
fn le_premier_affichage_tient_dans_la_fenetre() {
    let vue = vue_par_defaut();
    assert!(
        vue.len() <= LIGNES_A_L_ECRAN,
        "le premier affichage compte {} lignes pour {LIGNES_A_L_ECRAN} visibles",
        vue.len()
    );
}

#[test]
fn le_reseau_et_le_navigateur_sont_ouverts_par_defaut() {
    // « Reseau et Navigateur ouverts par defaut. » Ouverts veut dire : leurs
    // enfants directs sont visibles.
    let vue = vue_par_defaut();
    let ids: Vec<&str> = vue.iter().map(|(id, _)| id.as_str()).collect();
    for enfant in ["net.nic", "net.l2", "net.l3", "net.l4", "net.security"] {
        assert!(ids.contains(&enfant), "« {enfant} » cache alors que net est ouvert");
    }
    for enfant in ["browser.host", "browser.request_server", "browser.navigation"] {
        assert!(ids.contains(&enfant), "« {enfant} » cache alors que browser est ouvert");
    }
}

#[test]
fn le_premier_affichage_montre_le_plus_de_detail_qui_tienne() {
    // Le budget sert a quelque chose : plus haute, la fenetre s'ouvre plus.
    let r = registre_complet();
    let mut etroit = Replies::neuf();
    repli_par_defaut(r.entrees(), &mut etroit, LIGNES_A_L_ECRAN);
    let mut large = Replies::neuf();
    repli_par_defaut(r.entrees(), &mut large, 200);
    assert!(
        modele(&r, &large).len() > modele(&r, &etroit).len(),
        "une fenetre plus haute doit montrer plus de lignes"
    );
}

#[test]
fn les_racines_restent_ouvertes_meme_sous_un_budget_impossible() {
    // Le plancher est assume : les trois racines et leurs enfants directs
    // s'affichent toujours. Une fenetre trop courte fait DEFILER -- elle ne
    // fait pas disparaitre le reseau.
    let r = registre_complet();
    let mut minuscule = Replies::neuf();
    repli_par_defaut(r.entrees(), &mut minuscule, 1);
    let ids: Vec<String> = modele(&r, &minuscule).into_iter().map(|(id, _)| id).collect();
    for racine in RACINES {
        assert!(ids.iter().any(|id| id == racine));
    }
    for enfant in ["sys.scheduler", "net.nic", "browser.host"] {
        assert!(ids.iter().any(|id| id == enfant), "« {enfant} » doit rester visible");
    }
    // Et rien de plus fin : le budget interdit le niveau suivant.
    assert!(!ids.iter().any(|id| id == "net.nic.rtl8168"));
}

#[test]
fn un_groupe_replie_par_defaut_s_ouvre_au_clic() {
    // Le repliage est un DEFAUT, pas une amputation : tout se rouvre.
    let r = registre_complet();
    let mut replies = Replies::neuf();
    repli_par_defaut(r.entrees(), &mut replies, LIGNES_A_L_ECRAN);
    assert!(!replies.deploye("net.l4"));

    replies.bascule("net.l4");
    let ids: Vec<String> = modele(&r, &replies).into_iter().map(|(id, _)| id).collect();
    assert!(ids.iter().any(|id| id == "net.tcp"));
    assert!(ids.iter().any(|id| id == "net.udp"));
}

#[test]
fn le_repliage_par_defaut_est_idempotent() {
    // Il est pose a la premiere peinture ; une seconde pose ne doit pas
    // saturer la table des replis.
    let r = registre_complet();
    let mut une = Replies::neuf();
    repli_par_defaut(r.entrees(), &mut une, LIGNES_A_L_ECRAN);
    let apres_une = modele(&r, &une).len();
    repli_par_defaut(r.entrees(), &mut une, LIGNES_A_L_ECRAN);
    assert_eq!(modele(&r, &une).len(), apres_une);
}

#[test]
fn les_deux_cartes_reseau_sont_declarees() {
    // Sous QEMU la carte est une e1000, sur la TRIGKEY un RTL8168. La
    // topologie porte les deux : celle qui n'est pas la reste « N/A » au lieu
    // d'afficher les compteurs, toujours nuls, de sa voisine absente.
    let r = registre_complet();
    for carte in ["net.nic.rtl8168", "net.nic.e1000"] {
        let entree = r.lis(carte).unwrap_or_else(|| panic!("« {carte} » non declaree"));
        assert_eq!(entree.genre, Genre::Pilote);
        assert!(entree.parent.egale("net.nic"));
        assert_eq!(entree.etat, Etat::Inconnu, "une carte non vue reste N/A");
    }
}

// ===========================================================================
// L'etat qu'un groupe reporte
//
// La seconde capture QEMU montrait sept entetes de groupe -- nic, l2, config,
// l3, l4, security, application -- et pas une seule mesure. Il fallait ouvrir
// les sept pour savoir lequel allait mal.
// ===========================================================================

#[test]
fn un_groupe_reporte_le_pire_de_ses_descendants() {
    let mut r = registre_complet();
    r.etat("net.ethernet", Etat::Actif, 1_000);
    r.etat("net.arp", Etat::Degrade, 1_000);
    assert_eq!(etat_agrege(r.entrees(), "net.l2"), Etat::Degrade);
    // Et cela remonte : `net` porte ce que `net.l2` porte.
    assert_eq!(etat_agrege(r.entrees(), "net"), Etat::Degrade);
}

#[test]
fn un_arret_ne_masque_pas_une_degradation() {
    // L'ordre de declaration de `Etat` met Arrete (7) au-dessus de Degrade
    // (5). Le groupe doit signaler la degradation, pas l'arret.
    let mut r = registre_complet();
    r.etat("net.ethernet", Etat::Arrete, 1_000);
    r.etat("net.arp", Etat::Degrade, 1_000);
    assert!(gravite(Etat::Degrade) > gravite(Etat::Arrete));
    assert_eq!(etat_agrege(r.entrees(), "net.l2"), Etat::Degrade);
}

#[test]
fn un_groupe_sans_mesure_reste_n_a() {
    // Rien n'a ete publie : le groupe n'invente pas un « Actif ».
    let r = registre_complet();
    assert_eq!(etat_agrege(r.entrees(), "net.l2"), Etat::Inconnu);
    assert_eq!(etat_affiche(etat_agrege(r.entrees(), "net.l2")), "N/A");
}

#[test]
fn la_ligne_d_un_groupe_porte_l_etat_agrege() {
    // Le modele visible le calcule : le peintre n'a pas a le refaire.
    let mut r = registre_complet();
    r.etat("net.tcp", Etat::Panne, 1_000);
    let mut replies = Replies::neuf();
    repli_par_defaut(r.entrees(), &mut replies, LIGNES_A_L_ECRAN);
    let mut tampon = vec![Ligne::vide(); SERVICES_MAX];
    let n = lignes(r.entrees(), &replies, &mut tampon);
    let l4 = tampon[..n]
        .iter()
        .find(|l| l.entree.id.egale("net.l4"))
        .expect("net.l4 doit etre visible");
    assert_eq!(l4.etat_effectif, Etat::Panne);
    assert!(!l4.deploye, "il est replie, et parle quand meme");
}

#[test]
fn la_ligne_d_un_service_porte_son_propre_etat() {
    let mut r = registre_complet();
    r.etat("net.tcp", Etat::Actif, 1_000);
    let mut replies = Replies::neuf();
    replies.deploie("net.l4");
    let mut tampon = vec![Ligne::vide(); SERVICES_MAX];
    let n = lignes(r.entrees(), &replies, &mut tampon);
    let tcp = tampon[..n].iter().find(|l| l.entree.id.egale("net.tcp")).unwrap();
    assert_eq!(tcp.etat_effectif, Etat::Actif);
}
