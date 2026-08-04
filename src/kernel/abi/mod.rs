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
//! ## Invariant de non-reentrance
//!
//! > **Aucun emprunt du [`Process`](crate::kernel::task::Process) ne doit
//! > survivre a un point de commutation.**
//!
//! Le noyau n'est pas reentrant, et tout le reste s'appuie la-dessus. L'etat
//! d'un processus vit dans un `Rc<RefCell<Process>>` : le `RefCell` compte les
//! emprunts a l'execution, pas a la compilation. Un appel systeme qui garderait
//! un `borrow_mut()` ouvert en appelant quelque chose qui rend la main —
//! [`yield_now`](crate::kernel::task::yield_now), un `futex` qui attend, une
//! lecture bloquante — laisserait le compteur d'emprunts arme pendant que la
//! tache suivante entre dans le meme processus. Le second emprunt paniquerait
//! sur un `BorrowMutError`, c'est-a-dire une panique noyau, avec une trace
//! designant la seconde tache et non celle qui a laisse l'emprunt ouvert.
//!
//! En pratique : relacher le `borrow` (fin de portee, ou copie de ce dont on a
//! besoin) **avant** tout appel susceptible de bloquer. Le point de commutation
//! unique, [`task::schedule`](crate::kernel::task::schedule), verifie
//! l'invariant par un `debug_assert` : la panique designe alors le fautif au
//! lieu de sa victime.
//!
//! La preemption par le timer, elle, ne peut pas casser l'invariant : elle
//! n'agit que si l'interruption a surpris du code ring 3, ou aucun emprunt
//! noyau n'est detenu (voir `arch::x86_64::idt`).

pub mod errno;
pub mod file;
pub mod mem;
pub mod net;
pub mod nr;
pub mod proc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::arch::x86_64::usermode::{self, TrapFrame};
use crate::kernel::task;

/// Compteur d'appels systeme, pour le diagnostic.
static mut SYSCALL_COUNT: u64 = 0;
/// Dernier appel systeme inconnu rencontre (numero), 0 si aucun.
static mut LAST_UNKNOWN: u64 = 0;
/// Trace des appels systeme sur la sortie serie (commande `strace`).
static mut TRACE: bool = false;

/// Active ou desactive la trace des appels systeme.
pub fn set_trace(on: bool) {
    unsafe { TRACE = on };
}

/// La trace est-elle active ?
pub fn trace_enabled() -> bool {
    unsafe { TRACE }
}

/// Nombre d'appels systeme traites depuis le boot.
pub fn syscall_count() -> u64 {
    unsafe { SYSCALL_COUNT }
}

/// Dernier numero d'appel systeme non implemente (0 si aucun).
pub fn last_unknown() -> u64 {
    unsafe { LAST_UNKNOWN }
}

/// Point d'entree du dispatch : lit la trame, execute, ecrit le retour.
pub fn handle(frame: &mut TrapFrame) {
    unsafe { SYSCALL_COUNT += 1 };
    let (number, args) = frame.syscall_args();
    let result = dispatch(number, args, frame);
    if unsafe { TRACE } {
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
    let saved = process.borrow().signals.blocked;
    if set != 0 {
        if let Some(mask) = user_read_u64(set) {
            let mut borrowed = process.borrow_mut();
            borrowed.signals.blocked = mask & !(1 << (crate::kernel::signal::SIGKILL - 1));
        }
    }
    while !task::signal_pending() {
        task::yield_now();
        crate::arch::x86_64::cpu::hlt();
    }
    process.borrow_mut().signals.blocked = saved;
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

/// Copie une chaine C depuis l'espace utilisateur.
fn user_string(addr: u64) -> Option<String> {
    if addr == 0 {
        return None;
    }
    let process = task::current_process();
    let mut process = process.borrow_mut();
    process.space.read_cstr(addr, 4096)
}

/// Copie un tampon depuis l'espace utilisateur.
pub fn user_read(addr: u64, len: usize) -> Option<Vec<u8>> {
    let mut buffer = alloc::vec![0u8; len];
    let process = task::current_process();
    let mut process = process.borrow_mut();
    if process.space.read(addr, &mut buffer) {
        Some(buffer)
    } else {
        None
    }
}

/// Copie un tampon vers l'espace utilisateur.
pub fn user_write(addr: u64, data: &[u8]) -> bool {
    let process = task::current_process();
    let mut process = process.borrow_mut();
    process.space.write(addr, data)
}

/// Ecrit une valeur 64 bits dans l'espace utilisateur.
pub fn user_write_u64(addr: u64, value: u64) -> bool {
    user_write(addr, &value.to_le_bytes())
}

/// Ecrit une valeur 32 bits dans l'espace utilisateur.
pub fn user_write_u32(addr: u64, value: u32) -> bool {
    user_write(addr, &value.to_le_bytes())
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
        FSYNC | FDATASYNC => 0,
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
        MADVISE | MLOCK | MUNLOCK | MLOCKALL | MUNLOCKALL => 0,

        // --- Processus, threads, ordonnancement ---
        GETPID => task::current().process.borrow().pid as i64,
        GETPPID => 1,
        GETTID => task::current().tid as i64,
        SET_TID_ADDRESS => {
            task::current().clear_child_tid = args[0];
            task::current().tid as i64
        }
        SET_ROBUST_LIST | GET_ROBUST_LIST | RSEQ => 0,
        CLONE => proc_clone(args, frame),
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
        GETUID | GETEUID => task::current().process.borrow().uid as i64,
        GETGID | GETEGID => task::current().process.borrow().gid as i64,
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
            let pending = task::current_process().borrow().signals.pending;
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
        SETRLIMIT => 0,
        GETRANDOM => sys_getrandom(args[0], args[1] as usize),
        SYSLOG => 0,
        MEMBARRIER => 0,

        _ => {
            unsafe { LAST_UNKNOWN = number };
            crate::serial_println!("[syscall] non implemente : {} ({})", number, nr::name(number));
            -errno::ENOSYS
        }
    }
}

/// Ancrage de l'horloge murale : (secondes Unix, ticks) releves au premier
/// appel. Voir [`realtime_ms`].
static mut EPOCH_ANCHOR: Option<(u64, u64)> = None;

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
    let (base_seconds, base_ticks) = unsafe {
        match EPOCH_ANCHOR {
            Some(anchor) => anchor,
            None => {
                let anchor = (rtc_seconds(), now_ticks);
                EPOCH_ANCHOR = Some(anchor);
                anchor
            }
        }
    };
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
    const CLOCK_REALTIME_COARSE: i32 = 5;
    const CLOCK_REALTIME_ALARM: i32 = 8;
    // Toutes les autres horloges (MONOTONIC, MONOTONIC_RAW, BOOTTIME,
    // les horloges de processus et de thread) partagent la meme base monotone.
    let ms = match clock {
        CLOCK_REALTIME | CLOCK_REALTIME_COARSE | CLOCK_REALTIME_ALARM => realtime_ms(),
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
            let timeout = match timespec_ms(args[3]) {
                None => 0, // sans limite
                Some(ms) if operation == FUTEX_WAIT => crate::kernel::timer::ms_to_ticks(ms).max(1),
                Some(deadline_ms) => {
                    let now_ms = if raw_operation & FUTEX_CLOCK_REALTIME != 0 {
                        unix_time().saturating_mul(1000)
                    } else {
                        crate::kernel::timer::monotonic_ms()
                    };
                    crate::kernel::timer::ms_to_ticks(deadline_ms.saturating_sub(now_ms)).max(1)
                }
            };
            if task::futex_wait(uaddr, expected, timeout) {
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
        // fork() reel : non supporte (voir la note du module).
        return -errno::ENOSYS;
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
    process.borrow_mut().threads += 1;
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
fn sys_prlimit(args: [u64; 6]) -> i64 {
    const RLIMIT_STACK: u64 = 3;
    const RLIMIT_NOFILE: u64 = 7;
    const RLIM_INFINITY: u64 = u64::MAX;
    // prlimit64(pid, resource, new, old) et getrlimit(resource, old).
    let (resource, old) = if args[3] != 0 || args[2] != 0 {
        (args[1], args[3])
    } else {
        (args[0], args[1])
    };
    if old == 0 {
        return 0;
    }
    let (soft, hard) = match resource {
        RLIMIT_STACK => (crate::kernel::vmm::USER_STACK_SIZE, crate::kernel::vmm::USER_STACK_SIZE),
        RLIMIT_NOFILE => (1024, 1024),
        _ => (RLIM_INFINITY, RLIM_INFINITY),
    };
    user_write(old, &soft.to_le_bytes());
    user_write(old + 8, &hard.to_le_bytes());
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
pub fn print_table() {
    crate::println!("ABI Bouchaud OS : appels systeme Linux x86-64");
    crate::println!("(numeros, structures et codes d'erreur identiques a Linux)");
    crate::println!("");
    nr::print_implemented();
    crate::println!("");
    crate::println!("appels traites depuis le boot : {}", syscall_count());
    let unknown = last_unknown();
    if unknown != 0 {
        crate::println!("dernier appel non implemente : {} ({})", unknown, nr::name(unknown));
    }
    crate::println!("trace serie (`strace on|off`) : {}", if trace_enabled() { "active" } else { "inactive" });
}
