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
//! Premier lot : le mecanisme, sa verification, et vingt-trois appels rendant
//! une constante. Aucun gain mesurable, et c'etait annonce.
//!
//! Deuxieme lot : l'identite et le temps, une fois la tache courante dotee de
//! son propre domaine ([`crate::kernel::task::identite_courante`]). Les
//! frequences mesurees sur les sondes libc de ce depot -- `syscalls` les
//! affiche desormais -- placent `clock_gettime` dans la tete de liste d'une
//! boucle d'evenements ; l'identite, elle, y est rare, et c'est dit tel quel
//! dans le journal du commit plutot que maquille.
//!
//! Troisieme lot : `poll` et `ppoll`. Son domaine existait deja -- le verrou de
//! la table des descripteurs, plus celui de chaque objet ; ce qui l'obligeait au
//! gros verrou etait la ROUTE vers le processus, `current_process()`, qui le
//! reprend. `current_process_local()` donne le meme `Arc` sans toucher `TASKS`.
//! Mesure a l'appui : `poll` tenait 23 a 38 % du verrou sur des fenetres de 5 s
//! d'un vrai chargement de Google.
//!
//! Ce qui reste, et qui est le vrai gisement : `writev`/`write`/`read`,
//! `mmap`/`munmap`/`close`, `rt_sigprocmask`. Ils demandent chacun un domaine
//! que ce noyau n'a pas encore -- coeur du systeme de fichiers, etat de signaux
//! par tache.

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
    // --- Identite : domaine CPU-local (`task::identite_courante`) -----------
    //
    // Lu :    `usermode::per_cpu().current` (le tid, bloc par-CPU adresse par
    //         GS) et `CURRENT_PROCESS[cpu]` (un `Arc<Process>`), plus
    //         `Process::metadata` pour uid/gid.
    // Ecrit : rien.
    // Verrou : les deux emplacements par-CPU ne sont ecrits que par `install`,
    //         sous le gros verrou, et seulement par CE CPU ; la lecture coupe
    //         les interruptions, donc aucun changement de contexte ne peut s'y
    //         intercaler. `metadata` est un `SpinLock` sur le `Process`, un
    //         domaine independant de la table des taches.
    // Duree de vie : le tid est une valeur ; le `Process` est retenu par une
    //         part d'`Arc` prise avant de rendre la main, donc il survit a la
    //         mort de la tache. Aucune reference vers une `Task` ne sort.
    // Memoire utilisateur : aucune.
    // Pourquoi pas de gros verrou : `TASKS` n'est pas touchee. C'etait sa
    //         seule raison d'etre sur ce chemin.
    // --- Attente de readiness : domaine table des descripteurs + objets -----
    //
    // BOUCHAUD_P3_POLL_SANS_BKL_V1
    //
    // Lu :    `process.files` (un `SpinLock<FdTable>`), puis le verrou propre
    //         de chaque objet -- `PipeState`, `Canal`, `EventFd`, `TimerFd`.
    //         La memoire utilisateur, pour lire les `pollfd` et y ecrire les
    //         `revents`.
    // Ecrit : `revents` en memoire utilisateur ; `TimerFd::expirations` sous le
    //         verrou de l'objet.
    // Verrou : chaque objet a le sien, et la table des descripteurs a le sien.
    //         `TASKS` n'est jamais touchee : le processus courant vient de
    //         `current_process_local()`, qui lit le bloc par-CPU interruptions
    //         coupees. C'etait la seule raison pour laquelle ce chemin reprenait
    //         le gros verrou, et elle a disparu.
    // Attente : `wait_readiness` passe par `WaitQueue::wait`, qui prend le gros
    //         verrou LUI-MEME, le temps d'inscrire la tache et de la parquer.
    //         `park_current_on` le suspend ensuite pour de bon avant de commuter
    //         (voir `smp_lock::suspend_for_schedule`). Personne ne dort en le
    //         tenant.
    // Etat global sans verrou : trois branches en touchent, et elles prennent
    //         le gros verrou elles-memes, au plus court -- clavier et souris
    //         (`static mut` de `kernel::input`), et socket inet (l'anneau e1000,
    //         entierement en `static mut`). Voir `file.rs`.
    // Pourquoi le liberer : mesure. Sur un vrai Ladybird chargeant Google,
    //         `[BKL-SYSCALL]` a donne `poll` a 23-38 % de detention du verrou
    //         sur des fenetres de 5 s, pour 100 000 acquisitions -- de loin le
    //         premier consommateur une fois `madvise` corrige. Un `poll` de
    //         sept descripteurs n'a aucune raison de serialiser trois autres
    //         coeurs.
    (nr::POLL, "table des descripteurs + verrou par objet ; l'attente prend le verrou elle-meme"),
    (nr::PPOLL, "table des descripteurs + verrou par objet ; l'attente prend le verrou elle-meme"),
    (nr::GETPID, "domaine CPU-local, aucune lecture de TASKS"),
    (nr::GETTID, "domaine CPU-local, aucune lecture de TASKS"),
    (nr::GETUID, "domaine CPU-local + verrou metadata du Process"),
    (nr::GETEUID, "domaine CPU-local + verrou metadata du Process"),
    (nr::GETGID, "domaine CPU-local + verrou metadata du Process"),
    (nr::GETEGID, "domaine CPU-local + verrou metadata du Process"),
    // --- Temps : horloges atomiques + memoire utilisateur -------------------
    //
    // Lu :    `kernel::timer` (TICKS, TSC_HZ, BOOT_TSC : que des atomiques,
    //         `monotonic_ns` maintient meme la monotonie inter-CPU par
    //         `fetch_max`) et l'ancre d'epoque, elle aussi atomique depuis ce
    //         lot -- c'etait un `static mut Option<(u64, u64)>` en
    //         initialisation paresseuse, donc une course des que deux CPU
    //         lisent l'heure sans verrou. Corrige a la source, pas contourne.
    // Ecrit : la memoire utilisateur, et rien d'autre.
    // Verrou : `Mm` pour la traduction et l'ecriture -- le domaine deja audite
    //         au jalon SMP4 pour `mprotect`/`brk`. Le remplissage a la demande
    //         (`peuple_a_la_demande`) est deja appele SANS gros verrou par le
    //         gestionnaire de faute de page, sur tous les CPU : ce chemin-la
    //         n'est pas nouveau.
    // Duree de vie : `user_read`/`user_write` tiennent une part d'`Arc` sur le
    //         `Process` pendant toute l'operation.
    // Memoire utilisateur : oui, en ecriture, par `user_write`.
    // Pourquoi pas de gros verrou : la seule branche qui touchait la table des
    //         taches est `CLOCK_PROCESS_CPUTIME_ID`/`CLOCK_THREAD_CPUTIME_ID`,
    //         qui passe par `cpu_time_ms` ; elle prend le gros verrou
    //         explicitement, dans le corps de l'appel. Le reste s'en passe.
    (nr::CLOCK_GETTIME, "horloges atomiques + Mm ; le verrou est pris dans la branche CPUTIME"),
    (nr::CLOCK_GETRES, "constante calculee + Mm"),
    (nr::GETTIMEOFDAY, "ancre d'epoque atomique + Mm"),
    (nr::TIME, "ancre d'epoque atomique + Mm"),
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
