//! Le chemin X.509 du noyau, contre des chaines fabriquees ailleurs.
//!
//! # Ce que ces epreuves defendent
//!
//! `validate.rs` decide si le cadenas du navigateur natif, de `wget` et de
//! la preuve Internet dit « [TLS OK] ». Il le decide en remontant la chaine
//! envoyee par le SERVEUR -- c'est-a-dire par quiconque se trouve sur le
//! chemin reseau. Chaque regle du parcours est donc une frontiere de
//! confiance, et une regle oubliee ne fait echouer aucun site honnete : elle
//! ne se voit que face a une chaine construite pour la contourner.
//!
//! La premiere de ces regles est le drapeau CA (basicConstraints, RFC 5280
//! 4.2.1.9) : une feuille legitimement emise pour `attaquant.test` ne doit
//! pas pouvoir signer une fausse feuille `victime.test`. Sa signature est
//! mathematiquement parfaite ; c'est son DROIT de signer qui manque.
//!
//! # Ce qui est reel, ce qui ne l'est pas
//!
//! `x509.rs`, `validate.rs` et toute la cryptographie sont ceux du noyau,
//! inclus tels quels. Les chaines viennent d'OpenSSL
//! (`fabrique-chaines-x509.sh`), qui ne partage aucune ligne avec eux, et
//! OpenSSL lui-meme les juge comme ces epreuves l'attendent.
//!
//! Le MAGASIN, et lui seul, est factice : `roots.rs` embarque les vraies
//! racines publiques, dont personne n'a la cle privee -- aucune chaine
//! d'attaque ne peut s'y terminer. Le magasin de ce fichier applique les deux
//! memes regles que `roots.rs` (emetteur : sujet ET signature ; ancre
//! equivalente : sujet ET cle publique) sur une racine de test. Le vrai
//! `roots.rs` est inclus a part, pour verifier que chacune de ses racines
//! est lisible par le noyau.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

extern crate alloc;

// Les modules du noyau, freres a la racine de ce test : `x509.rs` ecrit
// `use super::asn1`, `validate.rs` ecrit `use super::roots`. On recree la
// fratrie plutot que de tordre la source pour lui plaire.
#[path = "../../src/net/security/tls/asn1.rs"]
#[allow(dead_code)]
mod asn1;
#[path = "../../src/net/security/tls/bignum.rs"]
#[allow(dead_code)]
mod bignum;
#[path = "../../src/net/security/tls/ec.rs"]
#[allow(dead_code)]
mod ec;
#[path = "../../src/net/security/tls/p256.rs"]
#[allow(dead_code)]
mod p256;
#[path = "../../src/net/security/tls/p384.rs"]
#[allow(dead_code)]
mod p384;
#[path = "../../src/net/security/tls/rsa.rs"]
#[allow(dead_code)]
mod rsa;
#[path = "../../src/net/security/tls/sha256.rs"]
#[allow(dead_code)]
mod sha256;
#[path = "../../src/net/security/tls/sha512.rs"]
#[allow(dead_code)]
mod sha512;
#[path = "../../src/net/security/tls/validate.rs"]
#[allow(dead_code)]
mod validate;
#[path = "../../src/net/security/tls/x509.rs"]
#[allow(dead_code)]
mod x509;

// Le vrai magasin, pour l'epreuve de lisibilite seulement.
#[path = "../../src/net/security/tls/roots.rs"]
#[allow(dead_code)]
mod roots_embarques;

// `validate::now_stamp` lit l'horloge CMOS. Aucune epreuve ne l'appelle --
// chacune passe son `now` -- mais le module doit compiler.
#[allow(dead_code)]
mod arch {
    pub mod x86_64 {
        pub mod rtc {
            pub struct DateTime {
                pub year: u16,
                pub month: u8,
                pub day: u8,
                pub hour: u8,
                pub minute: u8,
                pub second: u8,
            }
            pub fn now_utc() -> DateTime {
                DateTime { year: 2030, month: 1, day: 1, hour: 0, minute: 0, second: 0 }
            }
        }
    }
}

/// Le magasin factice : une seule racine, les regles de `roots.rs`.
#[allow(dead_code)]
mod roots {
    use super::x509::{self, Certificate, PubKey};

    static RACINES: &[&[u8]] = &[include_bytes!("chaines-x509/racine.der")];

    pub fn find_issuer_for(child: &Certificate) -> Option<Certificate> {
        RACINES.iter().filter_map(|der| x509::parse(der)).find(|root| {
            root.subject == child.issuer && x509::verify_signed_by(child, &root.pubkey)
        })
    }

    fn meme_cle_publique(a: &PubKey, b: &PubKey) -> bool {
        match (a, b) {
            (PubKey::Rsa { n: an, e: ae }, PubKey::Rsa { n: bn, e: be }) => an == bn && ae == be,
            (PubKey::EcP256 { point: ap }, PubKey::EcP256 { point: bp }) => ap == bp,
            (PubKey::EcP384 { point: ap }, PubKey::EcP384 { point: bp }) => ap == bp,
            _ => false,
        }
    }

    pub fn find_equivalent_anchor(cert: &Certificate) -> Option<Certificate> {
        RACINES.iter().filter_map(|der| x509::parse(der)).find(|root| {
            root.subject == cert.subject && meme_cle_publique(&root.pubkey, &cert.pubkey)
        })
    }
}

use x509::Certificate;

/// 1er janvier 2030 : les fixtures sont valides de leur fabrication
/// jusqu'en 2105. L'heure de la machine qui lance la suite n'intervient pas.
const MAINTENANT: u64 = 2030_01_01_000000;
/// Au-dela de toute fixture.
const APRES_EXPIRATION: u64 = 2200_01_01_000000;

const RACINE: &[u8] = include_bytes!("chaines-x509/racine.der");
const INTERMEDIAIRE: &[u8] = include_bytes!("chaines-x509/intermediaire.der");
const FEUILLE: &[u8] = include_bytes!("chaines-x509/feuille.der");
const FEUILLE_ATTAQUANT: &[u8] = include_bytes!("chaines-x509/feuille-attaquant.der");
const FEUILLE_FORGEE: &[u8] = include_bytes!("chaines-x509/feuille-forgee.der");
const INTERMEDIAIRE_SANS_BC: &[u8] = include_bytes!("chaines-x509/intermediaire-sans-bc.der");
const FEUILLE_SOUS_SANS_BC: &[u8] = include_bytes!("chaines-x509/feuille-sous-sans-bc.der");
const FEUILLE_DIRECTE: &[u8] = include_bytes!("chaines-x509/feuille-directe.der");
const RACINE_INCONNUE: &[u8] = include_bytes!("chaines-x509/racine-inconnue.der");
const FEUILLE_INCONNUE: &[u8] = include_bytes!("chaines-x509/feuille-inconnue.der");

fn lit(der: &[u8]) -> Certificate {
    x509::parse(der).expect("fixture illisible par x509.rs")
}

fn chaine(ders: &[&[u8]]) -> Vec<Certificate> {
    ders.iter().map(|d| lit(d)).collect()
}

// -- Les fixtures sont bien ce qu'elles pretendent etre --------------------
//
// Si le drapeau lu par le noyau n'est pas celui qu'OpenSSL a ecrit, toutes
// les epreuves suivantes mesurent autre chose que ce qu'elles annoncent.

#[test]
fn le_drapeau_ca_est_lu_comme_openssl_l_a_ecrit() {
    assert!(lit(RACINE).is_ca);
    assert!(lit(INTERMEDIAIRE).is_ca);
    assert!(!lit(FEUILLE).is_ca);
    assert!(!lit(FEUILLE_ATTAQUANT).is_ca);
    // Extension absente : cA vaut FALSE (RFC 5280 4.2.1.9).
    assert!(!lit(INTERMEDIAIRE_SANS_BC).is_ca);
}

#[test]
fn la_signature_forgee_est_mathematiquement_valide() {
    // Tout le sujet est la : la fausse feuille EST signee par la cle de la
    // feuille de l'attaquant. Seul le droit de signer peut la refuser.
    let forgee = lit(FEUILLE_FORGEE);
    let attaquant = lit(FEUILLE_ATTAQUANT);
    assert_eq!(forgee.issuer, attaquant.subject);
    assert!(x509::verify_signed_by(&forgee, &attaquant.pubkey));
}

// -- Ce qui doit passer -----------------------------------------------------

#[test]
fn la_chaine_honnete_est_acceptee() {
    let r = validate::validate(&chaine(&[FEUILLE, INTERMEDIAIRE]), "victime.test", MAINTENANT);
    assert!(r.trusted, "detail: {}", r.detail);
    assert!(r.hostname_ok);
    assert!(!r.expired);
    assert_eq!(r.anchor, Some("magasin:issuer"));
}

#[test]
fn une_feuille_signee_par_la_racine_est_acceptee() {
    let r = validate::validate(&chaine(&[FEUILLE_DIRECTE]), "direct.test", MAINTENANT);
    assert!(r.trusted, "detail: {}", r.detail);
    assert!(r.hostname_ok);
}

#[test]
fn la_racine_envoyee_en_trop_ne_gene_pas() {
    // Beaucoup de serveurs envoient aussi la racine. Le chemin s'arrete a
    // l'ancre locale ; ce qui suit ne compte pas.
    let r = validate::validate(
        &chaine(&[FEUILLE, INTERMEDIAIRE, RACINE]),
        "victime.test",
        MAINTENANT,
    );
    assert!(r.trusted, "detail: {}", r.detail);
}

#[test]
fn l_expiration_est_dite_sans_casser_la_chaine() {
    let r = validate::validate(
        &chaine(&[FEUILLE, INTERMEDIAIRE]),
        "victime.test",
        APRES_EXPIRATION,
    );
    assert!(r.trusted);
    assert!(r.expired);
}

// -- Ce qui doit etre refuse ------------------------------------------------

#[test]
fn une_feuille_ne_peut_pas_signer_une_autre_feuille() {
    // L'ATTAQUE. L'intermediaire est envoye pour que la chaine remonte
    // jusqu'a l'ancre : sans basicConstraints, chaque maillon verifie, le
    // nom d'hote correspond, et le cadenas dit « [TLS OK] ».
    let r = validate::validate(
        &chaine(&[FEUILLE_FORGEE, FEUILLE_ATTAQUANT, INTERMEDIAIRE]),
        "victime.test",
        MAINTENANT,
    );
    assert!(
        !r.trusted,
        "une feuille CA:FALSE a signe un maillon et la chaine est jugee de confiance \
         (detail: {})",
        r.detail
    );
    // Refusee pour la BONNE raison : la signature, elle, est valide (voir
    // plus haut). Un refus pour une autre cause cacherait la perte du
    // controle le jour ou cette autre cause disparaitrait.
    assert_eq!(r.detail, "signataire sans basicConstraints CA dans la chaine");
}

#[test]
fn un_intermediaire_sans_basic_constraints_ne_signe_rien() {
    let r = validate::validate(
        &chaine(&[FEUILLE_SOUS_SANS_BC, INTERMEDIAIRE_SANS_BC]),
        "victime.test",
        MAINTENANT,
    );
    assert!(!r.trusted, "detail: {}", r.detail);
    assert_eq!(r.detail, "signataire sans basicConstraints CA dans la chaine");
}

#[test]
fn une_racine_inconnue_n_est_pas_une_ancre() {
    let r = validate::validate(
        &chaine(&[FEUILLE_INCONNUE, RACINE_INCONNUE]),
        "victime.test",
        MAINTENANT,
    );
    assert!(!r.trusted);
    assert_eq!(r.detail, "ancre de confiance inconnue (racine absente du magasin)");
}

#[test]
fn un_maillon_qui_ne_designe_pas_son_emetteur_arrete_le_chemin() {
    // `feuille` est emise par l'intermediaire, pas par l'attaquant.
    let r = validate::validate(
        &chaine(&[FEUILLE, FEUILLE_ATTAQUANT, INTERMEDIAIRE]),
        "victime.test",
        MAINTENANT,
    );
    assert!(!r.trusted);
    assert_eq!(r.detail, "issuer/subject incoherents dans la chaine");
}

#[test]
fn une_signature_alteree_est_refusee() {
    let mut feuille = lit(FEUILLE);
    let dernier = feuille.signature.len() - 1;
    feuille.signature[dernier] ^= 0x01;
    let r = validate::validate(&[feuille, lit(INTERMEDIAIRE)], "victime.test", MAINTENANT);
    assert!(!r.trusted);
    assert_eq!(r.detail, "signature de chaine invalide");
}

#[test]
fn un_nom_d_hote_non_couvert_est_dit() {
    // Chaine honnete, mauvais nom : `trusted` reste vrai, c'est
    // `hostname_ok` qui porte le refus -- le cadenas exige les deux.
    let r = validate::validate(
        &chaine(&[FEUILLE_ATTAQUANT, INTERMEDIAIRE]),
        "victime.test",
        MAINTENANT,
    );
    assert!(r.trusted);
    assert!(!r.hostname_ok);
}

#[test]
fn une_chaine_vide_n_est_pas_de_confiance() {
    let r = validate::validate(&[], "victime.test", MAINTENANT);
    assert!(!r.trusted);
    assert_eq!(r.detail, "aucun certificat");
}

// -- Le vrai magasin ----------------------------------------------------------

#[test]
fn chaque_racine_embarquee_est_lisible_et_utilisable() {
    // Une racine que `x509.rs` ne sait pas lire, ou dont la cle lui est
    // inconnue, ne sert d'ancre a personne -- et rien ne le signale : les
    // sites qu'elle couvre tombent simplement en « ancre inconnue ».
    assert_eq!(roots_embarques::count(), roots_embarques::ROOTS_DER.len());
    for (i, der) in roots_embarques::ROOTS_DER.iter().enumerate() {
        let racine = x509::parse(der)
            .unwrap_or_else(|| panic!("racine {} illisible par x509.rs", i));
        assert!(
            !matches!(racine.pubkey, x509::PubKey::Unknown),
            "racine {} : type de cle inconnu du noyau",
            i
        );
        assert!(racine.is_ca, "racine {} : basicConstraints CA absent", i);
        assert_eq!(racine.subject, racine.issuer, "racine {} : pas auto-emise", i);
    }
}
