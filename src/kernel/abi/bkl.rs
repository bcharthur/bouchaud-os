//! Quels appels systeme ont encore besoin du gros verrou noyau.
//!
//! ## Pourquoi une table plutot qu'un `match`
//!
//! Le retrait du BKL se fait appel par appel, et chaque retrait est un pari sur
//! une preuve : « cet appel se synchronise tout seul ». Un pari qui se perd ne
//! se voit pas a la compilation ni au boot — il se voit un jour, sous charge,
//! sur une machine a quatre coeurs, sous la forme d'une corruption qu'on ne
//! saura pas relier a sa cause.
//!
//! Trois proprietes rendent ce chantier tenable, et ce sont exactement les
//! trois que cette table donne :
//!
//!  1. **Le defaut est le verrou.** Un appel systeme ajoute demain, ou dont
//!     l'implementation change, garde le BKL sans que personne ait a y penser.
//!     Il faut un geste explicite pour le perdre.
//!  2. **La justification vit a cote de la decision.** Chaque numero libere
//!     porte la phrase qui dit *pourquoi* il l'est. Une justification qu'on ne
//!     peut plus ecrire est une ligne qu'il faut retirer.
//!  3. **C'est verifiable de l'exterieur.** `tools/verifie-verrouillage.py`
//!     relit cette table et l'aiguillage de `abi::dispatch`, et refuse qu'un
//!     appel soit declare sans verrou si son bras d'aiguillage fait autre chose
//!     que rendre une constante -- sauf pour les rares appels dont l'audit est
//!     nomme ici. Marquer un appel complexe « sans BKL » par inadvertance fait
//!     echouer la CI, pas la machine de l'utilisateur.
//!
//! ## Ce qui n'est PAS une preuve de surete
//!
//! « Cet appel a l'air simple » n'en est pas une. Les deux pieges rencontres
//! jusqu'ici dans ce noyau :
//!
//!  * [`crate::kernel::task::current`] rend un `&'static mut Task` obtenu en
//!    indexant `TASKS`, un `static mut Vec<Box<Task>>` qu'aucun verrou ne
//!    protege. Tout appel qui passe par la n'est pas liberable tant que cette
//!    table n'a pas son propre domaine de synchronisation. Cela couvre
//!    aujourd'hui `getpid`, `gettid`, `getuid`, `set_tid_address` et tout le
//!    reste de la famille identite.
//!  * `user_read`/`user_write` passent par
//!    [`crate::kernel::task::current_process`], qui **reprend** le BKL. Liberer
//!    un appel qui ecrit en memoire utilisateur ne le rend donc pas parallele :
//!    cela raccourcit seulement la tenue du verrou. Ce n'est pas faux, mais ce
//!    n'est pas non plus le gain qu'on croit obtenir.
//!
//! ## Etat du chantier
//!
//! Ce que la table libere aujourd'hui ne fait pas gagner de temps mesurable :
//! ce sont des appels rares, la plupart rendant une constante. C'est voulu. Le
//! premier lot sert a poser le mecanisme et sa verification, pas a courir apres
//! un chiffre. Les familles qui comptent vraiment -- identite, temps,
//! descripteurs -- attendent que la table des taches et l'acces a la memoire
//! utilisateur aient chacun leur propre domaine.

use super::nr;

/// Ce qu'un appel systeme exige du gros verrou noyau.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verrouillage {
    /// Le gros verrou est pris pour toute la duree de l'appel. C'est le defaut.
    Bkl,
    /// L'appel s'execute sans le gros verrou : sa synchronisation lui est
    /// propre, et la justification figure dans [`SANS_BKL`].
    Sans,
}

/// Les appels systeme qui s'executent sans le gros verrou, et pourquoi.
///
/// La justification n'est pas decorative : c'est elle qu'il faut pouvoir
/// reecrire quand l'implementation change. Si on ne sait plus l'ecrire, la
/// ligne doit disparaitre.
pub const SANS_BKL: &[(u64, &str)] = &[
    // --- Memoire : domaine `Arc<Process>::Mm` + protocole TLB sur IRQ -------
    //
    // Audite au jalon SMP4 (voir `arch::x86_64::usermode::syscall_dispatch`).
    // Ces deux appels ne touchent que des metadonnees serialisees par `Mm`,
    // l'allocateur de cadres (deja SMP-sur) et le protocole TLB. Des fils
    // freres peuvent ainsi muter des espaces d'adressage independants en
    // parallele. `munmap` reste exclu : il peut declencher une reecriture
    // MAP_SHARED vers le RAMFS, dont le coeur est encore sous BKL.
    (nr::MPROTECT, "metadonnees Mm + protocole TLB, aucun etat global"),
    (nr::BRK, "metadonnees Mm + protocole TLB, aucun etat global"),
    // --- Constantes : le bras d'aiguillage ne lit ni n'ecrit rien ------------
    //
    // Ces appels rendent une valeur litterale. Ils ne touchent ni la table des
    // taches, ni la memoire utilisateur, ni aucun etat partage : il n'y a
    // litteralement rien a serialiser. `tools/verifie-verrouillage.py` le
    // verifie sur l'aiguillage lui-meme, pour que la table ne puisse pas
    // survivre a une implementation qui, elle, aurait cesse d'etre triviale.
    (nr::LINK, "constante : -ENOTSUP, le RAMFS n'a pas de liens durs"),
    (nr::INOTIFY_INIT1, "constante : -ENOSYS, pas de surveillance de fichiers"),
    (nr::RSEQ, "constante : -ENOSYS, refus delibere de rseq"),
    (nr::TIMES, "constante : 0"),
    (nr::SYSLOG, "constante : 0"),
    (nr::MEMBARRIER, "constante : 0"),
    (nr::SETUID, "constante : 0, pas de modele d'utilisateurs"),
    (nr::SETGID, "constante : 0, pas de modele de groupes"),
    (nr::SETPGID, "constante : 0, pas de groupes de processus"),
    (nr::SETSID, "constante : 0, pas de sessions"),
    (nr::GETPPID, "constante : 1"),
    (nr::GETPGRP, "constante : 1"),
    (nr::GETPGID, "constante : 1"),
    (nr::GETSID, "constante : 1"),
    (nr::MLOCK, "constante : 0, tout est deja resident"),
    (nr::MUNLOCK, "constante : 0, tout est deja resident"),
    (nr::MLOCKALL, "constante : 0, tout est deja resident"),
    (nr::MUNLOCKALL, "constante : 0, tout est deja resident"),
    (nr::SET_ROBUST_LIST, "constante : 0, nettoyage de verrous non tenu"),
    (nr::GET_ROBUST_LIST, "constante : 0, nettoyage de verrous non tenu"),
    (nr::SCHED_SETPARAM, "constante : 0, une seule classe de priorite"),
    (nr::SCHED_SETSCHEDULER, "constante : 0, une seule politique"),
    (nr::SCHED_SETAFFINITY, "constante : 0, affinite non honoree ici"),
];

/// Ce que cet appel systeme exige du gros verrou.
pub fn verrouillage(numero: u64) -> Verrouillage {
    let mut i = 0;
    while i < SANS_BKL.len() {
        if SANS_BKL[i].0 == numero {
            return Verrouillage::Sans;
        }
        i += 1;
    }
    Verrouillage::Bkl
}

/// Cet appel systeme doit-il etre execute sous le gros verrou ?
///
/// C'est le defaut : tout ce qui n'est pas explicitement libere l'exige.
pub fn exige_bkl(numero: u64) -> bool {
    verrouillage(numero) == Verrouillage::Bkl
}
