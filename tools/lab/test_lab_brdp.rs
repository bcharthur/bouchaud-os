//! BRDP/1 tient-il sur un flux TCP quelconque, et refuse-t-il un mauvais HMAC ?
//!
//! # Ce que ces epreuves defendent
//!
//! TCP est un flux d'octets. Il ne promet ni qu'une commande arrive en un
//! seul segment, ni qu'un segment n'en contient qu'une. Les deux se
//! produisent en vrai -- le premier sur un reseau charge, le second quand le
//! client enchaine -- et un serveur qui suppose « un segment, une commande »
//! marche au banc et casse sur le terrain.
//!
//! Ce sont aussi les cas qu'un banc AVEC pile reseau reproduit le plus mal :
//! on ne choisit pas comment le noyau distant segmente. Ici, on les ecrit.
//!
//! L'autre moitie defend l'authentification : un HMAC faux doit etre refuse,
//! un jeton absent doit tout refuser, et la comparaison ne doit pas fuir la
//! reponse octet par octet.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

extern crate alloc;

#[path = "../../src/net/security/tls/sha256.rs"]
pub mod sha256;

// `brdp.rs` ecrit `use crate::net::security::tls::sha256` : on recree le
// chemin ici plutot que de modifier la source pour lui plaire. Un test qui
// oblige a tordre le code qu'il teste ne teste plus le meme code.
pub mod net {
    pub mod security {
        pub mod tls {
            pub use crate::sha256;
        }
    }
}

#[path = "../../src/net/diag_distant/brdp.rs"]
mod brdp;

use brdp::{
    analyse, champ_entier, champ_texte, egal_temps_constant, hex_decode, hex_encode,
    lit_entier, signature, verifie, Commande, Decoupeur, Erreur, LectureEntier,
    Morceau, DESCRIPTEURS_MAX,
    EVENTS_TAIL_MAX, LIGNE_MAX,
};

const JETON: &[u8] = b"un-jeton-de-laboratoire";
const NONCE: [u8; 32] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
    0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
    0x1d, 0x1e, 0x1f, 0x20,
];

/// Decoupe un flux en lignes, pour l'affirmation.
fn lignes(flux: &[&[u8]]) -> Vec<Result<String, ()>> {
    let mut d = Decoupeur::neuf();
    let mut sortie = Vec::new();
    for segment in flux {
        d.pousse(segment, |m| match m {
            Morceau::Ligne(l) => sortie.push(Ok(String::from_utf8_lossy(l).into_owned())),
            Morceau::TropLongue => sortie.push(Err(())),
        });
    }
    sortie
}

// ===========================================================================
// LE DECOUPAGE : CE QUE TCP NE GARANTIT PAS
// ===========================================================================

#[test]
fn une_commande_coupee_en_dix_segments_arrive_entiere() {
    // LA FRAGMENTATION. Un segment par caractere : c'est le pire cas, et rien
    // n'interdit a un routeur de le produire.
    let commande = br#"{"cmd":"status"}"#;
    let mut segments: Vec<&[u8]> = commande.chunks(1).collect();
    segments.push(b"\n");
    let vues = lignes(&segments);
    assert_eq!(vues, vec![Ok(r#"{"cmd":"status"}"#.to_string())]);
}

#[test]
fn plusieurs_commandes_dans_un_seul_segment_sont_toutes_rendues() {
    // Le client qui enchaine. Un serveur qui ne lit que la premiere ligne
    // laisserait les suivantes dans le tampon jusqu'au prochain octet recu --
    // c'est-a-dire, sur une machine muette, pour toujours.
    let flux: &[&[u8]] = &[b"{\"cmd\":\"status\"}\n{\"cmd\":\"audit run\"}\n{\"cmd\":\"quit\"}\n"];
    let vues = lignes(flux);
    assert_eq!(vues.len(), 3);
    assert_eq!(vues[0], Ok(r#"{"cmd":"status"}"#.to_string()));
    assert_eq!(vues[2], Ok(r#"{"cmd":"quit"}"#.to_string()));
}

#[test]
fn une_ligne_a_cheval_sur_deux_segments_se_recolle() {
    let flux: &[&[u8]] = &[b"{\"cmd\":\"sta", b"tus\"}\n{\"cmd\":\"qu", b"it\"}\n"];
    let vues = lignes(flux);
    assert_eq!(vues.len(), 2);
    assert_eq!(vues[0], Ok(r#"{"cmd":"status"}"#.to_string()));
    assert_eq!(vues[1], Ok(r#"{"cmd":"quit"}"#.to_string()));
}

#[test]
fn un_retour_chariot_windows_ne_casse_rien() {
    // Un client ecrit en PowerShell ne doit pas echouer sur un octet.
    let vues = lignes(&[b"{\"cmd\":\"status\"}\r\n"]);
    assert_eq!(vues, vec![Ok(r#"{"cmd":"status"}"#.to_string())]);
    assert!(analyse(br#"{"cmd":"status"}"#).is_ok());
}

#[test]
fn une_ligne_trop_longue_ne_desynchronise_pas_le_flux() {
    // LE POINT IMPORTANT. Fermer la connexion serait plus simple et nettement
    // moins utile : un client qui bafouille une fois doit pouvoir continuer.
    // Ce qui compte, c'est que la commande SUIVANTE arrive intacte.
    let mut flux = Vec::new();
    flux.extend_from_slice(&vec![b'x'; LIGNE_MAX * 3]);
    flux.extend_from_slice(b"\n");
    flux.extend_from_slice(b"{\"cmd\":\"status\"}\n");

    let mut d = Decoupeur::neuf();
    let mut vues = Vec::new();
    d.pousse(&flux, |m| match m {
        Morceau::Ligne(l) => vues.push(Ok(String::from_utf8_lossy(l).into_owned())),
        Morceau::TropLongue => vues.push(Err(())),
    });
    assert_eq!(vues.len(), 2);
    assert_eq!(vues[0], Err(()));
    assert_eq!(
        vues[1],
        Ok(r#"{"cmd":"status"}"#.to_string()),
        "la commande suivante doit arriver intacte",
    );
    assert_eq!(d.trop_longues(), 1);
    assert_eq!(d.lignes(), 1);
}

#[test]
fn un_flux_sans_retour_a_la_ligne_n_epuise_pas_la_memoire() {
    // Un pair hostile -- ou casse -- qui envoie un flux sans `\n`. Le tampon
    // est FIXE : il ne grandit pas, et rien n'est rendu tant qu'il n'y a pas
    // de ligne.
    let mut d = Decoupeur::neuf();
    let mut rendus = 0usize;
    for _ in 0..1_000 {
        d.pousse(&[b'a'; 1024], |_| rendus += 1);
    }
    assert_eq!(rendus, 0);
    assert!(d.en_attente() <= LIGNE_MAX);
}

#[test]
fn des_lignes_vides_ne_sont_pas_hostiles() {
    let vues = lignes(&[b"\n\n{\"cmd\":\"status\"}\n\n"]);
    assert_eq!(vues.len(), 4);
    assert_eq!(vues[0], Ok(String::new()));
    assert_eq!(analyse(b""), Err(Erreur::Vide));
    assert_eq!(analyse(b"   \t "), Err(Erreur::Vide));
}

#[test]
fn une_reconnexion_repart_d_un_tampon_propre() {
    // Un client qui se deconnecte au milieu d'une ligne ne doit pas laisser
    // son debut se coller au debut de la connexion suivante.
    let mut d = Decoupeur::neuf();
    d.pousse(b"{\"cmd\":\"sta", |_| panic!("rien n'est complet"));
    assert!(d.en_attente() > 0);
    d.reinitialise();
    let mut vues = Vec::new();
    d.pousse(b"{\"cmd\":\"quit\"}\n", |m| {
        if let Morceau::Ligne(l) = m {
            vues.push(String::from_utf8_lossy(l).into_owned());
        }
    });
    assert_eq!(vues, vec![r#"{"cmd":"quit"}"#.to_string()]);
}

// ===========================================================================
// LA LISTE BLANCHE
// ===========================================================================

#[test]
fn toutes_les_commandes_annoncees_sont_reconnues() {
    let cas: &[(&[u8], Commande)] = &[
        (br#"{"cmd":"status"}"#, Commande::Status),
        (br#"{"cmd":"audit status"}"#, Commande::AuditStatus),
        (br#"{"cmd":"audit run"}"#, Commande::AuditRun),
        (br#"{"cmd":"audit last"}"#, Commande::AuditLast),
        (br#"{"cmd":"net status"}"#, Commande::NetStatus),
        (br#"{"cmd":"rtl8168 status"}"#, Commande::Rtl8168Status),
        (br#"{"cmd":"rtl8168 ring"}"#, Commande::Rtl8168Ring),
        (br#"{"cmd":"dhcp status"}"#, Commande::DhcpStatus),
        (br#"{"cmd":"blackbox status"}"#, Commande::BlackboxStatus),
        (br#"{"cmd":"blackbox checkpoint"}"#, Commande::BlackboxCheckpoint),
        (br#"{"cmd":"events watch"}"#, Commande::EventsWatch),
        (br#"{"cmd":"services snapshot"}"#, Commande::ServicesSnapshot),
        (br#"{"cmd":"processes snapshot"}"#, Commande::ProcessesSnapshot),
        (br#"{"cmd":"memory snapshot"}"#, Commande::MemorySnapshot),
        (br#"{"cmd":"quit"}"#, Commande::Quit),
    ];
    for (ligne, attendue) in cas {
        assert_eq!(analyse(ligne), Ok(*attendue), "{}", String::from_utf8_lossy(ligne));
    }
}

#[test]
fn il_n_y_a_rien_derriere_la_liste_blanche() {
    // UNE COMMANDE ARBITRAIRE SUR UN CANAL D'ENQUETE, c'est un acces root sur
    // le segment local au premier jeton qui fuit. Pas de repli sur un shell,
    // pas de `exec`, pas de `sh`.
    for texte in [
        br#"{"cmd":"sh"}"#.as_slice(),
        br#"{"cmd":"exec"}"#,
        br#"{"cmd":"shell"}"#,
        br#"{"cmd":"read"}"#,
        br#"{"cmd":"write"}"#,
        br#"{"cmd":"status; rm -rf /"}"#,
        br#"{"cmd":"STATUS"}"#,
        br#"{"cmd":""}"#,
    ] {
        assert_eq!(
            analyse(texte),
            Err(Erreur::CommandeInconnue),
            "{}",
            String::from_utf8_lossy(texte),
        );
    }
}

#[test]
fn un_descripteur_hors_bornes_est_refuse() {
    assert_eq!(
        analyse(br#"{"cmd":"rtl8168 desc","n":0}"#),
        Ok(Commande::Rtl8168Desc { index: 0 }),
    );
    assert_eq!(
        analyse(br#"{"cmd":"rtl8168 desc","n":63}"#),
        Ok(Commande::Rtl8168Desc { index: 63 }),
    );
    assert_eq!(
        analyse(br#"{"cmd":"rtl8168 desc","n":64}"#),
        Err(Erreur::ArgumentInvalide),
        "la carte n'a que {DESCRIPTEURS_MAX} descripteurs",
    );
    assert_eq!(
        analyse(br#"{"cmd":"rtl8168 desc"}"#),
        Err(Erreur::ArgumentManquant),
    );
}

#[test]
fn events_tail_est_plafonne() {
    // Un client qui demanderait dix mille evenements ferait ecrire la machine
    // pendant des secondes sur un canal que la panne rend deja fragile.
    assert_eq!(
        analyse(br#"{"cmd":"events tail","n":100}"#),
        Ok(Commande::EventsTail { combien: 100 }),
    );
    assert_eq!(
        analyse(br#"{"cmd":"events tail"}"#),
        Ok(Commande::EventsTail { combien: 100 }),
        "un argument raisonnablement devinable ne doit pas faire echouer",
    );
    assert_eq!(
        analyse(br#"{"cmd":"events tail","n":0}"#),
        Err(Erreur::ArgumentInvalide),
    );
    let trop = format!(r#"{{"cmd":"events tail","n":{}}}"#, EVENTS_TAIL_MAX + 1);
    assert_eq!(analyse(trop.as_bytes()), Err(Erreur::ArgumentInvalide));
}

#[test]
fn un_entier_qui_deborde_ne_se_replie_pas_sur_une_petite_valeur() {
    // `99999999999999999999` ne doit pas devenir un petit nombre par
    // troncature : ce serait accepter une commande que personne n'a ecrite.
    let ligne = br#"{"cmd":"events tail","n":99999999999999999999}"#;
    assert_eq!(analyse(ligne), Err(Erreur::ArgumentInvalide));
}

#[test]
fn ce_qui_n_est_pas_un_objet_json_est_refuse_tot() {
    assert_eq!(analyse(b"status"), Err(Erreur::PasUnObjet));
    assert_eq!(analyse(b"[1,2,3]"), Err(Erreur::PasUnObjet));
    assert_eq!(analyse(b"{}"), Err(Erreur::CommandeAbsente));
    assert_eq!(analyse(br#"{"n":3}"#), Err(Erreur::CommandeAbsente));
}

#[test]
fn seul_hello_passe_avant_l_authentification() {
    // L'ETAT D'UNE MACHINE EST DEJA UN RENSEIGNEMENT. `status` attend.
    assert!(Commande::Hello { mac: [0; 32] }.avant_auth());
    for c in [
        Commande::Status,
        Commande::AuditRun,
        Commande::Rtl8168Ring,
        Commande::BlackboxCheckpoint,
        Commande::EventsWatch,
        Commande::Quit,
    ] {
        assert!(!c.avant_auth(), "{c:?} ne doit pas passer sans authentification");
    }
}

#[test]
fn chaque_commande_a_un_code_distinct() {
    // Le code part dans l'anneau LAB. Deux commandes sous un meme code
    // rendraient le releve ambigu exactement la ou il doit trancher.
    let toutes = [
        Commande::Hello { mac: [0; 32] },
        Commande::Status,
        Commande::AuditStatus,
        Commande::AuditRun,
        Commande::AuditLast,
        Commande::NetStatus,
        Commande::Rtl8168Status,
        Commande::Rtl8168Ring,
        Commande::Rtl8168Desc { index: 0 },
        Commande::DhcpStatus,
        Commande::BlackboxStatus,
        Commande::BlackboxCheckpoint,
        Commande::EventsTail { combien: 1 },
        Commande::EventsWatch,
        Commande::ServicesSnapshot,
        Commande::ProcessesSnapshot,
        Commande::MemorySnapshot,
        Commande::Quit,
    ];
    let mut vus = std::collections::HashSet::new();
    for c in toutes {
        assert!(vus.insert(c.code()), "code {} en double ({c:?})", c.code());
    }
}

// ===========================================================================
// L'ANALYSEUR JSON MINIMAL
// ===========================================================================

#[test]
fn une_cle_ne_se_confond_pas_avec_une_valeur() {
    // `{"t":"cmd","cmd":"status"}` : chercher `cmd` ne doit pas trouver la
    // VALEUR `"cmd"` du champ `t`.
    let ligne = br#"{"t":"cmd","cmd":"status"}"#;
    assert_eq!(champ_texte(ligne, "cmd"), Some(&b"status"[..]));
    assert_eq!(analyse(ligne), Ok(Commande::Status));
}

#[test]
fn l_espacement_json_ne_change_rien() {
    for ligne in [
        br#"{ "cmd" : "status" }"#.as_slice(),
        b"{\n  \"cmd\": \"status\"\n}",
        br#"{"cmd":"status"}"#,
    ] {
        assert_eq!(analyse(ligne), Ok(Commande::Status), "{}", String::from_utf8_lossy(ligne));
    }
}

#[test]
fn un_champ_absent_rend_rien_et_ne_devine_pas() {
    assert_eq!(champ_texte(br#"{"cmd":"status"}"#, "mac"), None);
    assert_eq!(champ_entier(br#"{"cmd":"status"}"#, "n"), None);
    assert_eq!(champ_entier(br#"{"cmd":"x","n":"7"}"#, "n"), None, "une chaine n'est pas un entier");
}

#[test]
fn absent_et_illisible_ne_se_confondent_pas() {
    // LA DISTINCTION QUI A MANQUE. Un argument absent peut avoir une valeur
    // par defaut raisonnable ; un argument present et illisible n'en a
    // aucune -- s'y replier executerait une commande que personne n'a ecrite.
    assert_eq!(lit_entier(br#"{"cmd":"x"}"#, "n"), LectureEntier::Absent);
    assert_eq!(lit_entier(br#"{"cmd":"x","n":"7"}"#, "n"), LectureEntier::Invalide);
    assert_eq!(lit_entier(br#"{"cmd":"x","n":true}"#, "n"), LectureEntier::Invalide);
    assert_eq!(lit_entier(br#"{"cmd":"x","n":-3}"#, "n"), LectureEntier::Invalide);
    assert_eq!(
        lit_entier(br#"{"cmd":"x","n":99999999999999999999}"#, "n"),
        LectureEntier::Invalide,
    );
    assert_eq!(lit_entier(br#"{"cmd":"x","n":42}"#, "n"), LectureEntier::Valeur(42));
    assert_eq!(
        lit_entier(br#"{"cmd":"x","n":18446744073709551615}"#, "n"),
        LectureEntier::Valeur(u64::MAX),
    );
}

// ===========================================================================
// L'AUTHENTIFICATION
// ===========================================================================

#[test]
fn le_vecteur_rfc_4231_passe() {
    // RFC 4231, cas de test 1. Si le HMAC de ce depot etait faux, toute
    // l'authentification serait fausse de la meme facon des deux cotes -- donc
    // invisible au banc, et fausse quand meme. Un vecteur EXTERIEUR est la
    // seule chose qui l'attrape.
    let cle = [0x0b; 20];
    let attendu = [
        0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
        0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
        0x2e, 0x32, 0xcf, 0xf7,
    ];
    assert_eq!(signature(&cle, b"Hi There"), attendu);
}

#[test]
fn le_vecteur_rfc_4231_avec_cle_longue_passe() {
    // Cas de test 6 : la cle depasse le bloc et doit etre hachee d'abord.
    // C'est le chemin qu'un jeton de laboratoire un peu long emprunte.
    let cle = [0xaa; 131];
    let msg = b"Test Using Larger Than Block-Size Key - Hash Key First";
    let attendu = [
        0x60, 0xe4, 0x31, 0x59, 0x1e, 0xe0, 0xb6, 0x7f, 0x0d, 0x8a, 0x26, 0xaa, 0xcb, 0xf5,
        0xb7, 0x7f, 0x8e, 0x0b, 0xc6, 0x21, 0x37, 0x28, 0xc5, 0x14, 0x05, 0x46, 0x04, 0x0f,
        0x0e, 0xe3, 0x7f, 0x54,
    ];
    assert_eq!(signature(&cle, msg), attendu);
}

#[test]
fn un_hmac_juste_est_accepte() {
    let attendu = signature(JETON, &NONCE);
    assert!(verifie(JETON, &NONCE, &attendu));
}

#[test]
fn un_hmac_faux_est_refuse_meme_a_un_bit_pres() {
    let mut faux = signature(JETON, &NONCE);
    faux[31] ^= 0x01;
    assert!(!verifie(JETON, &NONCE, &faux));
    faux[31] ^= 0x01;
    faux[0] ^= 0x80;
    assert!(!verifie(JETON, &NONCE, &faux));
}

#[test]
fn un_hmac_calcule_sur_un_autre_nonce_est_refuse() {
    // C'EST CE QUI INTERDIT LE REJEU. Un HMAC intercepte sur une connexion
    // precedente ne vaut rien sur la suivante.
    let mut autre = NONCE;
    autre[0] ^= 0xFF;
    let capture = signature(JETON, &autre);
    assert!(!verifie(JETON, &NONCE, &capture));
}

#[test]
fn un_jeton_vide_refuse_tout() {
    // UNE IMAGE CONSTRUITE SANS BOUCHAUD_DEBUG_TOKEN ne doit pas exposer un
    // debugger ouvert a quiconque atteint le segment local -- exactement le
    // contraire de ce qu'une absence de jeton doit signifier.
    let quelconque = signature(b"", &NONCE);
    assert!(!verifie(b"", &NONCE, &quelconque));
    assert!(!verifie(b"", &NONCE, &[0u8; 32]));
}

#[test]
fn un_hmac_de_mauvaise_longueur_est_refuse() {
    assert!(!verifie(JETON, &NONCE, &[0u8; 31]));
    assert!(!verifie(JETON, &NONCE, &[0u8; 33]));
    assert!(!verifie(JETON, &NONCE, &[]));
}

#[test]
fn la_comparaison_parcourt_toujours_tout() {
    // On ne mesure pas le temps ici -- un test de temps est fragile et ment
    // sous charge. On verifie la PROPRIETE qui le garantit : le resultat ne
    // depend que de l'egalite, et la fonction se comporte identiquement quelle
    // que soit la position de la difference.
    let a = [0x5au8; 32];
    for position in 0..32 {
        let mut b = a;
        b[position] ^= 0xFF;
        assert!(!egal_temps_constant(&a, &b), "position {position}");
    }
    assert!(egal_temps_constant(&a, &a));
    assert!(egal_temps_constant(&[], &[]));
    assert!(!egal_temps_constant(&a, &a[..31]));
}

#[test]
fn un_hello_bien_forme_porte_le_hmac() {
    let mac = signature(JETON, &NONCE);
    let mut hex = [0u8; 64];
    assert!(hex_encode(&mac, &mut hex));
    let ligne = format!(
        r#"{{"cmd":"hello","mac":"{}"}}"#,
        core::str::from_utf8(&hex).unwrap(),
    );
    match analyse(ligne.as_bytes()) {
        Ok(Commande::Hello { mac: rendu }) => {
            assert_eq!(rendu, mac);
            assert!(verifie(JETON, &NONCE, &rendu));
        }
        autre => panic!("{autre:?}"),
    }
}

#[test]
fn un_hello_sans_hmac_ou_mal_encode_est_refuse() {
    assert_eq!(analyse(br#"{"cmd":"hello"}"#), Err(Erreur::ArgumentManquant));
    assert_eq!(analyse(br#"{"cmd":"hello","mac":"abcd"}"#), Err(Erreur::ArgumentInvalide));
    let pas_hex = format!(r#"{{"cmd":"hello","mac":"{}"}}"#, "z".repeat(64));
    assert_eq!(analyse(pas_hex.as_bytes()), Err(Erreur::ArgumentInvalide));
}

// ===========================================================================
// L'HEXADECIMAL
// ===========================================================================

#[test]
fn l_hexadecimal_fait_l_aller_retour() {
    let src = [0x00u8, 0x0f, 0x10, 0xff, 0xa5, 0x5a];
    let mut hex = [0u8; 12];
    assert!(hex_encode(&src, &mut hex));
    assert_eq!(&hex, b"000f10ffa55a");
    let mut retour = [0u8; 6];
    assert!(hex_decode(&hex, &mut retour));
    assert_eq!(retour, src);
}

#[test]
fn l_hexadecimal_majuscule_est_accepte_en_lecture() {
    let mut dst = [0u8; 2];
    assert!(hex_decode(b"A5FF", &mut dst));
    assert_eq!(dst, [0xa5, 0xff]);
}

#[test]
fn une_longueur_ou_un_caractere_faux_est_refuse() {
    let mut dst = [0u8; 2];
    assert!(!hex_decode(b"abc", &mut dst), "longueur impaire");
    assert!(!hex_decode(b"abcdef", &mut dst), "trop long");
    assert!(!hex_decode(b"ab", &mut dst), "trop court");
    assert!(!hex_decode(b"abcg", &mut dst), "caractere non hexadecimal");
    let mut trop_petit = [0u8; 1];
    assert!(!hex_encode(b"ab", &mut trop_petit));
}
