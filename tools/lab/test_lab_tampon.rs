//! Le tampon de sortie borne dit-il la verite sur ce qu'il contient ?
//!
//! # Ce que ces epreuves defendent
//!
//! Un tampon qui tronque en silence produit du JSON invalide -- une accolade
//! manquante -- et le client rejette la ligne entiere sans savoir pourquoi.
//! Tout l'interet de ce type est que `tronque()` soit EXACT : vrai des qu'un
//! octet a ete perdu, faux sinon, et jamais blanchi par accident.
//!
//! Le chemin le plus delicat est `tronque_a`, qui sert a defaire une ecriture
//! qui ne tenait pas. Une premiere redaction effacait le drapeau dans tous les
//! cas, y compris quand elle refusait le rollback : un appelant qui se
//! trompait de borne obtenait alors un tampon declare intact alors qu'il avait
//! perdu des octets. C'est exactement le mensonge que ce type existe pour
//! empecher, et trois des epreuves ci-dessous ne parlent que de cela.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

use core::fmt::Write;

#[path = "../../src/net/diag_distant/tampon.rs"]
mod tampon;

use tampon::Tampon;

/// Une taille assez petite pour que la borne soit atteinte en une ligne.
const N: usize = 32;

fn neuf() -> Tampon<N> {
    Tampon::<N>::neuf()
}

// ===========================================================================
// L'ECRITURE ORDINAIRE
// ===========================================================================

#[test]
fn une_ecriture_qui_tient_ne_se_declare_pas_tronquee() {
    let mut t = neuf();
    write!(t, "bonjour").unwrap();
    assert_eq!(t.octets(), b"bonjour");
    assert_eq!(t.len(), 7);
    assert!(!t.tronque(), "rien n'a ete perdu, le drapeau doit rester bas");
    assert!(!t.is_empty());
}

#[test]
fn un_tampon_neuf_est_vide_et_intact() {
    let t = neuf();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
    assert!(!t.tronque());
    assert_eq!(t.octets(), b"");
}

#[test]
fn plusieurs_ecritures_se_concatenent_dans_l_ordre() {
    let mut t = neuf();
    write!(t, "a").unwrap();
    write!(t, "bc").unwrap();
    write!(t, "{}", 42).unwrap();
    assert_eq!(t.octets(), b"abc42");
    assert!(!t.tronque());
}

#[test]
fn vide_efface_le_contenu_et_le_drapeau() {
    let mut t = neuf();
    write!(t, "{}", "x".repeat(N + 10)).unwrap();
    assert!(t.tronque());
    t.vide();
    assert!(t.is_empty());
    assert!(!t.tronque(), "un tampon vide n'a rien perdu");
}

// ===========================================================================
// LA CAPACITE EXACTE : LA FRONTIERE, DES DEUX COTES
// ===========================================================================

#[test]
fn exactement_la_capacite_tient_et_ne_tronque_pas() {
    // LA BORNE EST INCLUSIVE. Un tampon qui se declarerait tronque a `N`
    // octets pile ferait rejeter des reponses parfaitement entieres.
    let mut t = neuf();
    let plein = "y".repeat(N);
    write!(t, "{plein}").unwrap();
    assert_eq!(t.len(), N);
    assert_eq!(t.octets(), plein.as_bytes());
    assert!(!t.tronque(), "N octets dans un tampon de N : rien n'est perdu");
}

#[test]
fn un_octet_de_trop_tronque_et_le_dit() {
    let mut t = neuf();
    let trop = "z".repeat(N + 1);
    write!(t, "{trop}").unwrap();
    assert_eq!(t.len(), N, "le tampon ne depasse jamais sa capacite");
    assert!(t.tronque());
    // CE QUI A TENU EST GARDE. Rendre un tampon vide sur debordement ferait
    // perdre le debut de la reponse, qui est souvent la partie utile.
    assert_eq!(t.octets(), "z".repeat(N).as_bytes());
}

#[test]
fn le_debordement_garde_le_prefixe_de_la_premiere_ecriture() {
    let mut t = neuf();
    write!(t, "{}", "a".repeat(N - 2)).unwrap();
    assert!(!t.tronque());
    write!(t, "bcdef").unwrap();
    assert!(t.tronque());
    assert_eq!(t.len(), N);
    let mut attendu = "a".repeat(N - 2);
    attendu.push_str("bc");
    assert_eq!(t.octets(), attendu.as_bytes());
}

#[test]
fn le_drapeau_ne_se_leve_pas_tout_seul_apres_une_ecriture_courte() {
    let mut t = neuf();
    for _ in 0..N {
        write!(t, "q").unwrap();
        assert!(!t.tronque());
    }
    assert_eq!(t.len(), N);
    write!(t, "q").unwrap();
    assert!(t.tronque(), "le N+1-eme octet est celui qui deborde");
}

// ===========================================================================
// `termine()` : LE TERMINATEUR FAIT PARTIE DE LA LIGNE
// ===========================================================================

#[test]
fn termine_ajoute_le_saut_de_ligne() {
    let mut t = neuf();
    write!(t, "ok").unwrap();
    t.termine();
    assert_eq!(t.octets(), b"ok\n");
    assert!(!t.tronque());
}

#[test]
fn termine_sur_un_tampon_plein_tronque_et_le_dit() {
    // LE TERMINATEUR EST UN OCTET COMME UN AUTRE. Une ligne sans `\n` n'est
    // pas une ligne : le protocole la laisserait en attente indefiniment, donc
    // ne pas pouvoir l'ecrire est bel et bien une troncature.
    let mut t = neuf();
    write!(t, "{}", "p".repeat(N)).unwrap();
    assert_eq!(t.len(), N);
    assert!(!t.tronque());
    t.termine();
    assert!(t.tronque(), "le terminateur n'a pas tenu : c'est une troncature");
    assert_eq!(t.len(), N, "et rien n'a ete ecrit au-dela de la capacite");
}

#[test]
fn termine_a_la_derniere_place_tient_juste() {
    let mut t = neuf();
    write!(t, "{}", "p".repeat(N - 1)).unwrap();
    t.termine();
    assert_eq!(t.len(), N);
    assert!(!t.tronque());
    assert_eq!(t.octets()[N - 1], b'\n');
}

// ===========================================================================
// `tronque_a` : LE CONTRAT ETROIT, ET POURQUOI IL L'EST
// ===========================================================================

#[test]
fn tronque_a_une_longueur_inferieure_defait_et_repare_le_drapeau() {
    let mut t = neuf();
    write!(t, "gardez").unwrap();
    let marque = t.len();
    write!(t, "{}", "x".repeat(N)).unwrap();
    assert!(t.tronque());

    assert!(t.tronque_a(marque), "un retour en arriere reel est accepte");
    assert_eq!(t.octets(), b"gardez");
    // LE CONTENU JUSQU'A `marque` EST ENTIER PAR CONSTRUCTION : il a ete ecrit
    // avant que la borne soit atteinte. Le drapeau tombe donc a juste titre.
    assert!(!t.tronque());
}

#[test]
fn tronque_a_une_longueur_superieure_est_refuse_et_ne_blanchit_rien() {
    // L'EPREUVE QUI DEFEND LE TYPE. Une premiere redaction effacait le
    // drapeau meme ici : l'appelant obtenait un tampon declare intact alors
    // qu'il venait de perdre des octets.
    let mut t = neuf();
    write!(t, "{}", "w".repeat(N + 5)).unwrap();
    assert!(t.tronque());
    let avant = t.len();

    assert!(!t.tronque_a(N + 1), "on ne revient pas a une longueur jamais atteinte");
    assert_eq!(t.len(), avant, "un refus ne bouge rien");
    assert!(t.tronque(), "et surtout pas le drapeau");
}

#[test]
fn tronque_a_la_longueur_courante_ne_defait_rien_donc_ne_repare_rien() {
    // RIEN N'EST DEFAIT, DONC RIEN N'EST REPARE. Sans cette regle, un rollback
    // qui ne rollback rien blanchirait une troncature reelle.
    let mut t = neuf();
    write!(t, "{}", "v".repeat(N + 3)).unwrap();
    assert!(t.tronque());
    let n = t.len();

    assert!(t.tronque_a(n), "revenir a ou l'on est deja est accepte");
    assert_eq!(t.len(), n);
    assert!(t.tronque(), "le drapeau survit : la perte a bien eu lieu");
}

#[test]
fn tronque_a_zero_vide_le_tampon_et_le_declare_intact() {
    let mut t = neuf();
    write!(t, "{}", "u".repeat(N + 1)).unwrap();
    assert!(t.tronque());
    assert!(t.tronque_a(0));
    assert!(t.is_empty());
    assert!(!t.tronque());
}

#[test]
fn tronque_a_sur_un_tampon_intact_reste_intact() {
    let mut t = neuf();
    write!(t, "abcdef").unwrap();
    assert!(t.tronque_a(3));
    assert_eq!(t.octets(), b"abc");
    assert!(!t.tronque());
}

// ===========================================================================
// LA REECRITURE APRES ROLLBACK : LE CAS QUE `politique::ajoute` EMPRUNTE
// ===========================================================================

#[test]
fn on_peut_reecrire_apres_un_rollback_et_le_resultat_est_propre() {
    // C'EST LE CHEMIN DU TRANSPORT. `politique::ajoute` ecrit un evenement,
    // constate qu'il ne tient pas, defait, et rend la main a l'appelant qui
    // doit pouvoir ecrire AUTRE CHOSE dans le meme tampon.
    let mut t = neuf();
    write!(t, "garde:").unwrap();
    let marque = t.len();

    write!(t, "{}", "0".repeat(N)).unwrap();
    assert!(t.tronque());
    assert!(t.tronque_a(marque));
    assert!(!t.tronque());

    write!(t, "court").unwrap();
    assert_eq!(t.octets(), b"garde:court");
    assert!(!t.tronque(), "la reecriture tient : le tampon est intact");
    t.termine();
    assert_eq!(t.octets(), b"garde:court\n");
    assert!(!t.tronque());
}

#[test]
fn deux_rollbacks_successifs_ramenent_bien_au_meme_point() {
    let mut t = neuf();
    write!(t, "base").unwrap();
    let marque = t.len();
    for _ in 0..2 {
        write!(t, "{}", "9".repeat(N)).unwrap();
        assert!(t.tronque());
        assert!(t.tronque_a(marque));
        assert_eq!(t.octets(), b"base");
        assert!(!t.tronque());
    }
}

#[test]
fn le_rollback_rend_exactement_l_etat_confie() {
    // La propriete dont `politique::ajoute` depend : apres un rollback, ce qui
    // SUIT peut ecrire. Si le drapeau restait pose, la premiere ecriture
    // suivante serait declaree tronquee sans l'etre.
    let mut t = neuf();
    write!(t, "abc").unwrap();
    let etat = t.octets().to_vec();
    let marque = t.len();

    write!(t, "{}", "-".repeat(N)).unwrap();
    assert!(t.tronque_a(marque));

    assert_eq!(t.octets(), etat.as_slice());
    assert_eq!(t.len(), marque);
    assert!(!t.tronque());
}
