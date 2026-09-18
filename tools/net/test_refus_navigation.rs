//! LA RECONNAISSANCE D'UNE NAVIGATION REFUSEE, A L'HOTE.
//!
//! # Le defaut, releve le 18 septembre 2026
//!
//! Le journal physique porte, cote navigateur :
//!
//! ```text
//! [ladybird-bouchaud] BROWSER_NETWORK_NOT_READY id=0
//! WebContent(14): Failed load of: "https://www.google.com/"
//! ```
//!
//! et la fenetre Services, au meme instant :
//!
//! ```text
//! browser.navigation   Repos   url : aucune navigation
//! ```
//!
//! Les deux disaient vrai du point de vue du noyau : le `RequestServer` avait
//! renonce AVANT le moindre `connect`, donc aucun appel systeme reseau n'avait
//! eu lieu. Ensemble, ils faisaient croire que personne n'avait rien demande
//! -- au moment precis ou l'utilisateur regardait une page qui ne charge pas.
//!
//! Une navigation refusee est une navigation. Ce banc verifie la seule partie
//! qui se teste sans machine : la reconnaissance du marqueur, et le fait
//! qu'elle ne se declenche sur rien d'autre.

#[path = "../../src/drivers/network/analyse_refus.rs"]
mod analyse_refus;

use analyse_refus::{refus_dans, Refus};

#[test]
fn le_marqueur_rend_l_url_et_le_motif() {
    let ligne = b"[ladybird-bouchaud] BOUCHAUD_NAV_BLOQUEE url=https://www.google.com/ raison=network-not-ready\n";
    let refus = refus_dans(ligne).expect("le marqueur doit etre reconnu");
    assert_eq!(refus.url, "https://www.google.com/");
    assert_eq!(refus.raison, "network-not-ready");
}

#[test]
fn une_ligne_ordinaire_du_navigateur_ne_declenche_rien() {
    // LE POINT IMPORTANT. Ce chemin voit TOUT ce que l'anneau 3 ecrit sur sa
    // sortie d'erreur. S'il se declenchait sur autre chose que notre propre
    // marqueur, n'importe quelle page pourrait fabriquer une fausse
    // navigation en ecrivant la bonne chaine dans une trace.
    for ligne in [
        &b"WebContent(14): ResourceLoader: Failed load of: \"https://www.google.com/\"\n"[..],
        &b"[ladybird-bouchaud] BROWSER_NETWORK_NOT_READY id=0\n"[..],
        &b"[ladybird-bouchaud] BROWSER_DNS_UPDATED server=10.0.2.3\n"[..],
        &b"rien du tout\n"[..],
        &b""[..],
    ] {
        assert!(refus_dans(ligne).is_none(), "declenche sur : {:?}", ligne);
    }
}

#[test]
fn un_marqueur_tronque_ne_rend_rien() {
    // Une ecriture sur la console peut etre coupee en deux appels. Mieux vaut
    // manquer un refus que d'en inventer un avec une URL a moitie lue.
    assert!(refus_dans(b"BOUCHAUD_NAV_BLOQUEE url=").is_none());
    assert!(refus_dans(b"BOUCHAUD_NAV_BLOQ").is_none());
}

#[test]
fn le_motif_est_facultatif() {
    let refus = refus_dans(b"BOUCHAUD_NAV_BLOQUEE url=http://exemple.test/a\n").unwrap();
    assert_eq!(refus.url, "http://exemple.test/a");
    assert_eq!(refus.raison, "refus");
}

#[test]
fn l_url_s_arrete_au_premier_blanc() {
    let refus = refus_dans(b"x BOUCHAUD_NAV_BLOQUEE url=http://a/b raison=x autre=1\n").unwrap();
    assert_eq!(refus.url, "http://a/b");
    assert_eq!(refus.raison, "x");
}

#[test]
fn une_url_trop_longue_est_tronquee_et_non_refusee() {
    // Tronquer garde l'hote et le debut du chemin, ce qui suffit a savoir de
    // quelle page on parle. Refuser ferait disparaitre la navigation.
    let mut ligne = Vec::from(&b"BOUCHAUD_NAV_BLOQUEE url=http://exemple.test/"[..]);
    ligne.extend(core::iter::repeat(b'a').take(400));
    ligne.push(b'\n');
    let refus = refus_dans(&ligne).expect("une URL longue reste une URL");
    assert!(refus.url.starts_with("http://exemple.test/"));
}

#[test]
fn le_marqueur_est_reconnu_au_milieu_d_un_tampon() {
    // La console recoit des blocs, pas des lignes.
    let ligne = b"trace precedente\n[ladybird-bouchaud] BOUCHAUD_NAV_BLOQUEE url=https://a.test/ raison=network-not-ready\ntrace suivante\n";
    let refus = refus_dans(ligne).unwrap();
    assert_eq!(refus.url, "https://a.test/");
}

#[test]
fn un_refus_ne_porte_jamais_une_url_vide() {
    assert!(refus_dans(b"BOUCHAUD_NAV_BLOQUEE url= raison=x\n").is_none());
}

#[allow(dead_code)]
fn type_utilise(_: Refus) {}
