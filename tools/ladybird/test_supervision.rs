//! Preuve hote de la supervision multi-processus du navigateur.
//!
//! Le module de production `src/kernel/navigateur/supervision.rs` est inclus
//! tel quel, avec un `SpinLock` d'hote a la place de celui du noyau : ce qui
//! est mis a l'epreuve est la logique de supervision, pas le verrou.
//!
//! # Ce qui manquait
//!
//! Le noyau savait qu'un client graphique existait, et il n'en connaissait
//! qu'UN. Or Ladybird n'est pas un processus : c'est un courtier, un serveur
//! de requetes, un decodeur d'images, et un moteur de rendu PAR CONTEXTE. Le
//! passage a plusieurs onglets ne demande pas seulement de les lancer : il
//! demande de savoir lequel est mort, de ne pas emporter les autres, et de ne
//! pas relancer indefiniment celui qui plante a chaque essai.
//!
//! # Le scenario que ces tests jouent
//!
//!   1. le courtier lance plusieurs WebContent ;
//!   2. l'un d'eux meurt ;
//!   3. le courtier reste VIVANT, et les autres rendus aussi ;
//!   4. un nouveau WebContent peut etre lance a sa place ;
//!   5. un rendu qui plante en boucle finit par se voir refuser la relance.

#![allow(dead_code)]

extern crate alloc;

/// Le verrou du noyau, reduit a ce qu'il fait ici : l'exclusion mutuelle.
mod kernel {
    pub mod sync {
        pub struct SpinLock<T> {
            inner: std::sync::Mutex<T>,
        }
        impl<T> SpinLock<T> {
            pub const fn new(valeur: T) -> Self {
                Self { inner: std::sync::Mutex::new(valeur) }
            }
            pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
                self.inner.lock().unwrap_or_else(|e| e.into_inner())
            }
        }
    }
}

use crate::kernel::sync::SpinLock;

/// Le releve journalise sur le port serie ; sur l'hote, la trace n'a pas de
/// destination.
#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{ let _ = format!($($arg)*); }};
}

mod supervision {
    use super::SpinLock;
    use core::sync::atomic::{AtomicU64, Ordering};
    include!("../../src/kernel/navigateur/supervision_corps.rs");
}

use supervision::{
    autorise_relance, compteurs, contexte, etat, note_lancement, note_sortie, oublie,
    vivants, Etat, Role, RELANCES_MAX, SUIVIS_MAX,
};

const COURTIER: u32 = 100;
const MAINTENANT: u64 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Le role se deduit de l'image, comme la classification de securite.
// ---------------------------------------------------------------------------

#[test]
fn chaque_image_du_navigateur_a_son_role() {
    assert_eq!(
        Role::depuis_image("/usr/libexec/ladybird/WebContent"), Some(Role::Rendu)
    );
    assert_eq!(
        Role::depuis_image("/usr/libexec/ladybird/RequestServer"), Some(Role::Reseau)
    );
    assert_eq!(
        Role::depuis_image("/usr/libexec/ladybird/ImageDecoder"), Some(Role::Decodeur)
    );
    assert_eq!(
        Role::depuis_image("/usr/libexec/ladybird/WebWorker"), Some(Role::Travailleur)
    );
    assert_eq!(
        Role::depuis_image("/usr/libexec/ladybird/BrowserHost"), Some(Role::Courtier)
    );
    assert_eq!(
        Role::depuis_image("/usr/bin/ls"), None,
        "un programme ordinaire n'est pas supervise : le registre est borne, et \
         le remplir de tout ce qui tourne le rendrait inutile"
    );
}

/// LE DEFAUT, verrouille : le courtier se reconnait a son NOM, pas a son
/// repertoire.
///
/// La reconnaissance exigeait `/usr/bin/bo-navigateur`. Or le bureau lance
/// `client::CHEMIN_NAVIGATEUR`, qui vaut `/bo-navigateur` -- le binaire est
/// deplie a la racine du RAMFS. Aucun lancement reel n'etait donc reconnu, et
/// le registre restait vide : la supervision existait et ne supervisait rien.
#[test]
fn le_courtier_se_reconnait_quel_que_soit_son_repertoire() {
    for chemin in [
        "/bo-navigateur",
        "/usr/bin/bo-navigateur",
        "/usr/libexec/ladybird/BrowserHost",
        "/BrowserHost",
    ] {
        assert_eq!(
            Role::depuis_image(chemin), Some(Role::Courtier),
            "{chemin} doit etre reconnu : le repertoire a deja change une fois"
        );
    }
    // Les roles enfants aussi, quel que soit l'emplacement.
    assert_eq!(Role::depuis_image("/WebContent"), Some(Role::Rendu));
    assert_eq!(Role::depuis_image("/opt/l/RequestServer"), Some(Role::Reseau));
    // Et un nom qui CONTIENT le mot n'est pas le mot.
    assert_eq!(Role::depuis_image("/usr/bin/mon-WebContent-a-moi"), None);
    assert_eq!(Role::depuis_image("/bo-navigateur-test"), None);
}

/// Seul le courtier emporte ses enfants. C'est LA regle d'isolation.
#[test]
fn seul_le_courtier_emporte_ses_enfants() {
    assert!(Role::Courtier.emporte_ses_enfants());
    for role in [Role::Rendu, Role::Reseau, Role::Decodeur, Role::Travailleur] {
        assert!(
            !role.emporte_ses_enfants(),
            "{role:?} ne doit emporter personne : c'est ce que veut dire \
             « un moteur de rendu qui plante n'emporte pas le navigateur »"
        );
    }
}

// ---------------------------------------------------------------------------
// LE SCENARIO : plusieurs rendus, l'un meurt, le navigateur survit.
// ---------------------------------------------------------------------------

#[test]
fn un_rendu_qui_plante_n_emporte_pas_le_navigateur() {
    // Le courtier, puis trois onglets.
    assert!(note_lancement(COURTIER, Role::Courtier, 0, 0, MAINTENANT));
    for onglet in 1..=3u32 {
        assert!(note_lancement(200 + onglet, Role::Rendu, COURTIER, onglet, MAINTENANT));
    }
    assert!(note_lancement(300, Role::Reseau, COURTIER, 0, MAINTENANT));

    assert_eq!(vivants(Role::Rendu), 3);
    assert_eq!(vivants(Role::Courtier), 1);

    // L'onglet 2 plante.
    assert_eq!(note_sortie(202, 139, MAINTENANT + 1), Some(Role::Rendu));

    assert_eq!(etat(202), Some(Etat::Plante));
    assert_eq!(
        etat(COURTIER), Some(Etat::Vivant),
        "LE COURTIER RESTE VIVANT : c'est toute la raison d'etre du decoupage \
         en processus"
    );
    assert_eq!(etat(201), Some(Etat::Vivant), "les autres onglets aussi");
    assert_eq!(etat(203), Some(Etat::Vivant));
    assert_eq!(etat(300), Some(Etat::Vivant), "et le serveur de requetes");
    assert_eq!(vivants(Role::Rendu), 2);

    // Un nouveau WebContent prend sa place.
    assert!(autorise_relance(Role::Rendu, 2, MAINTENANT + 2));
    oublie(202);
    assert!(note_lancement(204, Role::Rendu, COURTIER, 2, MAINTENANT + 3));
    assert_eq!(vivants(Role::Rendu), 3, "l'onglet est de nouveau servi");
    assert_eq!(contexte(204), Some(2));

    // Nettoyage pour les tests suivants : les statiques sont partagees.
    for pid in [COURTIER, 201, 203, 204, 300] {
        oublie(pid);
    }
}

/// La mort du COURTIER, elle, condamne ses enfants : ils n'ont plus personne a
/// qui parler, et les laisser vivants ferait des processus qui tournent pour
/// rien.
#[test]
fn la_mort_du_courtier_orpheline_ses_enfants() {
    const HOTE: u32 = 400;
    assert!(note_lancement(HOTE, Role::Courtier, 0, 0, MAINTENANT));
    for onglet in 1..=2u32 {
        assert!(note_lancement(410 + onglet, Role::Rendu, HOTE, onglet, MAINTENANT));
    }
    // Un rendu d'un AUTRE courtier ne doit pas etre touche.
    const AUTRE: u32 = 500;
    assert!(note_lancement(AUTRE, Role::Courtier, 0, 0, MAINTENANT));
    assert!(note_lancement(511, Role::Rendu, AUTRE, 1, MAINTENANT));

    assert_eq!(note_sortie(HOTE, 1, MAINTENANT + 1), Some(Role::Courtier));

    assert_eq!(etat(411), Some(Etat::Orphelin));
    assert_eq!(etat(412), Some(Etat::Orphelin));
    assert_eq!(
        etat(511), Some(Etat::Vivant),
        "le rendu d'un AUTRE courtier ne doit pas etre emporte"
    );
    assert_eq!(etat(AUTRE), Some(Etat::Vivant));

    for pid in [HOTE, 411, 412, AUTRE, 511] {
        oublie(pid);
    }
}

// ---------------------------------------------------------------------------
// LA BOUCLE DE PLANTAGE.
// ---------------------------------------------------------------------------

/// Relancer sans compter transforme un moteur de rendu qui plante sur une page
/// en une machine qui ne fait plus que redemarrer.
#[test]
fn une_boucle_de_plantage_finit_par_etre_refusee() {
    const ONGLET: u32 = 77;
    for tour in 0..RELANCES_MAX {
        assert!(
            autorise_relance(Role::Rendu, ONGLET, MAINTENANT + tour as u64),
            "la relance {tour} doit etre permise"
        );
    }
    assert!(
        !autorise_relance(Role::Rendu, ONGLET, MAINTENANT + RELANCES_MAX as u64),
        "au-dela de {RELANCES_MAX} relances rapprochees, insister ne sert plus \
         qu'a occuper la machine"
    );
}

/// Un plantage isole tous les quarts d'heure n'est PAS une boucle, et ne doit
/// pas finir par bloquer.
#[test]
fn un_plantage_isole_ne_bloque_pas_apres_la_fenetre() {
    const ONGLET: u32 = 78;
    for tour in 0..RELANCES_MAX {
        assert!(autorise_relance(Role::Rendu, ONGLET, MAINTENANT + tour as u64));
    }
    assert!(!autorise_relance(Role::Rendu, ONGLET, MAINTENANT + 10));

    // Bien plus tard : la fenetre est passee.
    let plus_tard = MAINTENANT + supervision::FENETRE_RELANCE_NS * 2;
    assert!(
        autorise_relance(Role::Rendu, ONGLET, plus_tard),
        "la fenetre passee, le compteur repart : sinon un onglet ouvert toute \
         la journee finirait par ne plus pouvoir se relancer"
    );
}

/// Le budget est PAR CONTEXTE : un onglet qui plante ne doit pas empecher un
/// autre de se relancer.
#[test]
fn le_budget_de_relance_est_par_contexte() {
    const MAUVAIS: u32 = 90;
    const BON: u32 = 91;
    for tour in 0..RELANCES_MAX {
        assert!(autorise_relance(Role::Rendu, MAUVAIS, MAINTENANT + tour as u64));
    }
    assert!(!autorise_relance(Role::Rendu, MAUVAIS, MAINTENANT + 10));
    assert!(
        autorise_relance(Role::Rendu, BON, MAINTENANT + 10),
        "un autre onglet ne doit pas payer les plantages du premier"
    );
}

// ---------------------------------------------------------------------------
// Bornes et cycle de vie.
// ---------------------------------------------------------------------------

/// Un pid recycle efface son ancienne entree : sans cela, deux entrees
/// porteraient le meme pid et la sortie serait attribuee a la mauvaise.
#[test]
fn un_pid_recycle_remplace_son_ancienne_entree() {
    const PID: u32 = 600;
    assert!(note_lancement(PID, Role::Rendu, COURTIER, 1, MAINTENANT));
    note_sortie(PID, 1, MAINTENANT + 1);
    assert_eq!(etat(PID), Some(Etat::Plante));

    assert!(note_lancement(PID, Role::Decodeur, COURTIER, 2, MAINTENANT + 2));
    assert_eq!(
        etat(PID), Some(Etat::Vivant),
        "la nouvelle incarnation ne doit pas heriter de l'etat de l'ancienne"
    );
    assert_eq!(contexte(PID), Some(2));
    oublie(PID);
}

#[test]
fn la_sortie_d_un_processus_non_supervise_ne_dit_rien() {
    assert_eq!(
        note_sortie(999_999, 0, MAINTENANT), None,
        "tout ce qui tourne n'est pas du navigateur"
    );
    assert_eq!(etat(999_999), None);
}

/// Le registre est borne. Un registre qui alloue se fait epuiser par un
/// courtier qui boucle -- precisement le cas qu'on cherche a diagnostiquer.
#[test]
fn le_registre_est_borne_et_le_dit() {
    // Repartir d'un registre propre.
    for pid in 0..2000u32 {
        oublie(pid);
    }
    for index in 0..SUIVIS_MAX as u32 {
        assert!(
            note_lancement(1000 + index, Role::Rendu, COURTIER, index, MAINTENANT),
            "entree {index}"
        );
    }
    let avant = compteurs().registre_plein;
    assert!(
        !note_lancement(9000, Role::Rendu, COURTIER, 9000, MAINTENANT),
        "le registre plein doit REFUSER, pas ecraser une entree vivante"
    );
    assert_eq!(
        compteurs().registre_plein, avant + 1,
        "et le dire : un processus non supervise doit se voir dans la trace"
    );
    for index in 0..SUIVIS_MAX as u32 {
        oublie(1000 + index);
    }
}
