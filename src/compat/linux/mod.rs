//! ABI POSIX/Linux x86-64 : le contrat que voit une libc.
//!
//! Les numeros, les structures et les codes d'erreur sont **ceux de Linux
//! x86-64**, sans adaptation : c'est la condition pour executer un `musl`
//! statique non modifie, puis `ld.so`, puis une pile Qt. Un binaire compile
//! avec `musl-gcc` sur une machine Linux doit tourner ici sans recompilation.
//!
//! Le dispatch est appele par le stub d'entree (`arch::x86_64::usermode`) avec
//! la [`TrapFrame`] de la tache : `rax` porte le numero, `rdi/rsi/rdx/r10/r8/r9`
//! les arguments, et la valeur de retour repart dans `rax` — negative pour une
//! erreur (`-ENOENT`, ...), exactement comme Linux.
//!
//! Ce module est decoupe en trois : les numeros et constantes ([`nr`], [`errno`]),
//! les entrees/sorties sur descripteurs ([`file`]), et la memoire du processus
//! ([`mem`]).
//!
//! Process ownership is SMP-safe independently of the BKL: memory, descriptor,
//! signal, metadata and lifecycle state have distinct synchronization domains.
//! No domain guard may cross a blocking or scheduling boundary.

pub mod bkl;
pub mod errno;
pub mod file;
pub mod mem;
pub mod net;
pub mod nr;
pub mod proc;
pub mod verrous;

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use crate::arch::x86_64::usermode::{self, TrapFrame};
use crate::kernel::task;

/// Compteur d'appels systeme, pour le diagnostic.
static SYSCALL_COUNT: AtomicU64 = AtomicU64::new(0);
/// Compteur PAR appel systeme.
///
/// Sortir un appel du gros verrou coute une preuve ; la depenser sur un appel
/// que personne n'emet ne fait rien gagner. Le premier lot A1 l'a montre en
/// chiffres : vingt-trois appels liberes, aucun gain mesurable, parce qu'ils
/// etaient rares. Choisir les suivants demande de savoir lesquels sont chauds,
/// et le savoir demande de compter.
///
/// Un `fetch_add` relaxe sur un tableau indexe par le numero : pas de verrou,
/// pas d'allocation, rien qui se voie dans une mesure. Les numeros Linux
/// x86-64 utilises par ce noyau vont jusqu'a 334 (`rseq`) ; au-dela, l'appel
/// est compte dans la derniere case, qui vaut « hors table ».
const SYSCALL_HITS_LEN: usize = 336;
static SYSCALL_HITS: [AtomicU32; SYSCALL_HITS_LEN] =
    [const { AtomicU32::new(0) }; SYSCALL_HITS_LEN];
/// Dernier appel systeme inconnu rencontre (numero), 0 si aucun.
static LAST_UNKNOWN: AtomicU64 = AtomicU64::new(0);
/// Ce que la trace des appels systeme laisse passer (commande `strace`).
///
/// `Echecs` existe parce que `Tous` est inutilisable sur un vrai programme :
/// un navigateur emet des millions d'appels, la sortie serie devient le facteur
/// limitant et le journal noie ce qu'on cherche. Or ce qu'on cherche est
/// presque toujours un appel qui a ECHOUE -- c'est ainsi qu'on relie un
/// « disk I/O error » cote application a la primitive OS qui manque.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Trace {
    Aucune,
    /// Seuls les appels qui rendent une erreur, hors attente ordinaire.
    Echecs,
    Tous,
}

static TRACE: AtomicU8 = AtomicU8::new(0);

/// Regle la trace des appels systeme.
pub fn set_trace_mode(mode: Trace) {
    TRACE.store(mode as u8, Ordering::Release);
}

/// Active ou desactive la trace complete.
pub fn set_trace(on: bool) {
    set_trace_mode(if on { Trace::Tous } else { Trace::Aucune });
}

/// Mode de trace courant.
pub fn trace_mode() -> Trace {
    match TRACE.load(Ordering::Acquire) {
        1 => Trace::Echecs,
        2 => Trace::Tous,
        _ => Trace::Aucune,
    }
}

/// La trace est-elle active, sous une forme ou une autre ?
pub fn trace_enabled() -> bool {
    trace_mode() != Trace::Aucune
}

/// Ce retour merite-t-il d'etre trace en mode `Echecs` ?
///
/// `EAGAIN` et `EINTR` ne sont pas des defaillances : ce sont les reponses
/// ordinaires d'une entree/sortie non bloquante et d'un appel interrompu par un
/// signal. Un navigateur en produit des milliers par seconde ; les tracer
/// reviendrait a retrouver le probleme du mode `Tous`.
fn echec_notable(resultat: i64) -> bool {
    resultat < 0 && resultat != -errno::EAGAIN && resultat != -errno::EINTR
}

/// Nombre d'appels systeme traites depuis le boot.
pub fn syscall_count() -> u64 {
    SYSCALL_COUNT.load(Ordering::Relaxed)
}

/// Dernier numero d'appel systeme non implemente (0 si aucun).
pub fn last_unknown() -> u64 {
    LAST_UNKNOWN.load(Ordering::Relaxed)
}

/// Point d'entree du dispatch : lit la trame, execute, ecrit le retour.
pub fn handle(frame: &mut TrapFrame) {
    SYSCALL_COUNT.fetch_add(1, Ordering::Relaxed);
    let (number, args) = frame.syscall_args();
    SYSCALL_HITS[(number as usize).min(SYSCALL_HITS_LEN - 1)].fetch_add(1, Ordering::Relaxed);
    let result = dispatch(number, args, frame);
    match trace_mode() {
        Trace::Aucune => {}
        Trace::Tous => {
            crate::serial_println!(
                "[syscall] {} ({}) ({:#x}, {:#x}, {:#x}) = {}",
                number,
                nr::name(number),
                args[0],
                args[1],
                args[2],
                result
            );
        }
        Trace::Echecs => {
            if echec_notable(result) {
                crate::serial_println!(
                    "[syscall-echec] {} ({}) ({:#x}, {:#x}, {:#x}) = {} ({})",
                    number,
                    nr::name(number),
                    args[0],
                    args[1],
                    args[2],
                    result,
                    errno::name(-result)
                );
            }
        }
    }
    // `rt_sigreturn` a deja reecrit toute la trame : y remettre une valeur de
    // retour ecraserait le rax restaure.
    if number != nr::RT_SIGRETURN {
        frame.rax = result as u64;
    }

    // Preemption differee : le timer a pu marquer un besoin de commutation
    // pendant l'appel, ou l'appel a pu reveiller une tache plus prioritaire.
    if task::take_need_resched() {
        task::yield_now();
    }

    // Dernier moment ou la trame ring 3 est modifiable : c'est ici que se
    // livrent les signaux en attente.
    proc::deliver_pending(frame);
}

/// `rt_sigsuspend` / `pause` : attend qu'un signal arrive.
fn sys_sigsuspend(set: u64) -> i64 {
    let process = task::current_process();
    let saved = process.signals.lock().blocked;
    if set != 0 {
        if let Some(mask) = user_read_u64(set) {
            process.signals.lock().blocked = mask & !(1 << (crate::kernel::signal::SIGKILL - 1));
        }
    }
    while !task::signal_pending() {
        task::yield_now();
        task::wait_for_interrupt_releasing_bkl();
    }
    process.signals.lock().blocked = saved;
    // POSIX impose ce retour : l'attente s'est terminee par un signal.
    -errno::EINTR
}

/// `setitimer` / `getitimer` : `struct itimerval` = periode puis echeance,
/// chacune en `struct timeval` (secondes puis microsecondes).
///
/// C'est par la que passe `alarm()` de musl — pas par l'appel `alarm`, qui
/// n'est donc jamais emis en pratique.
fn sys_setitimer(which: u32, new_value: u64, old_value: u64) -> i64 {
    const ITIMER_REAL: u32 = 0;
    if which != ITIMER_REAL {
        // Les minuteurs de temps CPU demanderaient une comptabilite par
        // processus que l'ordonnanceur ne tient pas.
        return -errno::ENOSYS;
    }
    if old_value != 0 {
        sys_getitimer(which, old_value);
    }
    if new_value == 0 {
        return 0;
    }
    let seconds = user_read_u64(new_value + 16).unwrap_or(0);
    let micros = user_read_u64(new_value + 24).unwrap_or(0);
    let deadline = if seconds == 0 && micros == 0 {
        0 // desarmement
    } else {
        crate::kernel::timer::ticks()
            + crate::kernel::timer::ms_to_ticks(seconds * 1000 + micros / 1000).max(1)
    };
    task::set_alarm(deadline);
    0
}

fn sys_getitimer(which: u32, out: u64) -> i64 {
    const ITIMER_REAL: u32 = 0;
    if which != ITIMER_REAL || out == 0 {
        return -errno::EINVAL;
    }
    let deadline = task::peek_alarm();
    let remaining_ticks = deadline.saturating_sub(crate::kernel::timer::ticks());
    let per_second = crate::kernel::timer::TICKS_PER_SECOND;
    // it_interval reste nul : les alarmes ne se repetent pas.
    user_write(out, &0u64.to_le_bytes());
    user_write(out + 8, &0u64.to_le_bytes());
    user_write(out + 16, &(remaining_ticks / per_second).to_le_bytes());
    user_write(out + 24, &((remaining_ticks % per_second) * (1_000_000 / per_second)).to_le_bytes());
    0
}

/// `alarm` : programme un `SIGALRM`.
fn sys_alarm(seconds: u32) -> i64 {
    let previous = task::set_alarm(if seconds == 0 {
        0
    } else {
        crate::kernel::timer::ticks() + seconds as u64 * crate::kernel::timer::TICKS_PER_SECOND
    });
    if previous == 0 {
        0
    } else {
        let now = crate::kernel::timer::ticks();
        (previous.saturating_sub(now) / crate::kernel::timer::TICKS_PER_SECOND) as i64
    }
}

/// Rend residente une plage utilisateur avant une copie noyau <-> userland.
///
/// Un page fault materiel n'a lieu que lorsque le CPU dereference directement
/// l'adresse virtuelle. `AddressSpace::read/write`, eux, traduisent les pages
/// manuellement : une page seulement promisee leur apparaissait donc absente et
/// les syscalls rendaient EFAULT. Copyin/copyout doit resoudre le meme backing
/// paresseux que le gestionnaire #PF.
///
/// `write=true` impose en plus PTE_WRITE : le noyau ne doit pas permettre a un
/// syscall d'ecrire dans une page que le processus lui-meme voit read-only.
/// Le processus courant, sans le gros verrou quand c'est possible.
///
/// `task::current_process()` prend le gros verrou parce qu'il passe par
/// `task::current()`, donc par la table des taches. Le domaine CPU-local
/// (`task::current_process_local`) rend le meme `Arc` sans rien verrouiller ;
/// il ne rend `None` que pour un fil noyau, cas ou l'on retombe sur le chemin
/// historique. Aucun comportement ne change : c'est le meme processus.
pub(crate) fn processus_courant() -> alloc::sync::Arc<task::Process> {
    match task::current_process_local() {
        Some(process) => process,
        None => task::current_process(),
    }
}

fn fault_in_user_range(addr: u64, len: usize, write: bool) -> bool {
    if len == 0 {
        return true;
    }

    let last = match addr.checked_add(len as u64 - 1) {
        Some(last) => last,
        None => return false,
    };

    if !crate::kernel::vmm::is_user_addr(addr)
        || !crate::kernel::vmm::is_user_addr(last)
    {
        return false;
    }

    let page_size = crate::kernel::vmm::PAGE_SIZE;
    let mut page = addr & !(page_size - 1);
    let last_page = last & !(page_size - 1);

    loop {
        let present = {
            let process = processus_courant();
            let present = process.mm.lock().space.translate(page).is_some();
            present
        };

        if !present && task::peuple_a_la_demande(page, false) != task::FaultOutcome::Resolved {
            return false;
        }

        if write {
            let writable = {
                let process = processus_courant();
                let writable = process.mm.lock().space.writable(page);
                writable
            };
            if !writable {
                return false;
            }
        }

        if page == last_page {
            break;
        }
        page += page_size;
    }

    true
}

/// Copie une chaine C depuis l'espace utilisateur.
///
/// On avance page par page : demander arbitrairement 4096 octets d'avance
/// rejetterait une chaine parfaitement valide terminee juste avant une page
/// non mappee.
pub(crate) fn user_string(addr: u64) -> Option<String> {
    if addr == 0 {
        return None;
    }

    const MAX: usize = 4096;
    let page_size = crate::kernel::vmm::PAGE_SIZE as usize;
    let mut bytes = Vec::new();
    let mut cursor = addr;

    while bytes.len() < MAX {
        let in_page = (cursor as usize) & (page_size - 1);
        let room = page_size - in_page;
        let wanted = core::cmp::min(room, MAX - bytes.len());

        let chunk = user_read(cursor, wanted)?;
        if let Some(end) = chunk.iter().position(|&byte| byte == 0) {
            bytes.extend_from_slice(&chunk[..end]);
            return Some(String::from_utf8_lossy(&bytes).into_owned());
        }

        bytes.extend_from_slice(&chunk);
        cursor = cursor.checked_add(wanted as u64)?;
    }

    None
}

/// Copie un tampon depuis l'espace utilisateur.
pub fn user_read(addr: u64, len: usize) -> Option<Vec<u8>> {
    if !fault_in_user_range(addr, len, false) {
        return None;
    }

    let mut buffer = alloc::vec![0u8; len];
    let process = processus_courant();
    if process.mm.lock().space.read(addr, &mut buffer) {
        Some(buffer)
    } else {
        None
    }
}

/// Copie un tampon vers l'espace utilisateur.
pub fn user_write(addr: u64, data: &[u8]) -> bool {
    if !fault_in_user_range(addr, data.len(), true) {
        return false;
    }

    let process = processus_courant();
    let written = process.mm.lock().space.write(addr, data);
    written
}

/// Ecrit une valeur 64 bits dans l'espace utilisateur.
pub fn user_write_u64(addr: u64, value: u64) -> bool {
    user_write(addr, &value.to_le_bytes())
}

/// Ecrit une valeur 32 bits dans l'espace utilisateur.
pub fn user_write_u32(addr: u64, value: u32) -> bool {
    user_write(addr, &value.to_le_bytes())
}

/// Lit une valeur 32 bits depuis l'espace utilisateur.
///
/// Symetrique de [`user_write_u32`] : les ioctls qui echangent un entier — le
/// reglage d'un flux audio, par exemple — lisent la demande puis reecrivent au
/// meme endroit ce que le pilote a reellement retenu.
pub fn user_read_u32(addr: u64) -> Option<u32> {
    let bytes = user_read(addr, 4)?;
    let mut value = [0u8; 4];
    value.copy_from_slice(&bytes);
    Some(u32::from_le_bytes(value))
}

/// Lit une valeur 64 bits depuis l'espace utilisateur.
pub fn user_read_u64(addr: u64) -> Option<u64> {
    let bytes = user_read(addr, 8)?;
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes);
    Some(u64::from_le_bytes(value))
}

/// Aiguillage principal.
fn dispatch(number: u64, args: [u64; 6], frame: &mut TrapFrame) -> i64 {
    use nr::*;
    match number {
        // --- Entrees / sorties ---
        READ => file::sys_read(args[0] as i32, args[1], args[2] as usize),
        WRITE => file::sys_write(args[0] as i32, args[1], args[2] as usize),
        OPEN => file::sys_openat(file::AT_FDCWD, args[0], args[1] as u32, args[2] as u32),
        OPENAT => file::sys_openat(args[0] as i32, args[1], args[2] as u32, args[3] as u32),
        CLOSE => file::sys_close(args[0] as i32),
        LSEEK => file::sys_lseek(args[0] as i32, args[1] as i64, args[2] as u32),
        READV => file::sys_readv(args[0] as i32, args[1], args[2] as usize),
        WRITEV => file::sys_writev(args[0] as i32, args[1], args[2] as usize),
        PREAD64 => file::sys_pread(args[0] as i32, args[1], args[2] as usize, args[3] as i64),
        PWRITE64 => file::sys_pwrite(args[0] as i32, args[1], args[2] as usize, args[3] as i64),
        SENDFILE => file::sys_sendfile(args[0] as i32, args[1] as i32, args[2], args[3] as usize),
        STAT => file::sys_stat_path(args[0], args[1], false),
        LSTAT => file::sys_stat_path(args[0], args[1], true),
        FSTAT => file::sys_fstat(args[0] as i32, args[1]),
        NEWFSTATAT => file::sys_newfstatat(args[0] as i32, args[1], args[2], args[3] as u32),
        STATX => file::sys_statx(args[0] as i32, args[1], args[2] as u32, args[3] as u32, args[4]),
        ACCESS => file::sys_access(args[0], args[1] as u32),
        FACCESSAT => file::sys_access(args[1], args[2] as u32),
        READLINK => file::sys_readlink(args[0], args[1], args[2] as usize),
        READLINKAT => file::sys_readlink(args[1], args[2], args[3] as usize),
        GETDENTS64 => file::sys_getdents64(args[0] as i32, args[1], args[2] as usize),
        GETCWD => file::sys_getcwd(args[0], args[1] as usize),
        CHDIR => file::sys_chdir(args[0]),
        MKDIR => file::sys_mkdir(args[0]),
        MKDIRAT => file::sys_mkdir(args[1]),
        UNLINK => file::sys_unlink(args[0]),
        UNLINKAT => file::sys_unlink(args[1]),
        RENAME => file::sys_rename(args[0], args[1]),
        // Le RAMFS actuel fusionne entree de repertoire et inode : il ne peut
        // pas representer deux noms pointant vers le meme inode sans refonte
        // de son modele. Retourner succes ou copier le contenu serait un faux
        // hardlink et casserait les garanties POSIX.
        LINK => -errno::ENOTSUP,
        // La surveillance de fichiers n'est pas encore un service noyau
        // Bouchaud. ENOSYS est intentionnel : les bibliotheques retombent sur
        // leur chemin sans surveillance (ex. timezone) au lieu de croire que
        // l'abonnement existe.
        INOTIFY_INIT1 => -errno::ENOSYS,
        STATFS => file::sys_statfs(args[0], args[1]),
        FSTATFS => file::sys_fstatfs(args[0] as i32, args[1]),
        FTRUNCATE => file::sys_ftruncate(args[0] as i32, args[1] as usize),
        DUP => file::sys_dup(args[0] as i32),
        DUP2 => file::sys_dup2(args[0] as i32, args[1] as i32),
        DUP3 => file::sys_dup2(args[0] as i32, args[1] as i32),
        PIPE => file::sys_pipe(args[0], 0),
        PIPE2 => file::sys_pipe(args[0], args[1] as u32),
        FCNTL => file::sys_fcntl(args[0] as i32, args[1] as u32, args[2]),
        IOCTL => file::sys_ioctl(args[0] as i32, args[1] as u64, args[2]),
        POLL => file::sys_poll(args[0], args[1] as usize, args[2] as i32),
        // `ppoll(fds, n, tmo_p, sigmask, sigsetsize)` : le delai est un
        // **pointeur sur timespec**, pas un nombre de millisecondes. Le prendre
        // pour une duree faisait attendre indefiniment toute boucle
        // d'evenements qui passe par `ppoll` — c'est le cas de Qt via la glibc,
        // et ses minuteries ne se declenchaient jamais.
        PPOLL => {
            let timeout = if args[2] == 0 {
                -1 // pointeur nul : attente sans limite
            } else {
                match timespec_ms(args[2]) {
                    Some(ms) => ms.min(i32::MAX as u64) as i32,
                    None => return -errno::EFAULT,
                }
            };
            file::sys_poll(args[0], args[1] as usize, timeout)
        }
        SELECT | PSELECT6 => file::sys_select(args[0] as i32, args[1], args[2], args[3], args[4]),
        EPOLL_CREATE | EPOLL_CREATE1 => file::sys_epoll_create(),
        EPOLL_CTL => file::sys_epoll_ctl(args[0] as i32, args[1] as u32, args[2] as i32, args[3]),
        EPOLL_WAIT | EPOLL_PWAIT => {
            file::sys_epoll_wait(args[0] as i32, args[1], args[2] as usize, args[3] as i32)
        }
        EVENTFD => file::sys_eventfd(args[0] as u32, 0),
        EVENTFD2 => file::sys_eventfd(args[0] as u32, args[1] as u32),
        TIMERFD_CREATE => file::sys_timerfd_create(args[1] as u32),
        TIMERFD_SETTIME => file::sys_timerfd_settime(args[0] as i32, args[1] as u32, args[2], args[3]),
        TIMERFD_GETTIME => file::sys_timerfd_gettime(args[0] as i32, args[1]),
        // `fsync` sur un fichier persistant ecrit reellement la zone du
        // disque ; ailleurs il ne coute rien, car un programme en emet sans
        // compter et le RAMFS n'a rien a vider.
        MEMFD_CREATE => file::sys_memfd_create(args[0], args[1] as u32),
        FSYNC | FDATASYNC => {
            let noeud = match crate::kernel::task::current_process()
                .files.lock()
                .get(args[0] as i32)
                .map(|desc| desc.kind.clone())
            {
                Some(crate::kernel::fd::FdKind::File(idx)) => Some(idx),
                _ => None,
            };
            match noeud {
                Some(idx) if crate::fs::persistance::sous_racine(idx) => {
                    if crate::fs::persistance::synchronise() < 0 { -5 } else { 0 }
                }
                _ => 0,
            }
        }
        // `sync` ecrit tout ce qui doit survivre a l'extinction.
        SYNC => {
            if crate::fs::persistance::synchronise() < 0 { -5 } else { 0 }
        }
        UMASK => 0o022,
        FCHMOD | FCHOWN | CHMOD | CHOWN => 0,

        // --- Memoire ---
        BRK => mem::sys_brk(args[0]),
        MMAP => mem::sys_mmap(args[0], args[1], args[2] as u32, args[3] as u32, args[4] as i32, args[5]),
        MUNMAP => mem::sys_munmap(args[0], args[1]),
        MPROTECT => mem::sys_mprotect(args[0], args[1], args[2] as u32),
        MREMAP => mem::sys_mremap(args[0], args[1], args[2], args[3] as u32),
        MSYNC => {
            // Recopie les pages partagees vers le fichier : sans cela, un
            // MAP_SHARED ne serait visible que des autres mappings, pas d'un
            // `read` ordinaire.
            mem::sys_msync(args[0], args[1]);
            0
        }
        MADVISE => mem::sys_madvise(args[0], args[1], args[2] as i32),
        MLOCK | MUNLOCK | MLOCKALL | MUNLOCKALL => 0,

        // --- Processus, threads, ordonnancement ---
        // --- Identite : servie par le domaine CPU-local, sans le gros verrou --
        //
        // `task::identite_courante()` rend une COPIE (un entier, une part
        // d'`Arc`) lue dans le bloc par-CPU adresse par `GS` et dans
        // `CURRENT_PROCESS[cpu]`, tous deux publies par `install` sous le gros
        // verrou. La table des taches n'est pas touchee. Voir la preuve de
        // duree de vie sur `identite_courante`.
        //
        // Le repli sur `task::current()` couvre le fil noyau, qui n'a pas
        // d'identite utilisateur publiee ; aucun appel systeme ne vient de la,
        // mais rendre une valeur fausse serait pire que de reprendre le verrou.
        GETPID => match task::identite_courante() {
            Some(identite) => identite.process.pid as i64,
            None => task::current_process().pid as i64,
        },
        GETPPID => 1,
        GETTID => match task::identite_courante() {
            Some(identite) => identite.tid as i64,
            None => task::current().tid as i64,
        },
        SET_TID_ADDRESS => {
            task::current().clear_child_tid = args[0];
            task::current().tid as i64
        }
        // `set_robust_list` peut legitimement etre accepte sans effet : la glibc
        // s'en sert pour nettoyer les verrous d'un fil mort, ce qui degrade
        // proprement quand personne ne le fait.
        SET_ROBUST_LIST | GET_ROBUST_LIST => 0,
        // `rseq`, lui, doit etre **refuse**.
        //
        // Repondre « reussi » a ce qu'on n'implemente pas est pire que de
        // l'avouer : la glibc en conclut que sa zone est enregistree, pose son
        // `__rseq_size`, et lit ensuite un `cpu_id` que le noyau ne tient pas a
        // jour. Le chargeur dynamique s'y figeait sans un mot — c'est
        // exactement la ou s'arretait tout binaire glibc du monde reel, apres
        // avoir pourtant mappe la libc, monte le TLS et fait son RELRO.
        //
        // `ENOSYS` la fait retomber proprement sur son chemin sans rseq.
        RSEQ => -errno::ENOSYS,
        CLONE => proc_clone(args, frame),
        CLONE3 => proc_clone3(args, frame),
        // `vfork` partage l'espace d'adressage du parent jusqu'a l'`execve` ;
        // le dupliquer est plus couteux mais toujours correct.
        FORK | VFORK => proc::sys_fork(frame),
        EXECVE => proc::sys_execve(args[0], args[1], args[2]),
        EXIT => task::exit_current(args[0] as i32),
        EXIT_GROUP => task::exit_group(args[0] as i32),
        WAIT4 => proc::sys_wait4(args[0] as i64, args[1], args[2] as u32, args[3]),
        SCHED_YIELD => {
            task::yield_now();
            0
        }
        FUTEX => sys_futex(args),
        NANOSLEEP => sys_nanosleep(args[0], args[1]),
        CLOCK_NANOSLEEP => sys_clock_nanosleep(args[0] as i32, args[1], args[2], args[3]),
        // `metadata` est un verrou a lui, sur le `Process` : son domaine ne
        // depend pas de la table des taches. L'`Arc` rendu par
        // `identite_courante` garantit que le `Process` vit encore.
        GETUID | GETEUID => match task::identite_courante() {
            Some(identite) => identite.process.metadata.lock().uid as i64,
            None => task::current_process().metadata.lock().uid as i64,
        },
        GETGID | GETEGID => match task::identite_courante() {
            Some(identite) => identite.process.metadata.lock().gid as i64,
            None => task::current_process().metadata.lock().gid as i64,
        },
        SETUID | SETGID | SETPGID | SETSID => 0,
        GETPGRP | GETPGID | GETSID => 1,
        SCHED_GETAFFINITY => {
            // Un seul CPU : masque = 1.
            if args[2] != 0 {
                user_write(args[2], &1u64.to_le_bytes());
            }
            8
        }
        SCHED_SETAFFINITY | SCHED_SETSCHEDULER | SCHED_SETPARAM => 0,
        // `setpriority(which, who, nice)` : le seul moyen pour un programme de
        // se declarer interactif. Le noyau n'a que deux classes, mais le signe
        // de `nice` suffit a les distinguer — et un programme portable n'a rien
        // de special a faire, `nice(-5)` marche ici comme ailleurs.
        //
        // Seule la valeur compte, pas la cible : `which`/`who` designent
        // toujours le processus courant faute de groupes de processus reels.
        SETPRIORITY => {
            let gentillesse = args[2] as i32;
            let voulue = if gentillesse < 0 {
                task::Priorite::Interactive
            } else {
                task::Priorite::Normale
            };
            task::pose_priorite(voulue);
            0
        }
        GETPRIORITY => {
            // Linux decale la valeur rendue de 20 pour distinguer une erreur ;
            // musl la remet en place. On rend donc `20 - nice`.
            match task::priorite() {
                task::Priorite::Interactive => 25, // nice = -5
                task::Priorite::Normale => 20,     // nice = 0
            }
        }
        SCHED_GETSCHEDULER => 0,
        SCHED_GET_PRIORITY_MAX | SCHED_GET_PRIORITY_MIN => 0,
        PRCTL => 0,
        ARCH_PRCTL => sys_arch_prctl(args[0] as i32, args[1]),
        // Le numero de signal n'est pas au meme rang selon l'appel :
        // kill(pid, sig), tkill(tid, sig), mais tgkill(tgid, tid, sig).
        KILL => proc::sys_kill(args[0] as i64, args[1] as u32),
        TKILL => proc::sys_tkill(args[0] as u32, args[1] as u32),
        TGKILL => proc::sys_kill(args[0] as i64, args[2] as u32),

        // --- Signaux ---
        RT_SIGACTION => proc::sys_rt_sigaction(args[0] as u32, args[1], args[2]),
        RT_SIGPROCMASK => proc::sys_rt_sigprocmask(args[0] as i32, args[1], args[2]),
        RT_SIGRETURN => proc::sys_rt_sigreturn(frame),
        RT_SIGSUSPEND => sys_sigsuspend(args[0]),
        // Pas de pile de signal alternative : la trame est ecrite sur la pile
        // courante, sous la zone rouge.
        SIGALTSTACK => 0,
        RT_SIGPENDING => {
            let pending = task::current_process().signals.lock().pending;
            if args[0] != 0 && !user_write(args[0], &pending.to_le_bytes()) {
                -errno::EFAULT
            } else {
                0
            }
        }
        PAUSE => sys_sigsuspend(0),
        ALARM => sys_alarm(args[0] as u32),
        SETITIMER => sys_setitimer(args[0] as u32, args[1], args[2]),
        GETITIMER => sys_getitimer(args[0] as u32, args[1]),

        // --- Sockets ---
        SOCKET => net::sys_socket(args[0] as u32, args[1] as u32, args[2] as u32),
        CONNECT => net::sys_connect(args[0] as i32, args[1], args[2] as usize),
        BIND => net::sys_bind(args[0] as i32, args[1], args[2] as usize),
        LISTEN | ACCEPT | ACCEPT4 => net::sys_listen_unsupported(),
        SENDTO => net::sys_sendto(args[0] as i32, args[1], args[2] as usize, args[3] as u32, args[4], args[5] as usize),
        RECVFROM => net::sys_recvfrom(args[0] as i32, args[1], args[2] as usize, args[3] as u32, args[4], args[5]),
        SHUTDOWN => net::sys_shutdown(args[0] as i32, args[1] as u32),
        GETSOCKNAME => net::sys_getsockname(args[0] as i32, args[1], args[2], false),
        GETPEERNAME => net::sys_getsockname(args[0] as i32, args[1], args[2], true),
        SETSOCKOPT => net::sys_setsockopt(args[0] as i32, args[1] as u32, args[2] as u32, args[3], args[4] as usize),
        GETSOCKOPT => net::sys_getsockopt(args[0] as i32, args[1] as u32, args[2] as u32, args[3], args[4]),
        SOCKETPAIR => net::sys_socketpair(args[0] as u32, args[1] as u32, args[2] as u32, args[3]),
        SENDMSG => net::sys_sendmsg(args[0] as i32, args[1], args[2] as u32),
        RECVMSG => net::sys_recvmsg(args[0] as i32, args[1], args[2] as u32),
        SENDMMSG => net::sys_sendmmsg(args[0] as i32, args[1], args[2] as u32, args[3] as u32),
        RECVMMSG => net::sys_recvmmsg(args[0] as i32, args[1], args[2] as u32, args[3] as u32, args[4]),

        // --- Temps ---
        CLOCK_GETTIME => sys_clock_gettime(args[0] as i32, args[1]),
        CLOCK_GETRES => {
            // Resolution = un tick du PIT.
            let ns = 1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND;
            if args[1] != 0 {
                user_write(args[1], &0u64.to_le_bytes());
                user_write(args[1] + 8, &ns.to_le_bytes());
            }
            0
        }
        GETTIMEOFDAY => {
            let ms = realtime_ms();
            if args[0] != 0 {
                user_write(args[0], &(ms / 1000).to_le_bytes());
                user_write(args[0] + 8, &((ms % 1000) * 1000).to_le_bytes());
            }
            0
        }
        TIME => {
            let seconds = unix_time();
            if args[0] != 0 {
                user_write(args[0], &seconds.to_le_bytes());
            }
            seconds as i64
        }
        TIMES => 0,

        // --- Informations systeme ---
        UNAME => sys_uname(args[0]),
        SYSINFO => sys_sysinfo(args[0]),
        GETRLIMIT | PRLIMIT64 => sys_prlimit(args),
        // `setrlimit(resource, new)` : meme traitement que `prlimit64`, avec les
        // arguments decales d'un cran et pas d'ancienne valeur a rendre.
        SETRLIMIT => sys_prlimit([0, args[0], args[1], 0, 0, 0]),
        GETRANDOM => sys_getrandom(args[0], args[1] as usize),
        SYSLOG => 0,
        MEMBARRIER => 0,

        _ => {
            LAST_UNKNOWN.store(number, Ordering::Relaxed);
            // Le numero seul ne dit pas *qui* appelle, et c'est la seule chose
            // qui permette de decider entre implementer la vraie semantique et
            // documenter pourquoi l'appel est facultatif. `frame.rip` est
            // l'adresse de retour ring 3 sauvegardee par `syscall` ; les
            // binaires Ladybird sont des PIE statiques charges a
            // `user_load_base()`, donc la difference est directement un
            // deplacement dans le fichier :
            //
            //     addr2line -f -C -e WebContent <offset>
            let base = crate::kernel::vmm::user_load_base();
            let offset = frame.rip.wrapping_sub(base);
            crate::serial_println!(
                "[syscall] non implemente : {} ({}) appelant={} rip={:#x} offset={:#x}",
                number,
                nr::name(number),
                task::current_process().metadata.lock().name,
                frame.rip,
                offset
            );
            -errno::ENOSYS
        }
    }
}

/// Ancrage de l'horloge murale : (secondes Unix, ticks) releves au premier
/// appel. Voir [`realtime_ms`].
/// Ancre de l'horloge murale : la seconde RTC lue une fois, et le tick auquel
/// elle a ete lue.
///
/// C'etait un `static mut Option<(u64, u64)>` initialise paresseusement. Tant
/// que `clock_gettime` s'executait sous le gros verrou, un seul CPU pouvait y
/// etre a la fois. Le liberer expose la course : deux CPU lisent `None`,
/// appellent tous deux la RTC, et se marchent dessus sur une paire de 128 bits
/// qui n'a rien d'atomique. Un `Option` a moitie ecrit lu par un troisieme CPU
/// n'est pas une valeur approximative, c'est un comportement indefini.
///
/// Deux atomiques et un drapeau : le premier qui pose l'ancre gagne, les autres
/// jettent la leur et lisent la sienne. Tout le monde voit donc la MEME ancre,
/// ce qui est la vraie exigence -- une horloge murale qui differe d'un CPU a
/// l'autre reculerait a chaque migration.
static EPOCH_SECONDS: AtomicU64 = AtomicU64::new(0);
static EPOCH_TICKS: AtomicU64 = AtomicU64::new(0);
static EPOCH_POSEE: AtomicU8 = AtomicU8::new(0);

/// Lit la RTC et la convertit en secondes depuis l'epoch Unix.
fn rtc_seconds() -> u64 {
    let now = crate::arch::x86_64::rtc::now_utc();
    days_from_civil(now.year as i64, now.month as i64, now.day as i64) as u64 * 86400
        + now.hour as u64 * 3600
        + now.minute as u64 * 60
        + now.second as u64
}

/// Horloge murale en millisecondes depuis l'epoch Unix.
///
/// La RTC ne donne que des secondes entieres. S'en contenter rendrait toute
/// echeance sub-seconde inexploitable : un programme qui lit l'heure, y ajoute
/// 200 ms et attend jusque-la verrait une horloge figee pendant une seconde
/// entiere, puis un saut. On ancre donc la seconde RTC une fois pour toutes, et
/// on y ajoute le temps ecoule mesure par le timer — ce qui donne la
/// milliseconde et, accessoirement, une horloge qui ne recule jamais.
pub fn realtime_ms() -> u64 {
    let now_ticks = crate::kernel::timer::ticks();
    if EPOCH_POSEE.load(Ordering::Acquire) == 0 {
        let seconds = rtc_seconds();
        // Les deux valeurs sont publiees AVANT le drapeau ; le drapeau est lu
        // en `Acquire` : personne ne peut voir le drapeau leve sur une ancre
        // incomplete. Le perdant du `compare_exchange` jette simplement sa
        // lecture RTC.
        if EPOCH_POSEE
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            EPOCH_SECONDS.store(seconds, Ordering::Relaxed);
            EPOCH_TICKS.store(now_ticks, Ordering::Relaxed);
            EPOCH_POSEE.store(2, Ordering::Release);
        }
        while EPOCH_POSEE.load(Ordering::Acquire) != 2 {
            core::hint::spin_loop();
        }
    }
    let base_seconds = EPOCH_SECONDS.load(Ordering::Relaxed);
    let base_ticks = EPOCH_TICKS.load(Ordering::Relaxed);
    let elapsed_ticks = now_ticks.saturating_sub(base_ticks);
    base_seconds * 1000 + elapsed_ticks * 1000 / crate::kernel::timer::TICKS_PER_SECOND
}

/// Secondes depuis l'epoch Unix.
pub fn unix_time() -> u64 {
    realtime_ms() / 1000
}

/// Jours ecoules depuis 1970-01-01 (algorithme de Howard Hinnant).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

/// `clock_gettime` : horloge murale ou monotone.
fn sys_clock_gettime(clock: i32, out: u64) -> i64 {
    const CLOCK_REALTIME: i32 = 0;
    const CLOCK_PROCESS_CPUTIME_ID: i32 = 2;
    const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
    const CLOCK_REALTIME_COARSE: i32 = 5;
    const CLOCK_REALTIME_ALARM: i32 = 8;

    // Les horloges de temps **processeur** ne sont pas des horloges murales, et
    // les confondre avec le temps monotone — ce que faisait ce code — rend
    // impossible la seule mesure qui distingue une attente qui dort d'une
    // attente qui brule un cœur : les deux durent la meme chose au mur, pas du
    // tout la meme chose en processeur.
    //
    // Le noyau compte deja, par echantillonnage a chaque IRQ0. Il suffisait de
    // le dire a l'espace utilisateur.
    let ms = match clock {
        CLOCK_REALTIME | CLOCK_REALTIME_COARSE | CLOCK_REALTIME_ALARM => realtime_ms(),
        CLOCK_PROCESS_CPUTIME_ID | CLOCK_THREAD_CPUTIME_ID => {
            // Seule branche de cet appel qui touche la table des taches :
            // `cpu_time_ms` la parcourt pour additionner les ticks de tous les
            // fils du processus. Elle prend donc le gros verrou, ici et nulle
            // part ailleurs. Le reste de `clock_gettime` -- c'est-a-dire tout
            // ce qu'emet une boucle d'evenements -- s'en passe.
            let _kernel = crate::kernel::smp_lock::enter();
            let pid = crate::kernel::task::current_process().pid;
            crate::kernel::task::cpu_time_ms(pid)
        }
        _ => crate::kernel::timer::monotonic_ms(),
    };
    if out == 0 {
        return -errno::EFAULT;
    }
    let seconds = ms / 1000;
    let nanos = (ms % 1000) * 1_000_000;
    if !user_write(out, &seconds.to_le_bytes()) || !user_write(out + 8, &nanos.to_le_bytes()) {
        return -errno::EFAULT;
    }
    0
}

/// Lit un `struct timespec` utilisateur et le convertit en millisecondes.
fn timespec_ms(addr: u64) -> Option<u64> {
    if addr == 0 {
        return None;
    }
    let seconds = user_read_u64(addr)?;
    let nanos = user_read_u64(addr + 8)?;
    Some(seconds.saturating_mul(1000) + nanos / 1_000_000)
}

/// `nanosleep` : la duree demandee est toujours **relative**.
fn sys_nanosleep(request: u64, remain: u64) -> i64 {
    dors_ms(timespec_ms(request).unwrap_or(0));
    if remain != 0 {
        // Le sommeil n'a pas ete interrompu : il ne reste rien a dormir.
        user_write(remain, &0u64.to_le_bytes());
        user_write(remain + 8, &0u64.to_le_bytes());
    }
    0
}

/// `clock_nanosleep(clockid, flags, request, remain)`.
///
/// Meme piege que `FUTEX_WAIT_BITSET` : le drapeau `TIMER_ABSTIME` transforme
/// la duree en **echeance**. C'est cette forme qu'emploie `time.sleep()` de
/// CPython. Prendre l'echeance pour une duree fait dormir aussi longtemps que
/// la machine a deja tourne — un `sleep(0,2 s)` demande apres neuf secondes
/// d'uptime dure alors neuf secondes.
fn sys_clock_nanosleep(clock: i32, flags: u64, request: u64, remain: u64) -> i64 {
    const TIMER_ABSTIME: u64 = 1;
    const CLOCK_REALTIME: i32 = 0;
    const CLOCK_REALTIME_ALARM: i32 = 8;

    let demande = match timespec_ms(request) {
        Some(ms) => ms,
        None => return -errno::EFAULT,
    };

    if flags & TIMER_ABSTIME != 0 {
        let maintenant = match clock {
            CLOCK_REALTIME | CLOCK_REALTIME_ALARM => realtime_ms(),
            _ => crate::kernel::timer::monotonic_ms(),
        };
        // Une echeance deja passee rend la main tout de suite. `remain` n'est
        // pas ecrit dans cette forme : Linux ne le remplit que pour un sommeil
        // relatif, l'echeance etant deja connue de l'appelant.
        dors_ms(demande.saturating_sub(maintenant));
        return 0;
    }

    dors_ms(demande);
    if remain != 0 {
        user_write(remain, &0u64.to_le_bytes());
        user_write(remain + 8, &0u64.to_le_bytes());
    }
    0
}

/// Dort `ms` millisecondes, en cedant simplement le CPU sous le tick.
fn dors_ms(ms: u64) {
    let ticks = crate::kernel::timer::ms_to_ticks(ms);
    if ticks == 0 {
        task::yield_now();
    } else {
        task::sleep_ticks(ticks);
    }
}

/// `futex` : la primitive d'attente sur laquelle reposent tous les mutex et
/// variables de condition de musl (donc de Qt et de Python).
fn sys_futex(args: [u64; 6]) -> i64 {
    const FUTEX_WAIT: u32 = 0;
    const FUTEX_WAKE: u32 = 1;
    const FUTEX_REQUEUE: u32 = 3;
    const FUTEX_CMP_REQUEUE: u32 = 4;
    const FUTEX_WAIT_BITSET: u32 = 9;
    const FUTEX_WAKE_BITSET: u32 = 10;
    const FUTEX_PRIVATE_FLAG: u32 = 128;
    const FUTEX_CLOCK_REALTIME: u32 = 256;

    let uaddr = args[0];
    let raw_operation = args[1] as u32;
    let operation = raw_operation & !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME);
    let expected = args[2] as u32;

    match operation {
        FUTEX_WAIT | FUTEX_WAIT_BITSET => {
            // Piege de l'ABI : `FUTEX_WAIT` recoit une duree **relative**,
            // `FUTEX_WAIT_BITSET` une echeance **absolue** — et c'est cette
            // seconde forme qu'emploie `pthread_cond_timedwait`. Traiter une
            // date absolue comme une duree donnerait un delai de plusieurs
            // decennies : l'attente ne rendrait jamais la main.
            let timeout_ms = match timespec_ms(args[3]) {
                None => 0, // sans limite
                Some(ms) if operation == FUTEX_WAIT => ms.max(1),
                Some(deadline_ms) => {
                    let now_ms = if raw_operation & FUTEX_CLOCK_REALTIME != 0 {
                        unix_time().saturating_mul(1000)
                    } else {
                        crate::kernel::timer::monotonic_ms()
                    };
                    deadline_ms.saturating_sub(now_ms).max(1)
                }
            };
            if task::futex_wait(uaddr, expected, timeout_ms) {
                0
            } else {
                -errno::ETIMEDOUT
            }
        }
        FUTEX_WAKE | FUTEX_WAKE_BITSET => task::futex_wake(uaddr, expected) as i64,
        FUTEX_REQUEUE | FUTEX_CMP_REQUEUE => {
            // Sans file d'attente par adresse cible, on se contente de reveiller :
            // correct fonctionnellement, juste moins efficace.
            task::futex_wake(uaddr, expected) as i64
        }
        _ => -errno::ENOSYS,
    }
}

/// `arch_prctl` : c'est ainsi que la libc installe le TLS (`%fs`).
fn sys_arch_prctl(code: i32, addr: u64) -> i64 {
    const ARCH_SET_GS: i32 = 0x1001;
    const ARCH_SET_FS: i32 = 0x1002;
    const ARCH_GET_FS: i32 = 0x1003;
    const ARCH_GET_GS: i32 = 0x1004;
    match code {
        ARCH_SET_FS => {
            usermode::set_fs_base(addr);
            task::current().fs_base = addr;
            0
        }
        ARCH_GET_FS => {
            if user_write_u64(addr, usermode::fs_base()) { 0 } else { -errno::EFAULT }
        }
        ARCH_SET_GS => {
            // GS est reserve au noyau (structure par-CPU) : on stocke la valeur
            // dans KERNEL_GS_BASE cote utilisateur via swapgs, non supporte ici.
            -errno::EINVAL
        }
        ARCH_GET_GS => {
            if user_write_u64(addr, 0) { 0 } else { -errno::EFAULT }
        }
        _ => -errno::EINVAL,
    }
}

/// `clone3` : la forme moderne de `clone`, avec ses arguments en memoire.
///
/// ## Pourquoi il a fallu l'implementer
///
/// Parce que la glibc l'emet **avant** `clone`, et que rendre `ENOSYS` ne
/// suffit pas toujours a la faire reculer. C'est exactement le meme piege que
/// celui deja documente dans `proc_clone` a propos de `fork` : l'appel systeme
/// historique etait implemente et teste, mais les programmes reels emettent
/// l'autre.
///
/// Le symptome, lui, ne parlait pas de `clone3` : le temoin LibJS s'arretait a
/// sa dixieme verification — celle qui met le ramasse-miettes sous pression et
/// fait donc demarrer le fil de collecte de LibGC — par une dereference de
/// pointeur nul a l'adresse 0x120, apres que la memoire de la machine soit
/// montee a 100 %.
///
/// ## La difference qui compte
///
/// `struct clone_args` (`include/uapi/linux/sched.h`) donne la pile par sa
/// **base et sa taille**, la ou `clone` recevait son **sommet**. Sur x86-64 la
/// pile descend : le sommet vaut donc `stack + stack_size`. Confondre les deux
/// donnerait au fil une pile qui croit vers le bas depuis son propre debut,
/// c'est-a-dire hors de sa zone — un defaut qui ne se manifeste qu'au premier
/// appel un peu profond.
fn proc_clone3(args: [u64; 6], frame: &TrapFrame) -> i64 {
    let adresse = args[0];
    let taille = args[1] as usize;
    // La structure a grandi au fil des versions du noyau ; les 64 premiers
    // octets sont figes et portent tout ce dont nous avons besoin.
    if adresse == 0 || taille < 64 {
        return -errno::EINVAL;
    }

    let lire = |decalage: u64| -> Option<u64> { user_read_u64(adresse + decalage) };

    let (flags, child_tid, parent_tid, pile, taille_pile, tls) = match (
        lire(0), lire(16), lire(24), lire(40), lire(48), lire(56),
    ) {
        (Some(f), Some(c), Some(p), Some(s), Some(ts), Some(t)) => (f, c, p, s, ts, t),
        _ => return -errno::EFAULT,
    };

    // `clone` attend le sommet de pile ; `clone3` donne la base et la taille.
    let sommet = if pile != 0 { pile + taille_pile } else { 0 };

    proc_clone([flags, sommet, parent_tid, child_tid, tls, 0], frame)
}

/// `clone` : creation de thread. Seul `CLONE_THREAD` (pthread) est supporte ;
/// un `fork` complet demanderait la copie paresseuse de l'espace d'adressage.
fn proc_clone(args: [u64; 6], frame: &TrapFrame) -> i64 {
    const CLONE_VM: u64 = 0x100;
    const CLONE_THREAD: u64 = 0x10000;
    const CLONE_SETTLS: u64 = 0x80000;
    const CLONE_PARENT_SETTID: u64 = 0x100000;
    const CLONE_CHILD_CLEARTID: u64 = 0x200000;
    const CLONE_CHILD_SETTID: u64 = 0x1000000;

    let flags = args[0];
    let child_stack = args[1];
    let parent_tid = args[2];
    let child_tid = args[3];
    let tls = args[4];

    if flags & (CLONE_VM | CLONE_THREAD) != (CLONE_VM | CLONE_THREAD) {
        // Sans `CLONE_VM` ni `CLONE_THREAD`, ce n'est pas un fil : c'est un
        // **fork**, et `proc::sys_fork` sait le faire depuis longtemps.
        //
        // Cette branche rendait `ENOSYS`, et c'est ce qui a empeche le
        // navigateur de creer son processus de rendu sous Bouchaud OS.
        // `fork()` de la glibc n'emet pas l'appel systeme `fork` (57) : depuis
        // longtemps, elle passe par `clone` avec
        // `CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID | SIGCHLD`. L'appel
        // systeme 57 etait donc implemente, teste, et **jamais atteint** par
        // un programme reel — tandis que celui que les programmes emettent
        // vraiment repondait « non implemente ».
        //
        // C'est le genre de trou qu'aucune epreuve sur Linux ne peut voir : la
        // separation navigateur/renderer passe ses 1630 verifications sur une
        // machine ou `fork` existe, et n'avait jamais tourne ici.
        let resultat = proc::sys_fork(frame);
        if resultat > 0 {
            // `CLONE_PARENT_SETTID` : le pere veut connaitre l'identifiant de
            // son fils a cet emplacement, en plus de la valeur de retour.
            if flags & CLONE_PARENT_SETTID != 0 && parent_tid != 0 {
                user_write_u32(parent_tid, resultat as u32);
            }
            // `CLONE_CHILD_SETTID` n'est pas honore : il demande d'ecrire dans
            // l'espace d'adressage **du fils**, auquel le pere n'a pas acces
            // depuis ici. La consequence est bornee et connue : le fils garde
            // dans son bloc de fil l'identifiant copie du pere, si bien qu'une
            // libc qui le lit sans repasser par un appel systeme se croit son
            // pere. `getpid`, `gettid` et l'envoi de signaux par le noyau ne
            // s'en servent pas — ils passent par le noyau, qui a la bonne
            // valeur. Ce qui s'en sert est `raise()` de la glibc, qui viserait
            // alors le pere. A corriger le jour ou l'on saura ecrire dans
            // l'espace d'un processus qu'on vient de creer.
            let _ = child_tid;
        }
        return resultat;
    }
    if child_stack == 0 {
        return -errno::EINVAL;
    }

    // Le thread reprend a l'instruction qui suit le `syscall`, avec sa propre
    // pile et rax = 0 (c'est ainsi que la libc distingue parent et enfant).
    //
    // La pile est reprise **telle quelle**, sans realignement : le trampoline
    // `__clone` de musl a deja aligne puis empile l'argument du thread juste
    // sous le pointeur transmis. Le moindre arrondi ferait depiler la mauvaise
    // valeur, puis appeler une adresse arbitraire.
    let mut child_frame = *frame;
    child_frame.rsp = child_stack;
    child_frame.rax = 0;

    let process = task::current().process.clone();
    process.lifecycle.lock().threads += 1;
    let mut child = task::Task::new(process, child_frame);
    if flags & CLONE_SETTLS != 0 {
        child.fs_base = tls;
    }
    if flags & CLONE_CHILD_CLEARTID != 0 {
        child.clear_child_tid = child_tid;
    }
    let tid = child.tid;
    if flags & (CLONE_PARENT_SETTID | CLONE_CHILD_SETTID) != 0 {
        if parent_tid != 0 {
            user_write_u32(parent_tid, tid);
        }
        if child_tid != 0 && flags & CLONE_CHILD_SETTID != 0 {
            user_write_u32(child_tid, tid);
        }
    }
    task::register(child);
    tid as i64
}

/// `kill` / `tkill` / `tgkill` : modele minimal — seuls les signaux fatals
/// agissent, faute de gestionnaires en ring 3.
///
/// Les traiter reellement importe : `abort()` d'une libc s'envoie `SIGABRT` en
/// boucle jusqu'a mourir. Un noyau qui repondrait « 0, rien fait » laisserait le
/// programme tourner indefiniment au lieu de s'arreter.
fn sys_kill(signal: u64) -> i64 {
    const SIGABRT: u64 = 6;
    const SIGKILL: u64 = 9;
    const SIGSEGV: u64 = 11;
    const SIGTERM: u64 = 15;
    if matches!(signal, SIGABRT | SIGKILL | SIGSEGV | SIGTERM) {
        task::exit_group(128 + signal as i32);
    }
    0
}

/// `uname` : `struct utsname`, six champs de 65 octets.
fn sys_uname(addr: u64) -> i64 {
    if addr == 0 {
        return -errno::EFAULT;
    }
    let mut buffer = [0u8; 65 * 6];
    let fields = [
        "Linux",                    // sysname : musl et Qt testent cette valeur
        "bouchaud",                 // nodename
        "6.1.0-bouchaud",           // release : >= 3.2 exige par musl
        crate::VERSION,             // version
        "x86_64",                   // machine
        "(none)",                   // domainname
    ];
    for (index, field) in fields.iter().enumerate() {
        let bytes = field.as_bytes();
        let start = index * 65;
        let len = bytes.len().min(64);
        buffer[start..start + len].copy_from_slice(&bytes[..len]);
    }
    if user_write(addr, &buffer) { 0 } else { -errno::EFAULT }
}

/// `sysinfo` : uptime, memoire totale/libre.
fn sys_sysinfo(addr: u64) -> i64 {
    if addr == 0 {
        return -errno::EFAULT;
    }
    let (_, free_frames, total_frames) = crate::kernel::vmm::frame_stats();
    let mut buffer = [0u8; 112];
    let uptime = crate::kernel::timer::seconds();
    buffer[0..8].copy_from_slice(&uptime.to_le_bytes());
    // loads[3] a l'offset 8..32, laisses a zero.
    buffer[32..40].copy_from_slice(&(total_frames * 4096).to_le_bytes());
    buffer[40..48].copy_from_slice(&(free_frames * 4096).to_le_bytes());
    // mem_unit (offset 104) = 1 octet.
    buffer[104..108].copy_from_slice(&1u32.to_le_bytes());
    if user_write(addr, &buffer) { 0 } else { -errno::EFAULT }
}

/// `getrlimit` / `prlimit64`.
/// `getrlimit` / `setrlimit` / `prlimit64`.
///
/// Un seul quota est reellement applique : `RLIMIT_AS`. Ce n'est pas de la
/// paresse mais un choix — c'est le seul qui change quelque chose au projet de
/// processus separes. Un renderer qui fuit sans plafond emporte la machine, et
/// l'isolation des pannes ne vaut plus rien ; avec un plafond, sa `mmap` echoue
/// proprement et le processus meurt seul. Les autres ressources continuent de
/// se declarer illimitees, ce qui est la verite.
fn sys_prlimit(args: [u64; 6]) -> i64 {
    const RLIMIT_STACK: u64 = 3;
    const RLIMIT_NOFILE: u64 = 7;
    const RLIMIT_AS: u64 = 9;
    const RLIM_INFINITY: u64 = u64::MAX;
    // prlimit64(pid, resource, new, old) et getrlimit(resource, old).
    let (resource, new, old) = if args[3] != 0 || args[2] != 0 {
        (args[1], args[2], args[3])
    } else {
        (args[0], 0, args[1])
    };

    // L'ancienne valeur se lit **avant** d'installer la nouvelle : c'est ce que
    // `prlimit` promet, et ce dont depend tout code qui abaisse un quota le
    // temps d'une operation puis le restaure.
    if old != 0 {
        let (soft, hard) = match resource {
            RLIMIT_STACK => (crate::kernel::vmm::USER_STACK_SIZE, crate::kernel::vmm::USER_STACK_SIZE),
            RLIMIT_NOFILE => (1024, 1024),
            RLIMIT_AS => {
                let limite = task::current_process().mm.lock().limite_as;
                let valeur = if limite == 0 { RLIM_INFINITY } else { limite };
                (valeur, RLIM_INFINITY)
            }
            _ => (RLIM_INFINITY, RLIM_INFINITY),
        };
        user_write(old, &soft.to_le_bytes());
        user_write(old + 8, &hard.to_le_bytes());
    }

    if new != 0 && resource == RLIMIT_AS {
        let soft = match user_read_u64(new) {
            Some(valeur) => valeur,
            None => return -errno::EFAULT,
        };
        // 0 en interne veut dire « pas de plafond » ; c'est ce que represente
        // `RLIM_INFINITY` cote utilisateur.
        task::current_process().mm.lock().limite_as =
            if soft == RLIM_INFINITY { 0 } else { soft };
    }
    0
}

/// `getrandom`.
fn sys_getrandom(addr: u64, len: usize) -> i64 {
    if len == 0 {
        return 0;
    }
    let mut buffer = alloc::vec![0u8; len.min(4096)];
    crate::net::security::tls::rng::fill(&mut buffer);
    if user_write(addr, &buffer) {
        buffer.len() as i64
    } else {
        -errno::EFAULT
    }
}

/// Resout un chemin utilisateur en chaine noyau, en le rendant absolu.
pub fn resolve_user_path(addr: u64) -> Option<String> {
    user_string(addr)
}

/// Affiche la table des appels systeme implementes (commande `syscalls`).
/// Les appels systeme les plus emis depuis le boot, du plus chaud au moins.
///
/// C'est la donnee sur laquelle se decide quel appel merite qu'on lui ecrive
/// une preuve de synchronisation (voir [`bkl`]). Le verrouillage de chacun est
/// affiche a cote : on voit d'un coup d'oeil ou passe le temps sous verrou.
pub fn print_frequences(combien: usize) {
    let mut top: Vec<(u64, u32)> = Vec::new();
    for numero in 0..SYSCALL_HITS_LEN {
        let hits = SYSCALL_HITS[numero].load(Ordering::Relaxed);
        if hits != 0 {
            top.push((numero as u64, hits));
        }
    }
    top.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    crate::println!("");
    crate::println!("appels les plus emis (numero, nom, compte, verrou) :");
    for (numero, hits) in top.iter().take(combien) {
        crate::println!(
            "  {:>4} {:<20} {:>10}  {}",
            numero,
            nr::name(*numero),
            hits,
            if bkl::exige_bkl(*numero) { "BKL" } else { "sans" },
        );
    }
    crate::println!("  ({} numeros distincts emis)", top.len());
}

pub fn print_table() {
    crate::println!("ABI Bouchaud OS : appels systeme Linux x86-64");
    crate::println!("(numeros, structures et codes d'erreur identiques a Linux)");
    crate::println!("");
    nr::print_implemented();
    crate::println!("");
    crate::println!("appels traites depuis le boot : {}", syscall_count());
    print_frequences(12);
    let unknown = last_unknown();
    if unknown != 0 {
        crate::println!("dernier appel non implemente : {} ({})", unknown, nr::name(unknown));
    }
    crate::println!("trace serie (`strace on|off`) : {}", if trace_enabled() { "active" } else { "inactive" });
}
