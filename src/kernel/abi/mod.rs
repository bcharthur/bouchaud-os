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

pub mod errno;
pub mod file;
pub mod mem;
pub mod nr;

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
    frame.rax = result as u64;

    // Preemption differee : le timer a pu marquer un besoin de commutation
    // pendant l'appel, ou l'appel a pu reveiller une tache plus prioritaire.
    if task::take_need_resched() {
        task::yield_now();
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
        PPOLL => file::sys_poll(args[0], args[1] as usize, -1),
        SELECT | PSELECT6 => file::sys_select(args[0] as i32, args[1], args[2], args[3], args[4]),
        EPOLL_CREATE | EPOLL_CREATE1 => file::sys_epoll_create(),
        EPOLL_CTL => file::sys_epoll_ctl(args[0] as i32, args[1] as u32, args[2] as i32, args[3]),
        EPOLL_WAIT | EPOLL_PWAIT => {
            file::sys_epoll_wait(args[0] as i32, args[1], args[2] as usize, args[3] as i32)
        }
        EVENTFD | EVENTFD2 => file::sys_eventfd(),
        FSYNC | FDATASYNC => 0,
        UMASK => 0o022,
        FCHMOD | FCHOWN | CHMOD | CHOWN => 0,

        // --- Memoire ---
        BRK => mem::sys_brk(args[0]),
        MMAP => mem::sys_mmap(args[0], args[1], args[2] as u32, args[3] as u32, args[4] as i32, args[5]),
        MUNMAP => mem::sys_munmap(args[0], args[1]),
        MPROTECT => mem::sys_mprotect(args[0], args[1], args[2] as u32),
        MREMAP => mem::sys_mremap(args[0], args[1], args[2], args[3] as u32),
        MADVISE | MSYNC | MLOCK | MUNLOCK | MLOCKALL | MUNLOCKALL => 0,

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
        FORK | VFORK => -errno::ENOSYS,
        EXECVE => -errno::ENOSYS,
        EXIT => task::exit_current(args[0] as i32),
        EXIT_GROUP => task::exit_group(args[0] as i32),
        WAIT4 => -errno::ECHILD,
        SCHED_YIELD => {
            task::yield_now();
            0
        }
        FUTEX => sys_futex(args),
        NANOSLEEP => sys_nanosleep(args[0], args[1]),
        CLOCK_NANOSLEEP => sys_nanosleep(args[2], args[3]),
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
        KILL | TKILL => sys_kill(args[1]),
        TGKILL => sys_kill(args[2]),

        // --- Signaux (modele minimal) ---
        RT_SIGACTION | RT_SIGPROCMASK | SIGALTSTACK | RT_SIGSUSPEND => 0,
        RT_SIGRETURN => 0,

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
            let seconds = unix_time();
            if args[0] != 0 {
                user_write(args[0], &seconds.to_le_bytes());
                user_write(args[0] + 8, &0u64.to_le_bytes());
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

/// Secondes depuis l'epoch Unix, calculees depuis la RTC.
pub fn unix_time() -> u64 {
    let now = crate::arch::x86_64::rtc::now_utc();
    days_from_civil(now.year as i64, now.month as i64, now.day as i64) as u64 * 86400
        + now.hour as u64 * 3600
        + now.minute as u64 * 60
        + now.second as u64
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
    const CLOCK_MONOTONIC: i32 = 1;
    let (seconds, nanos) = match clock {
        CLOCK_REALTIME => (unix_time(), 0u64),
        _ => {
            // Monotone : le TSC calibre donne bien mieux que les 18,2 Hz du PIT,
            // ce dont depend toute animation d'interface.
            let ms = crate::kernel::timer::cycles_to_ms(crate::kernel::timer::cycles_since_boot());
            (ms / 1000, (ms % 1000) * 1_000_000)
        }
    };
    let _ = CLOCK_MONOTONIC;
    if out == 0 {
        return -errno::EFAULT;
    }
    if !user_write(out, &seconds.to_le_bytes()) || !user_write(out + 8, &nanos.to_le_bytes()) {
        return -errno::EFAULT;
    }
    0
}

/// Convertit un `struct timespec` utilisateur en ticks du timer.
fn timespec_ticks(addr: u64) -> Option<u64> {
    if addr == 0 {
        return None;
    }
    let seconds = user_read_u64(addr)?;
    let nanos = user_read_u64(addr + 8)?;
    let ms = seconds * 1000 + nanos / 1_000_000;
    Some((ms * crate::kernel::timer::TICKS_PER_SECOND).div_ceil(1000))
}

/// `nanosleep` / `clock_nanosleep`.
fn sys_nanosleep(request: u64, remain: u64) -> i64 {
    let ticks = timespec_ticks(request).unwrap_or(0);
    if ticks == 0 {
        // Sommeil sous-tick : on cede simplement le CPU.
        task::yield_now();
    } else {
        task::sleep_ticks(ticks);
    }
    if remain != 0 {
        user_write(remain, &0u64.to_le_bytes());
        user_write(remain + 8, &0u64.to_le_bytes());
    }
    0
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
    let operation = (args[1] as u32) & !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME);
    let expected = args[2] as u32;

    match operation {
        FUTEX_WAIT | FUTEX_WAIT_BITSET => {
            let timeout = timespec_ticks(args[3]).unwrap_or(0);
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
