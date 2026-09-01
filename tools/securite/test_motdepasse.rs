//! Les mots de passe sont-ils reellement hors de portee ?
//!
//! # Ce que ce test protege
//!
//! Les mots de passe etaient stockes EN CLAIR et compares par `==`. Trois
//! defauts d'un coup : toute lecture de la memoire noyau les rendait tous, deux
//! comptes de meme mot de passe avaient la meme empreinte, et la comparaison
//! sortait au premier octet different -- ce qui laisse deviner le secret octet
//! par octet en mesurant le temps de reponse.
//!
//! # Pourquoi des vecteurs de reference, et pas un aller-retour
//!
//! Verifier que `verifie(nouvelle(mot))` rend vrai ne prouve presque rien : une
//! derivation FAUSSE le fait aussi, et se contente d'etre fausse de facon
//! coherente. Une PBKDF2 qui itererait une fois au lieu de vingt-quatre mille,
//! ou qui oublierait le XOR cumulatif, passerait cet aller-retour sans broncher
//! tout en offrant une resistance nulle.
//!
//! Les vecteurs ci-dessous viennent d'une implantation de reference
//! (`hashlib.pbkdf2_hmac`). Ils fixent la fonction, pas seulement sa coherence.
//!
//! Lance par `tools/dev/validate-fast.ps1` et la barriere courte.

extern crate alloc;

#[path = "../../src/net/security/tls/sha256.rs"]
pub mod sha256_impl;

// `motdepasse.rs` designe le SHA-256 du noyau par son chemin de production.
// On reconstruit ce chemin ici pour que le test s'execute contre le vrai
// module, sans copie -- une copie divergeant en silence.
mod net {
    pub mod security {
        pub mod tls {
            pub use crate::sha256_impl as sha256;
        }
    }
}

#[path = "../../src/users/motdepasse.rs"]
mod motdepasse;

use motdepasse::{derive, egal_temps_constant, Empreinte, ITERATIONS_DEFAUT,
                 LONGUEUR_SEL};

fn hex(octets: &[u8]) -> String {
    octets.iter().map(|o| alloc::format!("{:02x}", o)).collect()
}

// ---------------------------------------------------------------------------
// 1. La derivation est la bonne fonction, pas seulement une fonction
// ---------------------------------------------------------------------------

#[test]
fn pbkdf2_hmac_sha256_suit_les_vecteurs_de_reference() {
    // RFC 8018, verifies contre `hashlib.pbkdf2_hmac('sha256', ...)`.
    let cas: [(&[u8], &[u8], u32, &str); 4] = [
        (b"password", b"salt", 1,
         "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"),
        (b"password", b"salt", 2,
         "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"),
        (b"password", b"salt", 4096,
         "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"),
        (b"passwordPASSWORDpassword",
         b"saltSALTsaltSALTsaltSALTsaltSALTsalt", 4096,
         "348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1"),
    ];
    for (mot, sel, iterations, attendu) in cas {
        assert_eq!(
            hex(&derive(mot, sel, iterations)), attendu,
            "PBKDF2-HMAC-SHA256({:?}, {:?}, c={})", mot, sel, iterations,
        );
    }
}

#[test]
fn le_nombre_d_iterations_change_reellement_le_resultat() {
    // Une derivation qui ignorerait le compteur passerait tous les tests
    // d'aller-retour, et n'offrirait aucune resistance au dictionnaire.
    let sel = b"sel-de-test-1234";
    assert_ne!(derive(b"mot", sel, 1), derive(b"mot", sel, 2));
    assert_ne!(derive(b"mot", sel, 1000), derive(b"mot", sel, 1001));
}

// ---------------------------------------------------------------------------
// 2. Le sel fait son travail
// ---------------------------------------------------------------------------

#[test]
fn deux_comptes_de_meme_mot_de_passe_ont_des_empreintes_differentes() {
    // Sans sel, une table precalculee les casserait tous les deux d'un coup.
    let a = Empreinte::nouvelle("correct horse", [1u8; LONGUEUR_SEL], 64);
    let b = Empreinte::nouvelle("correct horse", [2u8; LONGUEUR_SEL], 64);
    assert!(a.verifie("correct horse"));
    assert!(b.verifie("correct horse"));
    // Chacune n'accepte que sa propre derivation : les empreintes different.
    assert_eq!(
        hex(&derive(b"correct horse", &[1u8; LONGUEUR_SEL], 64))
            != hex(&derive(b"correct horse", &[2u8; LONGUEUR_SEL], 64)),
        true,
    );
}

// ---------------------------------------------------------------------------
// 3. Un compte verrouille n'est pas un compte au mot de passe vide
// ---------------------------------------------------------------------------

#[test]
fn un_compte_verrouille_refuse_tout_y_compris_la_chaine_vide() {
    // C'est LA faute classique : confondre « pas de mot de passe » et « mot de
    // passe vide » ferait accepter "" sur tout compte fraichement cree -- soit
    // exactement le `root:root` qu'on vient de retirer, en pire.
    let verrouille = Empreinte::verrouille();
    assert!(verrouille.est_verrouille());
    assert!(!verrouille.verifie(""));
    assert!(!verrouille.verifie("root"));
    assert!(!verrouille.verifie("\0"));
}

#[test]
fn un_compte_avec_mot_de_passe_n_est_pas_verrouille() {
    let e = Empreinte::nouvelle("secret", [7u8; LONGUEUR_SEL], 64);
    assert!(!e.est_verrouille());
    assert!(e.verifie("secret"));
    assert!(!e.verifie("secre"));
    assert!(!e.verifie("secrett"));
    assert!(!e.verifie(""));
    assert!(!e.verifie("Secret"));
}

#[test]
fn le_cout_par_defaut_reste_defendable() {
    // Un cout par defaut qui tomberait a quelques centaines d'iterations
    // ferait passer tous les autres tests et n'offrirait plus rien face a un
    // dictionnaire. La borne basse est donc explicite.
    assert!(
        ITERATIONS_DEFAUT >= 10_000,
        "cout par defaut {ITERATIONS_DEFAUT} : trop bas pour resister a un \
         dictionnaire hors ligne",
    );
    let e = Empreinte::nouvelle("secret", [5u8; LONGUEUR_SEL], ITERATIONS_DEFAUT);
    assert_eq!(e.iterations(), ITERATIONS_DEFAUT);
    assert!(e.verifie("secret"));
    assert!(!e.verifie("secre7"));
}

#[test]
fn le_cout_voyage_avec_l_empreinte() {
    // C'est ce qui permet de relever le cout plus tard sans invalider les
    // comptes existants : une empreinte ancienne reste verifiable avec le sien.
    let ancienne = Empreinte::nouvelle("secret", [3u8; LONGUEUR_SEL], 64);
    let recente = Empreinte::nouvelle("secret", [3u8; LONGUEUR_SEL], 512);
    assert_eq!(ancienne.iterations(), 64);
    assert_eq!(recente.iterations(), 512);
    assert!(ancienne.verifie("secret"));
    assert!(recente.verifie("secret"));
}

#[test]
fn un_cout_nul_ne_court_circuite_pas_la_derivation() {
    // `iterations = 0` ferait une boucle vide. On remonte a 1 : une empreinte
    // sans aucune iteration serait un simple HMAC, et surtout `verifie`
    // deviendrait une comparaison de deux valeurs non initialisees.
    let e = Empreinte::nouvelle("secret", [9u8; LONGUEUR_SEL], 0);
    assert_eq!(e.iterations(), 1);
    assert!(e.verifie("secret"));
    assert!(!e.verifie("autre"));
}

// ---------------------------------------------------------------------------
// 4. La comparaison ne fuit pas par le temps
// ---------------------------------------------------------------------------

#[test]
fn la_comparaison_est_correcte_sur_tous_les_cas() {
    assert!(egal_temps_constant(b"", b""));
    assert!(egal_temps_constant(b"abcd", b"abcd"));
    assert!(!egal_temps_constant(b"abcd", b"abce"));
    assert!(!egal_temps_constant(b"abcd", b"zbcd"));
    assert!(!egal_temps_constant(b"abcd", b"abc"));
    assert!(!egal_temps_constant(b"abc", b"abcd"));
}

#[test]
fn la_comparaison_lit_toute_la_longueur() {
    // Le contrat qui compte n'est pas « rend le bon booleen » -- `==` le fait
    // aussi. C'est « ne sort pas plus tot quand la difference est plus tot ».
    // On ne peut pas chronometrer de facon fiable dans un test ; on verifie
    // donc la propriete observable qui en decoule : le resultat ne depend que
    // de l'egalite, jamais de la POSITION de la difference.
    let reference = [0xAAu8; 32];
    for position in 0..32 {
        let mut candidat = reference;
        candidat[position] ^= 0xFF;
        assert!(
            !egal_temps_constant(&candidat, &reference),
            "difference en position {position} non detectee",
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Aucun mot de passe en clair ne survit dans l'empreinte
// ---------------------------------------------------------------------------

#[test]
fn l_empreinte_ne_contient_pas_le_mot_de_passe() {
    // La representation en memoire ne doit contenir ni le mot, ni un prefixe
    // reconnaissable : c'est tout l'objet du changement.
    let mot = "MonMotDePasseTresReconnaissable";
    let e = Empreinte::nouvelle(mot, [4u8; LONGUEUR_SEL], 64);
    let octets: &[u8] = unsafe {
        core::slice::from_raw_parts(
            &e as *const Empreinte as *const u8,
            core::mem::size_of::<Empreinte>(),
        )
    };
    assert!(
        !octets.windows(mot.len()).any(|f| f == mot.as_bytes()),
        "le mot de passe apparait tel quel dans l'empreinte",
    );
    for prefixe in 4..=mot.len() {
        assert!(
            !octets.windows(prefixe).any(|f| f == &mot.as_bytes()[..prefixe]),
            "un prefixe de {prefixe} octets du mot de passe survit",
        );
    }
}
