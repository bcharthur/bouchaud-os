//! Empreintes de mots de passe : PBKDF2-HMAC-SHA256, sel par compte,
//! comparaison a temps constant.
//!
//! # Ce que ce module remplace
//!
//! Les mots de passe etaient stockes EN CLAIR dans la table des comptes, et
//! compares par `==` :
//!
//! ```text
//! fn pass_eq(&self, pass: &str) -> bool {
//!     self.pass_len == pass.len() && &self.pass[..self.pass_len] == pass.as_bytes()
//! }
//! ```
//!
//! Trois defauts, dans l'ordre de gravite :
//!
//!   1. **En clair.** Toute lecture de la memoire noyau -- une image de
//!      panique, un `/proc` trop bavard, un pilote qui deborde -- rend tous les
//!      mots de passe du systeme. Il n'y a pas de « faible risque » ici : le
//!      mot de passe est reutilise ailleurs par l'utilisateur.
//!   2. **Sans sel.** Deux comptes ayant le meme mot de passe auraient la meme
//!      empreinte, et une table precalculee les casserait tous les deux d'un
//!      coup.
//!   3. **Comparaison qui sort tot.** `==` sur des tranches s'arrete au premier
//!      octet different. Le temps de reponse revele donc combien d'octets sont
//!      justes, ce qui permet de deviner le secret octet par octet.
//!
//! # Le choix de PBKDF2, et son cout
//!
//! Argon2id resisterait mieux au materiel dedie. L'ecrire correctement sans
//! pouvoir le valider serait cependant plus dangereux que ce qu'il apporte :
//! une derivation subtilement fausse ne se voit pas, et affaiblit tout.
//! PBKDF2-HMAC-SHA256 se verifie exactement, vecteur par vecteur, contre une
//! implantation de reference -- et c'est ce que fait `tools/test_motdepasse.rs`.
//!
//! Le noyau porte deja SHA-256 et HMAC pour TLS : la derivation ne fait
//! qu'iterer dessus, sans nouvelle primitive a auditer.
//!
//! Le nombre d'iterations est STOCKE AVEC CHAQUE EMPREINTE, et non fige dans
//! le code. C'est ce qui permet de l'augmenter plus tard sans invalider les
//! comptes existants : une empreinte ancienne reste verifiable avec son propre
//! cout, et se reecrit au prochain changement de mot de passe.

use crate::net::security::tls::sha256::hmac;

pub const LONGUEUR_SEL: usize = 16;
pub const LONGUEUR_EMPREINTE: usize = 32;

/// Cout par defaut d'une nouvelle empreinte.
///
/// Bouchaud tourne sous TCG, ou un coeur emule vaut une petite fraction du
/// coeur hote. A ce rythme, 24 000 iterations coutent quelques centaines de
/// millisecondes -- acceptable pour une ouverture de session, et deja quatre
/// ordres de grandeur au-dessus d'un simple SHA-256 pour qui essaie un
/// dictionnaire.
///
/// Ce n'est pas le chiffre qu'on choisirait sur du materiel reel. C'est le
/// chiffre qu'on peut tenir ici sans rendre l'ouverture de session penible, et
/// il se releve sans migration puisqu'il voyage avec l'empreinte.
pub const ITERATIONS_DEFAUT: u32 = 24_000;

/// L'empreinte d'un compte, ou l'absence d'empreinte.
///
/// « Pas de mot de passe » et « mot de passe vide » sont deux etats
/// DIFFERENTS, et les confondre est la faute classique : un compte cree sans
/// mot de passe accepterait alors la chaine vide. `defini == false` refuse
/// toute authentification, quoi qu'on presente.
#[derive(Clone, Copy)]
pub struct Empreinte {
    defini: bool,
    iterations: u32,
    sel: [u8; LONGUEUR_SEL],
    empreinte: [u8; LONGUEUR_EMPREINTE],
}

impl Empreinte {
    /// Un compte VERROUILLE : aucune authentification ne peut reussir.
    pub const fn verrouille() -> Self {
        Self {
            defini: false,
            iterations: 0,
            sel: [0; LONGUEUR_SEL],
            empreinte: [0; LONGUEUR_EMPREINTE],
        }
    }

    /// Derive l'empreinte d'un mot de passe avec le sel fourni.
    pub fn nouvelle(mot: &str, sel: [u8; LONGUEUR_SEL], iterations: u32) -> Self {
        let iterations = iterations.max(1);
        Self {
            defini: true,
            iterations,
            sel,
            empreinte: derive(mot.as_bytes(), &sel, iterations),
        }
    }

    /// Vrai si `mot` est le mot de passe de ce compte.
    ///
    /// Un compte verrouille rend toujours faux -- y compris pour la chaine
    /// vide, et y compris si l'appelant insiste.
    pub fn verifie(&self, mot: &str) -> bool {
        if !self.defini {
            return false;
        }
        let candidat = derive(mot.as_bytes(), &self.sel, self.iterations);
        egal_temps_constant(&candidat, &self.empreinte)
    }

    pub fn est_verrouille(&self) -> bool {
        !self.defini
    }

    pub fn iterations(&self) -> u32 {
        self.iterations
    }
}

/// PBKDF2-HMAC-SHA256, sortie de 32 octets (RFC 8018).
///
/// Un seul bloc suffit : la longueur demandee vaut exactement celle du
/// condensat, donc `T_1` est tout le resultat.
pub fn derive(
    mot: &[u8],
    sel: &[u8],
    iterations: u32,
) -> [u8; LONGUEUR_EMPREINTE] {
    // U_1 = HMAC(mot, sel || INT(1)) -- l'indice de bloc est en gros-boutien.
    let mut message = [0u8; 64];
    let longueur = sel.len().min(message.len() - 4);
    message[..longueur].copy_from_slice(&sel[..longueur]);
    message[longueur..longueur + 4].copy_from_slice(&1u32.to_be_bytes());

    let mut u = hmac(mot, &message[..longueur + 4]);
    let mut resultat = u;
    for _ in 1..iterations {
        u = hmac(mot, &u);
        for octet in 0..LONGUEUR_EMPREINTE {
            resultat[octet] ^= u[octet];
        }
    }
    resultat
}

/// Comparaison dont le temps ne depend pas de l'endroit ou les octets
/// divergent.
///
/// `==` sur des tranches s'arrete au premier octet different : le temps de
/// reponse revele alors combien d'octets sont justes, et le secret se devine
/// octet par octet. On accumule donc les differences et on ne decide qu'a la
/// fin.
pub fn egal_temps_constant(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for index in 0..a.len() {
        difference |= a[index] ^ b[index];
    }
    difference == 0
}
