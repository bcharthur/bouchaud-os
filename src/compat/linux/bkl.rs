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
//!  * [`crate::kernel::task::current`] rendait un `&'static mut Task` obtenu en
//!    indexant `TASKS`, un `static mut Vec<Box<Task>>` qu'aucun verrou ne
//!    protegeait. Ce n'est plus le cas : la table est un registre a
//!    emplacements generationnels, `current` rend une reference PARTAGEE, et
//!    les champs que d'autres coeurs touchent sont atomiques. La famille
//!    identite -- `getpid`, `gettid`, `getuid`, `set_tid_address` -- n'a donc
//!    plus besoin du gros verrou pour cette raison-la.
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
    // V14: munmap/madvise are now fully domain-owned: Mm serialises VMA/PTE
    // metadata, the shared/clean caches own their locks, and TLB ACK waits run
    // with no Mm guard. RAMFS writeback already takes a short internal BKL in
    // `memory/shared.rs`; neither syscall needs an outer global lock.
    (nr::MUNMAP, "V14: Mm + TLB + caches auto-synchronises; no outer BKL"),
    (nr::MADVISE, "V14: Mm + TLB + clean-cache; no outer BKL"),
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
    // V14: copyin and iovec walking execute with no outer BKL. `ecrit_octets`
    // uses per-object locks for pipe/eventfd/socketpair, and takes a short
    // internal BKL only for legacy global console/RAMFS/audio/inet branches.
    // This prevents a demand fault during write/writev copyin from owning the
    // global lock -- the exact site=212 / ~239 ms stall seen on V13.3.1.
    (nr::WRITE, "V14: copyin sans BKL; legacy global sinks lock internally"),
    (nr::WRITEV, "V14: iovec/copyin sans BKL; sys_write preserves sink domains"),
    // --- Lecture : la symetrique de WRITE, liberee une fois ses sources
    //     verrouillees --------------------------------------------------------
    //
    // `WRITE` etait libere depuis V14 et `READ` ne l'etait pas. Sur un
    // navigateur, c'est l'appel le plus frequent du noyau : chaque `read` de
    // chaque `WebContent` prenait le verrou global, et le tenait pendant la
    // recopie vers l'utilisateur -- donc pendant une eventuelle faute de
    // demande, exactement le blocage que V14 avait retire du cote ecriture.
    //
    // Il le tenait AUSSI pendant les attentes bloquantes : `console_read`
    // tourne sur `attends_un_tick` jusqu'a la premiere touche, et la branche
    // tube attend `wait_readiness`. Dormir en tenant le verrou global est ce
    // qu'il ne faut jamais faire ; c'etait le cas ici a chaque lecture de
    // console ou de tube vide.
    //
    // Ce que chaque branche possede maintenant :
    //   File        RAMFS (rang `Vfs`) et cache de lecture pour le
    //               contenu, `CONTROLLER` de l'ATA pour le disque.
    //   Console     la file de scancodes (`SpinLockIrq`) et l'etat du
    //               decodeur, qui portait encore un `static mut`.
    //   Random      le generateur, dont `compteur` etait lu et incremente
    //               sans atomicite : deux CPU pouvaient produire le MEME
    //               bloc d'alea.
    //   Input       le sous-systeme d'entree, deja verrouille.
    //   Pipe, SocketPair, EventFd, TimerFd  leur propre verrou d'objet.
    //   Instantane  le contenu vit dans le descripteur.
    //   Socket      SEULE branche encore globale : l'anneau e1000 et la pile
    //               inet. Elle prend le verrou elle-meme, borne a la mutation
    //               reseau, comme la branche symetrique de `ecrit_octets`.
    //
    // Memoire utilisateur : oui, en ecriture, par `user_write` -- hors verrou,
    //               ce qui est le but.
    // Duree de vie : `user_write` tient une part d'`Arc` sur le `Process`.
    (nr::READ, "sources verrouillees par objet ; seule la branche socket prend le verrou, en interne"),
    (nr::READV, "boucle sur `sys_read`, audite ci-dessus ; l'iovec se lit depuis l'utilisateur"),
    // --- Alea : le generateur porte son propre verrou depuis c1 -------------
    //
    // BOUCHAUD_C13_GETRANDOM_SANS_BKL_V1
    //
    // Lu :    rien de global. `rng::fill` prend `GENERATEUR`, un `SpinLock` qui
    //         couvre ensemble la lecture de l'etat et l'INCREMENT du compteur.
    //         C'est leur atomicite conjointe qui garantit qu'aucun bloc ne sort
    //         deux fois ; le lot c1 l'a mise en place en liberant `read`, dont
    //         la branche `Random` appelle exactement ce chemin.
    // Ecrit : la memoire utilisateur, par `user_write`, et rien d'autre.
    // Verrou : `GENERATEUR` pour l'alea ; `Mm` pour la traduction et l'ecriture
    //         -- le domaine deja audite au jalon SMP4. La route vers le
    //         processus est `processus_courant()`, qui prefere
    //         `current_process_local()` : `TASKS` n'est pas touchee.
    // Duree de vie : `user_write` tient une part d'`Arc` sur le `Process`
    //         pendant toute l'operation.
    // Faute de demande : `fault_in_user_range` peut en declencher une. Le
    //         gestionnaire de faute s'execute deja sans gros verrou sur tous
    //         les CPU ; c'est le meme chemin que `read` et `clock_gettime`.
    // Allocation : un tampon de 4 Kio au plus, par l'allocateur global, deja
    //         SMP-sur -- `read` et `write` en allouent a chaque appel.
    // Pourquoi le liberer : mesure. Sur une session de navigateur,
    //         `[BKL-SYSCALL]` donne `getrandom` a 473 444 acquisitions, avec
    //         une attente maximale de 218 ms relevee a l'entree du verrou sur
    //         le CPU 1. Un demi-million de fois, quatre coeurs se sont
    //         serialises pour lire un generateur qui possede son propre
    //         verrou -- et c'est le JS d'un CAPTCHA, celui-la meme dont
    //         l'utilisateur dit qu'il n'en voit pas la fin, qui en emet le plus.
    (nr::GETRANDOM, "generateur a verrou propre depuis c1 + Mm ; aucune lecture de TASKS"),
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
    // --- Lot c2 : ce qui ne touche que la tache courante ---------------------
    //
    // Ces quatre appels ne lisent ni la table des taches, ni le systeme de
    // fichiers, ni aucun etat partage entre processus. Ils touchent :
    //
    //   * un REGISTRE du CPU courant (`FS_BASE`), ecrit par `wrmsr` -- il n'y a
    //     rien de plus local qu'un registre de modele specifique ;
    //   * un CHAMP de la tache courante, pris par `current_exclusif()`, qui est
    //     un garde par EMPLACEMENT : il attend l'emplacement de cette tache-ci,
    //     pas un verrou global ;
    //   * la memoire utilisateur, par `user_write`, qui prend le verrou `mm` du
    //     processus -- le meme domaine dont `WRITE`, `BRK` et `MPROTECT`
    //     dependent depuis V14.
    //
    // `ARCH_PRCTL` est le plus important des quatre : la glibc l'emet a la
    // creation de CHAQUE fil pour poser sa zone de stockage local. Un
    // navigateur qui demarre ses processus de rendu en emet donc autant qu'il
    // cree de fils, et chacun prenait le gros verrou pour ecrire un registre.
    (nr::ARCH_PRCTL, "registre FS_BASE du CPU courant + champ de la tache courante + user_write (verrou mm)"),
    (nr::SET_TID_ADDRESS, "champ `clear_child_tid` de la tache courante, par garde d'emplacement"),
    (nr::SCHED_GETAFFINITY, "masque constant + user_write (verrou mm) ; ne lit pas la table des taches"),
    (nr::GETPRIORITY, "lit `current().priorite`, un atomique par tache ; aucune table parcourue"),

    // --- Lot c3 : la table des descripteurs, et rien de plus -----------------
    //
    // Ces cinq appels prennent `process.files.lock()` -- la table des
    // descripteurs, dont le domaine est celui sur lequel `POLL` repose depuis
    // le lot A1/3 -- et rien d'autre qui soit partage entre processus.
    //
    // `LSEEK` et `FSTAT` descendent en plus dans `backing` et `ramfs`, dont les
    // domaines `Fs` et `Vfs` sont declares SORTIS et verifies comme tels : leur
    // reprise du gros verrou serait comptee comme une regression par
    // `[BKL-DOMAINES] regressions=`, que le budget borne a zero.
    //
    // `FSTAT` est le plus chaud des cinq et le moins evident : la glibc l'emet
    // a chaque `fopen` pour dimensionner son tampon. Un navigateur qui ouvre
    // ses polices, ses certificats et ses ressources en emet des centaines au
    // demarrage, et chacun prenait le gros verrou pour lire une taille.
    (nr::FSTAT, "table des descripteurs + ramfs/backing, domaines Fs et Vfs declares sortis"),
    (nr::LSEEK, "table des descripteurs + `backing::logical_len`, domaine Fs declare sorti"),
    (nr::DUP, "table des descripteurs seule ; le descripteur est clone puis insere sous le meme verrou"),
    (nr::DUP2, "table des descripteurs seule ; meme chemin que DUP"),
    (nr::DUP3, "table des descripteurs seule ; meme chemin que DUP"),

    // --- Lot c3 : l'ordonnanceur, qui n'a jamais eu besoin du verrou ---------
    //
    // `SCHED_YIELD` etait le cas le plus absurde du lot. Il prenait le gros
    // verrou, puis appelait `schedule()`, qui appelle `suspend_for_schedule()`
    // -- lequel RELACHE le verrou pour la duree du changement de contexte et le
    // reprend au retour. On payait donc une acquisition globale, une liberation
    // et une reacquisition pour un appel dont tout l'objet est de rendre la
    // main. `suspend_for_schedule()` gere deja la profondeur zero : une tache
    // qui cede sans tenir le verrou suit exactement le meme chemin, en moins.
    //
    // `SETPRIORITY` parcourt la table des taches, mais sous `VueRegistre`, qui
    // tient une LECTURE du registre -- le domaine `RegistreProcessus`, sorti du
    // gros verrou et servi par son propre verrou de rang. Ce qu'il ecrit est un
    // atomique porte par chaque tache.
    (nr::SCHED_YIELD, "`schedule()` relache deja le verrou pour commuter ; le prendre avant ne sert qu'a le rendre"),
    (nr::SETPRIORITY, "lecture du registre (domaine RegistreProcessus, sorti) + atomique par tache"),

    // --- Lot c4 : les deux plus chauds qui restaient -------------------------
    //
    // `MMAP` etait l'anomalie du groupe memoire : `MUNMAP`, `MPROTECT`, `BRK`
    // et `MADVISE` sont liberes depuis V14, et lui seul prenait encore le
    // verrou. Il ne touche pourtant rien que ces quatre-la ne touchent :
    //
    //   * `mm.lock()` -- le domaine dont V14 depend deja ;
    //   * `files.lock()` -- la table des descripteurs, domaine du lot A1/3 ;
    //   * `metadata.lock()` -- le verrou du `Process`, domaine du lot A1/2 ;
    //   * `backing::logical_len` et `is_disk_backed` -- domaine `Fs`, sorti ;
    //   * `partage::mappe` -- son propre `CACHE.lock()` dans `memory/shared.rs` ;
    //   * `finish_mapping_replacement`, le MEME chemin de remplacement que
    //     `munmap` utilise, et que son audit couvre deja.
    //
    // Reste `peuple_a_la_demande`, appele quand `MAP_POPULATE` est demande.
    // C'est le gestionnaire de FAUTE DE PAGE, et c'est ce qui tranche : une
    // faute de page peut survenir a tout instant, y compris pendant qu'un autre
    // coeur tient le gros verrou. S'il en avait besoin, le systeme serait deja
    // casse. Il prend d'ailleurs `current_process_local()`, l'accesseur sans
    // verrou que le lot A1/3 exige.
    //
    // `CLOSE` ne touche que trois domaines, tous DECLARES SORTIS et verifies
    // comme tels : la table des descripteurs, les verrous d'enregistrement
    // POSIX (`VerrouEnregistrement`, servi par un verrou de rang `PosixRecord`)
    // et la readiness (`Readiness`). Le verrou de la table est relache avant
    // `notify_readiness` -- le commentaire du code le dit deja, et c'est ce qui
    // evite de tenir deux domaines a la fois.
    (nr::MMAP, "mm + table des descripteurs + metadata + Fs, et la faute de page qui n'a jamais eu le verrou"),
    (nr::CLOSE, "table des descripteurs, verrous d'enregistrement et readiness : trois domaines declares sortis"),

    // --- Lot c5 : le futex, qui relachait deja le verrou qu'on lui donnait ---
    //
    // BOUCHAUD_C5_FUTEX_SANS_BKL_V1
    //
    // C'est le meme cas que `SCHED_YIELD`, en plus cher. L'aiguilleur prenait
    // le gros verrou parce que la table le disait ; `futex_wait` et
    // `futex_wake` appelaient aussitot `smp_lock::suspend_for_schedule()` pour
    // s'en debarrasser, faisaient leur travail, puis le REPRENAIENT pour que
    // l'aiguilleur puisse le relacher. Une acquisition globale, une
    // liberation, une reacquisition et une seconde liberation, autour d'un
    // chemin qui n'en voulait pas.
    //
    // Et c'est l'appel le plus cher a laisser ainsi. Un navigateur multifil
    // emet un futex a CHAQUE contention de verrou de sa libc : chaque attente
    // d'un fil de rendu serialisait les autres coeurs le temps d'entrer dans
    // une primitive qui, elle, ne partage rien.
    //
    // Lu :    la memoire utilisateur, pour la duree (`timespec_ms`), par le
    //         domaine `Mm` -- celui dont `MMAP`, `BRK` et `MPROTECT` dependent
    //         depuis V14 ; et les horloges atomiques (`monotonic_ms`,
    //         `unix_time`), deja auditees pour `TIME` et `GETTIMEOFDAY`.
    // Ecrit : rien hors du coeur wait-word.
    // Verrou : `wait_word` est a domaines propres et le montre -- soixante-
    //         quatre seaux, chacun un `SpinLock<Vec<Arc<WaitWordEntry>>>`,
    //         chaque entree portant sa propre `WaitSource` et ses compteurs
    //         atomiques. Aucun parcours de la table des taches : la recherche
    //         est locale au seau, par cle physique.
    // Attente : `wait_word_wait` parque la tache. Elle ne dort donc PAS en
    //         tenant le gros verrou -- c'est precisement ce que le
    //         `suspend_for_schedule` interne garantissait deja, et ce que ce
    //         retrait rend inutile.
    // Falsification : `[BKL-FUTEX] herites=` compte les operations entrees avec
    //         un verrou HERITE. Ce compteur doit desormais rester a zero ; s'il
    //         monte, c'est qu'un appelant redonne le verrou au futex, et
    //         l'invariant est faux. La preuve est runtime, et elle est
    //         refutable.
    (nr::FUTEX, "c5: wait-word a seaux verrouilles + Mm + horloges atomiques ; le chemin suspendait deja le verrou lui-meme"),

    // --- Lot c6 : la famille de la boucle d'evenements -----------------------
    //
    // BOUCHAUD_C6_BOUCLE_EVENEMENTS_SANS_BKL_V1
    //
    // Huit appels du meme domaine, et c'est celui sur lequel `POLL` repose
    // depuis le lot A1/3 : la table des descripteurs, plus le verrou propre de
    // chaque objet. Un navigateur les emet en rafale -- creation des tubes de
    // ses processus de rendu, minuteries de sa boucle Qt, inscription des
    // descripteurs a surveiller -- et chacun serialisait les autres coeurs.
    //
    // Lu :    `task::current_process()`, puis `process.files.lock()`, puis le
    //         verrou de l'objet vise (`EventFdState`, `TimerFdState`, la liste
    //         epoll, l'etat de tube). Les horloges atomiques pour les
    //         minuteries. La memoire utilisateur pour les descripteurs rendus
    //         et les `epoll_event`.
    // Ecrit : le seul objet vise, sous son propre verrou ; la table des
    //         descripteurs, sous le sien ; la memoire utilisateur par
    //         `user_write` (domaine `Mm`).
    // Verrou : aucun etat partage entre processus n'est touche. Les objets sont
    //         des `Arc<SpinLock<...>>` crees par l'appel lui-meme, ou atteints
    //         par un descripteur du processus courant.
    //
    // Ce qui a change depuis que l'en-tete de ce fichier deconseillait ce lot :
    // `current_process()` ne reprend PLUS le gros verrou. Il lit le champ
    // `process` de la tache courante -- pose a la creation, jamais modifie --
    // et clone un `Arc`, ce qui est atomique par construction. C'etait la
    // seule raison pour laquelle ces huit appels le reprenaient.
    (nr::EVENTFD, "c6: table des descripteurs + objet cree sur place ; current_process() ne reprend plus le verrou"),
    (nr::EVENTFD2, "c6: identique a EVENTFD, avec les drapeaux"),
    (nr::TIMERFD_CREATE, "c6: table des descripteurs + objet cree sur place"),
    (nr::TIMERFD_SETTIME, "c6: verrou de l'objet minuterie + horloges atomiques + Mm"),
    (nr::TIMERFD_GETTIME, "c6: verrou de l'objet minuterie + horloges atomiques + Mm"),
    (nr::PIPE, "c6: table des descripteurs + etat de tube cree sur place + Mm pour les deux descripteurs rendus"),
    (nr::PIPE2, "c6: identique a PIPE, avec les drapeaux"),
    (nr::EPOLL_CTL, "c6: table des descripteurs + verrou de la liste epoll + Mm"),

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
    (nr::PRCTL, "constante : 0, aucune operation de controle de processus n'est honoree"),
    (nr::SCHED_GETSCHEDULER, "constante : 0, une seule politique"),
    (nr::SIGALTSTACK, "constante : 0, pas de pile de signal alternative"),
    (nr::UMASK, "constante : 0o022, le RAMFS n'a pas de masque de creation"),
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
