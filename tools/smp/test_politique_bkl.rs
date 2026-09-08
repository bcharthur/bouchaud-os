//! Preuve hote de la table de politique du gros verrou.
//!
//! Les modules de production `src/compat/linux/nr.rs` et
//! `src/compat/linux/bkl.rs` sont inclus tels quels : ce qui est mis a
//! l'epreuve ici est la table qui decide, a chaque appel systeme, si le noyau
//! entier se serialise.
//!
//! # Ce que ces cas protegent
//!
//! Le retrait du gros verrou se fait appel par appel, et chaque retrait est un
//! pari sur une preuve. Deux regressions sont possibles, et elles sont
//! opposees :
//!
//!   * **liberer par accident** -- un appel qui touche de l'etat global cesse
//!     d'etre serialise, et la corruption apparait un jour, sous charge, sans
//!     qu'on sache la relier a sa cause. C'est `tools/verifie-verrouillage.py`
//!     qui garde ce cote-la, en exigeant un audit nomme ;
//!   * **reprendre par accident** -- un appel deja libere retombe sous le
//!     verrou, et la contention revient sans que rien ne le dise. C'est ce
//!     fichier qui garde ce cote-la.

// `nr.rs` sait s'imprimer lui-meme (la commande `syscalls` du shell) et passe
// pour cela par les macros du noyau. L'hote n'en a pas ; les rendre muettes
// permet d'inclure le module de PRODUCTION plutot qu'une copie qui lui
// ressemble -- c'est tout l'interet de la manoeuvre.
macro_rules! print {
    ($($arg:tt)*) => {{}};
}
macro_rules! println {
    ($($arg:tt)*) => {{}};
}
pub(crate) use {print, println};

#[path = "../../src/compat/linux/nr.rs"]
mod nr;

#[path = "../../src/compat/linux/bkl.rs"]
mod bkl;

use bkl::{exige_bkl, verrouillage, Verrouillage, SANS_BKL};

/// LE DEFAUT EST LE VERROU.
///
/// C'est la propriete qui rend ce chantier tenable : un appel systeme ajoute
/// demain, ou dont l'implementation change, reste serialise sans que personne
/// ait a y penser. Il faut un geste explicite pour le perdre.
#[test]
fn tout_ce_qui_n_est_pas_declare_garde_le_verrou() {
    // Un numero qui n'existe dans aucune table.
    assert!(exige_bkl(60_000));
    // Un appel reel, volontairement laisse sous verrou : il descend dans le
    // coeur du systeme de fichiers, dont le domaine n'existe pas encore.
    assert!(exige_bkl(nr::OPENAT));
    assert!(exige_bkl(nr::IOCTL));
    assert!(exige_bkl(nr::EXECVE));
}

/// `FUTEX` est libere, et doit le rester.
///
/// C'est l'appel le plus cher a laisser sous verrou sur une charge de
/// navigateur : un programme multifil en emet un a chaque contention de verrou
/// de sa libc. Le chemin `futex_wait`/`futex_wake` SUSPENDAIT deja le verrou
/// que l'aiguilleur venait de prendre, pour le reprendre ensuite -- une
/// acquisition globale et deux liberations autour d'un coeur `wait_word` qui
/// ne partage rien.
#[test]
fn le_futex_reste_hors_du_gros_verrou() {
    assert!(!exige_bkl(nr::FUTEX));
    assert_eq!(verrouillage(nr::FUTEX), Verrouillage::Sans);
}

/// Les appels dont le retrait a ete mesure ne doivent pas revenir en arriere.
/// Chacun a coute un audit ecrit ; les reprendre silencieusement rendrait la
/// contention sans que rien ne le signale.
#[test]
fn les_retraits_deja_mesures_ne_regressent_pas() {
    for numero in [
        nr::POLL, nr::PPOLL,     // 23-38 % de detention mesures sur Ladybird
        nr::READ, nr::WRITE,     // l'appel le plus frequent d'un navigateur
        nr::GETRANDOM,           // 473 444 acquisitions mesurees
        nr::MMAP, nr::CLOSE,     // lot c4
        nr::FUTEX,               // lot c5
        nr::CLOCK_GETTIME,       // tete de liste d'une boucle d'evenements
        nr::SCHED_YIELD,         // relachait deja le verrou pour commuter
        // lot c6 : la famille de la boucle d'evenements, meme domaine que POLL
        nr::EVENTFD, nr::EVENTFD2,
        nr::TIMERFD_CREATE, nr::TIMERFD_SETTIME, nr::TIMERFD_GETTIME,
        nr::PIPE, nr::PIPE2, nr::EPOLL_CTL,
    ] {
        assert!(!exige_bkl(numero), "l'appel {} est retombe sous le gros verrou", numero);
    }
}

/// `NANOSLEEP` et `CLOCK_NANOSLEEP` restent sous le gros verrou, et ce n'est
/// pas un oubli.
///
/// Ils descendent dans `task::sleep_ticks`, qui porte un `debug_assert!` sans
/// ambiguite : « requiert le BKL externe de l'appelant ». Le chemin suspend
/// puis reprend ce verrou externe autour de la commutation, et la profondeur
/// rendue est verifiee. Les liberer sans changer d'abord ce contrat ferait
/// suspendre une profondeur nulle et reprendre une profondeur nulle -- ce qui
/// passerait les tests et romprait l'invariant que l'assertion protege.
///
/// Ce cas existe pour que le prochain qui parcourt la liste des appels encore
/// verrouilles trouve la raison ici, plutot que de refaire l'analyse.
#[test]
fn le_sommeil_reste_sous_verrou_tant_que_son_contrat_l_exige() {
    assert!(exige_bkl(nr::NANOSLEEP));
    assert!(exige_bkl(nr::CLOCK_NANOSLEEP));
}

/// Chaque ligne porte une justification NON VIDE. Une justification qu'on ne
/// peut plus ecrire est une ligne qu'il faut retirer -- pas une ligne qu'on
/// laisse avec une chaine vide.
///
/// Le seuil est la non-vacuite, et rien de plus. Exiger une longueur minimale
/// serait arbitraire : « constante : 0 » dit tout ce qu'il y a a dire d'un
/// appel dont le bras d'aiguillage rend zero, et une regle qui le refuserait
/// pousserait a delayer la phrase plutot qu'a la rendre vraie.
#[test]
fn chaque_liberation_porte_sa_justification() {
    for (numero, raison) in SANS_BKL {
        assert!(
            !raison.trim().is_empty(),
            "l'appel {} est libere sans justification : {:?}",
            numero, raison
        );
    }
}

/// La table ne doit pas contenir deux fois le meme appel. Un doublon rend la
/// seconde justification morte : on la relit, on la croit vraie, et c'est la
/// premiere qui decide.
#[test]
fn aucun_appel_n_est_declare_deux_fois() {
    let mut vus = std::collections::HashSet::new();
    for (numero, _) in SANS_BKL {
        assert!(vus.insert(*numero), "l'appel {} est declare deux fois", numero);
    }
}
