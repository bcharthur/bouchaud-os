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

    // --- Lot c7 : le sommeil, dont le contrat exigeait ce dont il se defaisait
    //
    // BOUCHAUD_C7_SOMMEIL_SANS_BKL_V1
    //
    // `sleep_ticks` portait `debug_assert!(held_by_current_cpu())`. L'exigence
    // ne decrivait rien de ce que la fonction fait : tout ce qu'elle touche est
    // atomique -- `wake_deadline_ns` et `state` sont des stores ordonnes,
    // `arme_echeance` un `fetch_min` -- et sa boucle d'attente tourne SANS le
    // verrou, que `suspend_for_schedule()` rend des la premiere ligne.
    //
    // L'assertion forcait donc ses appelants a prendre un verrou global pour le
    // lui rendre aussitot. Le contrat a ete change a la source plutot que
    // contourne : la profondeur d'entree est relevee quelle qu'elle soit, et
    // `verifie_profondeur_rendue` exige toujours qu'on la retrouve. Zero est
    // une profondeur comme une autre.
    //
    // Lu :    la memoire utilisateur pour la duree (`timespec_ms`, domaine
    //         `Mm`) et les horloges atomiques.
    // Ecrit : `remain` en memoire utilisateur, et les deux champs atomiques de
    //         la tache courante.
    // Verrou : aucun. `schedule()` s'execute deja sans le gros verrou.
    (nr::NANOSLEEP, "c7: contrat de sleep_ticks corrige ; stores atomiques + Mm, la boucle tournait deja sans verrou"),
    (nr::CLOCK_NANOSLEEP, "c7: identique a NANOSLEEP, avec l'echeance absolue"),


    // --- Lot c8/v2 : receive-side socket sans BKL externe --------------------
    // BOUCHAUD_C8_RECV_SANS_BKL_V2
    //
    // Le pump inet legacy reprend Domaine::Reseau + BKL en interne, en
    // conservant l'ordre BKL -> SocketState. Les attentes se font hors BKL.
    // CONNECT/SEND*/SHUTDOWN restent volontairement sous le verrou.
    (nr::SOCKET, "c8v2: creation locale + table des descripteurs"),
    (nr::SOCKETPAIR, "c8v2: Canaux + table des descripteurs + Mm"),
    (nr::BIND, "c8v2: SocketState + port ephemere AtomicU16"),
    (nr::GETPEERNAME, "c8v2: SocketState + Mm"),
    (nr::SETSOCKOPT, "c8v2: no-op explicite"),
    (nr::GETSOCKOPT, "c8v2: SocketState/Canal + Mm"),
    (nr::RECVFROM, "c8v2: pump inet borne en interne, attente sans BKL"),
    (nr::RECVMSG, "c8v2: canaux/FD + RECVFROM audite"),
    (nr::RECVMMSG, "c8v2: boucle RECVMSG + safe point"),

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
    // B1 -- RETRAIT COMPLET DU GROS VERROU, LOT 1 : les appels simples.
    //
    // Audit. Aucun de ces appels ne parcourt la table des taches, ni le VFS,
    // ni la pile reseau. Ce qu'ils touchent a chacun son propre verrou :
    //  * `uname` : tampon sur pile, chaines constantes (`crate::VERSION`),
    //    `user_write` -> verrou `mm` du processus courant ;
    //  * `sysinfo` : `vmm::frame_stats` prend `FRAMES` (SpinLockIrq), l'uptime
    //    derive du compteur atomique de ticks, puis `user_write` ;
    //  * `getrlimit`/`setrlimit`/`prlimit64` : `limite_as` vit sous le verrou
    //    `mm` du processus ; les autres limites sont des constantes ; la
    //    lecture de l'ancienne valeur precede l'ecriture de la nouvelle SOUS
    //    deux prises distinctes -- comme avant sous BKL, puisque `mm.lock()`
    //    etait deja pris separement pour chacune.
    (nr::SCHED_GET_PRIORITY_MAX, "constante : 0, une seule classe de priorite"),
    (nr::SCHED_GET_PRIORITY_MIN, "constante : 0, une seule classe de priorite"),
    (nr::UNAME, "B1 -- tampon sur pile + chaines constantes + verrou mm"),
    (nr::SYSINFO, "B1 -- FRAMES (SpinLockIrq) + ticks atomiques + verrou mm"),
    (nr::GETRLIMIT, "B1 -- limite_as sous verrou mm ; autres limites constantes"),
    (nr::SETRLIMIT, "B1 -- limite_as sous verrou mm"),
    (nr::PRLIMIT64, "B1 -- limite_as sous verrou mm ; autres limites constantes"),
    // B2 -- `fcntl`, premier consommateur mesure du gros verrou.
    //
    // Mesure (endurance SMP4, [BKL-INVENTAIRE]) : fcntl=54910 acquisitions,
    // 860 ms tenu, 617 ms d'attente -- plus que tous les autres appels
    // reunis. `F_SETLKW` dort un tick par essai, et chaque reveil reprenait
    // le gros verrou pour retenter la pose.
    //
    // Audit. Chaque partie a son domaine :
    //  * `F_DUPFD`/`F_GETFD`/`F_SETFD`/`F_GETFL`/`F_SETFL` : table des
    //    descripteurs seule (`files.lock()`), lecture et ecriture sous la
    //    MEME prise ;
    //  * `F_GETLK`/`F_SETLK`/`F_SETLKW` : `user_read`/`user_write` (verrou
    //    mm) ; table des descripteurs puis `backing::logical_len` (EXTENTS,
    //    puis FS en RankedSpinLock) -- ordre FdTable -> Vfs, celui deja
    //    documente par `openat` ; les verrous d'enregistrement vivent dans
    //    `VERROUS`, RankedSpinLock de classe PosixRecord, dont `pose` fait
    //    le test ET l'insertion sous une seule prise (plus de course
    //    check-then-insert) ; l'attente est `sleep_ticks`, qui ne suppose
    //    plus le gros verrou.
    (nr::FCNTL, "B2 -- descripteurs + VERROUS (PosixRecord) + FS/EXTENTS + mm ; attente sans BKL"),
    // B3 -- persistance et memoire composee.
    //
    // Mesure (endurance SMP4) : fsync=348 acquisitions, 199-466 ms tenus.
    //
    // Audit.
    //  * `fsync`/`fdatasync` : table des descripteurs, puis `sous_racine`
    //    (FS) ; `sync` et les deux precedents appellent
    //    `persistance::synchronise`, serialise par `TRANSACTION`
    //    (SleepMutex) : l'instantane lit l'arbre sous `fs()`, et l'ecriture
    //    disque tournait DEJA gros verrou rendu (`suspend_for_schedule`), sous
    //    `TRANSACTION` seul et le verrou du controleur ATA ;
    //  * `msync` : `mm` du processus, puis `shared::writeback` -- CACHE
    //    (SpinLock) et le RAMFS sous `fs()` ; le commentaire qui y parle d'un
    //    RAMFS en `static mut` date d'avant son RankedSpinLock ;
    //  * `mremap` : composition de `mmap`, `munmap`, `user_read`,
    //    `user_write`, tous deja hors gros verrou -- son atomicite vis-a-vis
    //    d'un `munmap` concurrent n'etait donc deja plus garantie par lui.
    (nr::FSYNC, "B3 -- descripteurs + FS + TRANSACTION ; l'ecriture tournait deja sans BKL"),
    (nr::FDATASYNC, "B3 -- meme chemin que fsync"),
    (nr::SYNC, "B3 -- TRANSACTION (SleepMutex) + FS"),
    (nr::MSYNC, "B3 -- mm + CACHE partage + FS"),
    (nr::MREMAP, "B3 -- composition de mmap/munmap/user_read/user_write, deja hors BKL"),
    (nr::FCHMOD, "constante : 0, pas de modes de fichier honores"),
    (nr::FCHOWN, "constante : 0, pas de proprietaires honores"),
    (nr::CHMOD, "constante : 0, pas de modes de fichier honores"),
    (nr::CHOWN, "constante : 0, pas de proprietaires honores"),
    // B4 -- readiness : le domaine de `poll`, deja sorti.
    //
    // Audit. `epoll_create` : table des descripteurs seule. `select` /
    // `pselect6` et `epoll_wait` / `epoll_pwait` : `user_read`/`user_write`
    // (mm), `readable`/`writable` (verrou de chaque objet ; pour un socket, le
    // gros verrou est repris LOCALEMENT, comme sous `poll`), ticket et attente
    // de readiness (`fd::readiness_ticket` / `wait_readiness`, deja utilises
    // par `poll` hors gros verrou).
    //
    // Correction prealable (BOUCHAUD_EPOLL_SANS_VERROU_TENU_V1) :
    // `epoll_wait` evaluait `readable` et `user_write` EN TENANT le SpinLock de
    // la liste epoll. Sous le gros verrou de l'aiguilleur, la reprise locale du
    // gros verrou par un socket etait une reentrance sans effet ; hors gros
    // verrou, c'eut ete se garer SOUS un SpinLock. La liste est desormais
    // copiee, le verrou rendu, puis evaluee.
    (nr::EPOLL_CREATE, "B4 -- table des descripteurs seule"),
    (nr::EPOLL_CREATE1, "B4 -- table des descripteurs seule"),
    (nr::EPOLL_WAIT, "B4 -- liste copiee sous son verrou ; readiness comme poll ; mm"),
    (nr::EPOLL_PWAIT, "B4 -- meme chemin qu'epoll_wait"),
    (nr::SELECT, "B4 -- readiness comme poll ; mm"),
    (nr::PSELECT6, "B4 -- meme chemin que select"),
    // B5 -- le systeme de fichiers.
    //
    // Mesure (endurance SMP4, [BKL-INVENTAIRE]) : open=157 acquisitions ;
    // unlink, stat, ftruncate : quelques unites. Faible cout, mais c'est la
    // famille qui portait le plus de courses MASQUEES par le gros verrou.
    //
    // Audit. Le RAMFS vit sous `fs()` (RankedSpinLock, classe Vfs) ; les
    // etendues disque sous EXTENTS (SpinLock, pris sous FS, jamais l'inverse) ;
    // cwd et ecran sous `metadata` (pris AVANT FS, ordre de `resolve`) ; la
    // table des descripteurs sous `files` (jamais tenue en prenant FS pour une
    // modification) ; la politique de securite sous CONTEXTS (SpinLock) ; la
    // memoire utilisateur sous `mm`.
    //
    // Corrections prealables, toutes des courses que le gros verrou masquait
    // (BOUCHAUD_VFS_UNE_SEULE_PRISE_V1, BOUCHAUD_PREAD_POSITIONNE_V1) :
    //  * `unlinkat` resolvait sous une prise de FS et retirait sous une autre :
    //    un index recycle entre les deux faisait retirer un autre fichier ;
    //  * `rename` : meme defaut entre la resolution de la source et le
    //    deplacement ;
    //  * `open(O_CREAT)` concurrent sur un meme nom : le second recevait
    //    EEXIST ; le nom est revu sous la prise qui cree ;
    //  * `pread`/`pwrite` deplacaient le decalage PARTAGE du descripteur le
    //    temps de l'appel ; ils lisent/ecrivent desormais a la position
    //    demandee sans y toucher (POSIX), ESPIPE sur tube et socket.
    // Les effets de bord a l'ouverture d'un peripherique ont recu leur propre
    // verrou plutot qu'un gros verrou local (le budget « sites BKL / Pilote =
    // 0 » l'interdit, a raison) : bascule de l'affichage sous BASCULE (test et
    // bascule sous la meme prise), armement PS/2 sous ARMEMENT. `ioctl` reste
    // sous gros verrou : chantier des pilotes.
    (nr::OPEN, "B5 -- VFS : FS + descripteurs + metadata ; creation revue sous la prise qui cree"),
    (nr::OPENAT, "B5 -- meme chemin qu'open ; affichage sous BASCULE, souris sous ARMEMENT"),
    (nr::ACCESS, "B5 -- resolution sous FS"),
    (nr::FACCESSAT, "B5 -- resolution sous FS"),
    (nr::STAT, "B5 -- resolution sous FS + mm"),
    (nr::LSTAT, "B5 -- resolution sous FS + mm"),
    (nr::NEWFSTATAT, "B5 -- fstat ou stat, deja audites + mm"),
    (nr::STATX, "B5 -- descripteurs + FS + EXTENTS + mm"),
    (nr::READLINK, "B5 -- metadata du processus + mm"),
    (nr::READLINKAT, "B5 -- metadata du processus + mm"),
    (nr::GETDENTS64, "B5 -- descripteurs + FS ; copie sous FS, ecriture utilisateur apres"),
    (nr::GETCWD, "B5 -- metadata + FS + mm"),
    (nr::CHDIR, "B5 -- metadata + FS"),
    (nr::MKDIR, "B5 -- une seule prise de FS"),
    (nr::MKDIRAT, "B5 -- une seule prise de FS"),
    (nr::UNLINK, "B5 -- resolution ET retrait sous une seule prise de FS"),
    (nr::UNLINKAT, "B5 -- resolution ET retrait sous une seule prise de FS"),
    (nr::RENAME, "B5 -- source, cible et deplacement sous une seule prise de FS"),
    (nr::STATFS, "B5 -- resolution sous FS + mm"),
    (nr::FSTATFS, "B5 -- descripteurs + mm"),
    (nr::FTRUNCATE, "B5 -- descripteurs + EXTENTS + FS"),
    (nr::PREAD64, "B5 -- lecture positionnee, sans toucher au decalage partage"),
    (nr::PWRITE64, "B5 -- ecriture positionnee sous une seule prise de FS"),
    (nr::SENDFILE, "B5 -- descripteurs + backing + chemin d'ecriture deja audite"),
    (nr::MEMFD_CREATE, "B5 -- FS + descripteurs + mm"),
    // B6 -- les signaux.
    //
    // Mesure (endurance SMP4) : rt_sigprocmask=304 acquisitions, 52 ms tenus,
    // 133 ms d'ATTENTE -- l'appel le plus attendu apres fcntl et exit_group.
    //
    // Audit. L'etat de signaux vit dans `process.signals` (verrou propre) ;
    // les alarmes dans ALARMES (SpinLock) ; la recherche d'un processus dans
    // PROCESSES (SpinLock), d'une tache dans le registre en lecture ; le
    // reveil d'une tache endormie est le CAS Blocked -> Ready de
    // `wake_for_signal` (BOUCHAUD_REVEIL_SANS_EFFACER_LA_CLE_V1) ; l'attente de
    // `sigsuspend`/`pause` est `wait_for_interrupt_releasing_bkl`, dont la
    // comptabilite lit le registre et non le gros verrou (commentaire corrige).
    //
    // Corrections prealables (BOUCHAUD_SIGNAUX_UNE_SEULE_PRISE_V1) :
    // `rt_sigprocmask` lisait le masque sous une prise et l'ecrivait sous une
    // autre, calcule sur la copie perimee (mise a jour perdue entre deux fils) ;
    // `rt_sigaction` rendait comme « ancienne » une action lue sous une autre
    // prise que le remplacement. Les deux calculent et ecrivent desormais sous
    // une seule prise, la memoire utilisateur etant lue avant et ecrite apres.
    (nr::RT_SIGACTION, "B6 -- signals du processus, lecture/remplacement sous une prise ; mm hors verrou"),
    (nr::RT_SIGPROCMASK, "B6 -- masque calcule et ecrit sous une seule prise de signals ; mm hors verrou"),
    (nr::RT_SIGPENDING, "B6 -- signals + mm"),
    (nr::RT_SIGRETURN, "B6 -- trame restauree depuis la pile utilisateur (mm) + signals"),
    (nr::RT_SIGSUSPEND, "B6 -- signals + attente d'interruption (registre en lecture, pas de BKL)"),
    (nr::PAUSE, "B6 -- meme chemin que sigsuspend"),
    (nr::KILL, "B6 -- PROCESSES (SpinLock) + signals + reveil par CAS Blocked->Ready"),
    (nr::TKILL, "B6 -- registre des taches en lecture + meme chemin que kill"),
    (nr::TGKILL, "B6 -- meme chemin que kill"),
    (nr::ALARM, "B6 -- ALARMES (SpinLock) + ticks atomiques"),
    (nr::GETITIMER, "B6 -- ALARMES + mm"),
    (nr::SETITIMER, "B6 -- ALARMES + mm"),
    // B7 -- le reseau (BOUCHAUD_RESEAU_SANS_BKL_V1).
    //
    // Mesure (smoke Ladybird #373) : [BKL-MAX-HOLD] 25 611 ms sur connect(42)
    // -- la poignee TCP attendait le SYN-ACK en boucle active, gros verrou
    // tenu, pendant que `sendto` et `rt_sigprocmask` d'autres taches
    // attendaient derriere.
    //
    // Audit. La pile inet n'a plus d'etat que seul le gros verrou protege :
    // anneau RX, routage des trames et cache ARP sous VERROU_RECEPTION
    // (SpinLockIrq) ; anneau TX sous ANNEAU_TX ; etat de chaque socket et
    // connexion TCP sous le SpinLock de son SocketState ; port ephemere et
    // identifiant IP atomiques (BOUCHAUD_PORT_EPHEMERE_MONOTONE_V1,
    // PROCHAIN_IP_ID) ; cache DNS du resolveur noyau sous SpinLock. La
    // frontiere `avec_domaine_reseau` et les deux branches socket de `file.rs`
    // ne prennent plus le gros verrou, et le domaine `Reseau` est declare
    // `Migre` : toute reprise serait une regression comptee (budget zero).
    // `listen`/`accept`/`accept4` rendent ENOSYS. `TcpConn::connect` attend
    // desormais avec des points surs reguliers (meme patience, memes SYN).
    (nr::CONNECT, "B7 -- SocketState + TcpConn (poll_ip sous VERROU_RECEPTION, send_ip sous ANNEAU_TX) ; attente avec points surs"),
    (nr::SENDTO, "B7 -- envoie_octets : SocketState + send_ip (ANNEAU_TX, ARP) + port ephemere atomique"),
    (nr::SENDMSG, "B7 -- descripteurs + mm + envoie_octets deja audite"),
    (nr::SENDMMSG, "B7 -- boucle SENDMSG + mm"),
    (nr::SHUTDOWN, "B7 -- SocketState seul"),
    (nr::GETSOCKNAME, "B7 -- SocketState + adresse locale + mm"),
    (nr::LISTEN, "B7 -- sys_listen_unsupported : rend -ENOSYS sans rien lire"),
    (nr::ACCEPT, "B7 -- sys_listen_unsupported : rend -ENOSYS sans rien lire"),
    (nr::ACCEPT4, "B7 -- sys_listen_unsupported : rend -ENOSYS sans rien lire"),
    // B8 -- le cycle de vie des processus.
    //
    // Mesure (endurance SMP4, [BKL-INVENTAIRE]) : exit_group=65/419 ms tenus/
    // 158 ms d'attente, exit=28/173/44, fork=50/89/2, wait4=39/5/105.
    //
    // Audit. Chaque etat a son verrou : registre des taches (le sien, seule
    // section critique de `register`, qui ne prend plus le gros verrou) ;
    // PROCESSES (RankedSpinLock, classe ProcessTable) ; par processus, `mm`,
    // `files`, `metadata`, `signals` et `lifecycle` ; identifiants de fil
    // atomiques. `fork` prend ses instantanes sous ces verrous (duplication de
    // l'espace et champs de `mm` sous la MEME prise). `execve` attendait deja
    // la quiescence gros verrou rendu.
    //
    // Ce que le gros verrou serialisait et que `lifecycle` serialise desormais
    // (BOUCHAUD_GROUPE_RECLAME_V1, BOUCHAUD_WAIT4_RECOLTE_UNIQUE_V1) :
    //  * deux `exit_group`/`execve` concurrents se tuaient mutuellement et
    //    demontaient le processus deux fois : le premier fil qui RECLAME le
    //    groupe gagne, les autres se retirent comme les fils tues par execve ;
    //  * un `clone` pouvait enregistrer un fil apres le balayage et survivre a
    //    la sortie du groupe : il relit la reclamation apres l'enregistrement ;
    //  * un fil deja compte sorti pouvait retrouver `threads == 0` apres la
    //    remise a 1 d'`exit_group` : une seule transition, tranchee par
    //    `zombie` sous `lifecycle` ;
    //  * deux `wait4` du meme parent pouvaient recolter le meme zombie : seul
    //    celui qui le retire de PROCESSES le rend.
    (nr::CLONE, "B8 -- fork (ci-dessous) ou fil : lifecycle + registre ; groupe reclame relu apres enregistrement"),
    (nr::CLONE3, "B8 -- meme chemin que clone"),
    (nr::FORK, "B8 -- instantanes sous mm/files/metadata/signals ; PROCESSES + registre sous leurs verrous"),
    (nr::VFORK, "B8 -- meme chemin que fork"),
    (nr::EXECVE, "B8 -- groupe reclame sous lifecycle avant le point de non-retour ; quiescence deja hors BKL"),
    (nr::EXIT, "B8 -- lifecycle ; une seule transition vers le dernier fil"),
    (nr::EXIT_GROUP, "B8 -- groupe reclame sous lifecycle ; un seul demontage"),
    (nr::WAIT4, "B8 -- PROCESSES + lifecycle ; recolte unique sous le verrou de PROCESSES"),
    // B9 -- `ioctl`, le dernier appel reel sous le gros verrou.
    //
    // Audit, branche par branche :
    //  * audio (OSS) : l'etat du pilote AC97 etait treize `static mut` ; il est
    //    sous un SleepMutex (BOUCHAUD_AC97_VERROU_V1), chaque fonction prend le
    //    verrou une fois, et le reglage du format lit et ecrit sous la meme
    //    prise (un SPEED concurrent n'est plus perdu) ;
    //  * console, VT : constantes, et `gfx::is_active` devenu atomique
    //    (BOUCHAUD_GFX_DRAPEAUX_ATOMIQUES_V1) ;
    //  * framebuffer : `metadata` du processus (ecran virtuel), geometrie
    //    atomique, `lfb_phys` atomique ;
    //  * evdev : tables constantes du pilote d'entree ;
    //  * FIONBIO/FIONREAD : table des descripteurs, tube/paire sous leur
    //    verrou, prise inet sous SocketState (domaine Reseau, deja Migre).
    (nr::IOCTL, "B9 -- AC97 sous SleepMutex ; drapeaux gfx atomiques ; metadata ; files ; SocketState"),
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
