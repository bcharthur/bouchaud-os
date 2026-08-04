//! Numeros d'appels systeme Linux x86-64.
//!
//! Ces valeurs sont figees par l'ABI Linux : elles sont compilees en dur dans
//! le musl que l'on veut executer, il n'y a donc rien a choisir ici — seulement
//! a respecter. Source : `arch/x86/entry/syscalls/syscall_64.tbl`.

pub const READ: u64 = 0;
pub const WRITE: u64 = 1;
pub const OPEN: u64 = 2;
pub const CLOSE: u64 = 3;
pub const STAT: u64 = 4;
pub const FSTAT: u64 = 5;
pub const LSTAT: u64 = 6;
pub const POLL: u64 = 7;
pub const LSEEK: u64 = 8;
pub const MMAP: u64 = 9;
pub const MPROTECT: u64 = 10;
pub const MUNMAP: u64 = 11;
pub const BRK: u64 = 12;
pub const RT_SIGACTION: u64 = 13;
pub const RT_SIGPROCMASK: u64 = 14;
pub const RT_SIGRETURN: u64 = 15;
pub const IOCTL: u64 = 16;
pub const PREAD64: u64 = 17;
pub const PWRITE64: u64 = 18;
pub const READV: u64 = 19;
pub const WRITEV: u64 = 20;
pub const ACCESS: u64 = 21;
pub const PIPE: u64 = 22;
pub const SELECT: u64 = 23;
pub const SCHED_YIELD: u64 = 24;
pub const MREMAP: u64 = 25;
pub const MSYNC: u64 = 26;
pub const MADVISE: u64 = 28;
pub const DUP: u64 = 32;
pub const DUP2: u64 = 33;
pub const NANOSLEEP: u64 = 35;
pub const GETPID: u64 = 39;
pub const SENDFILE: u64 = 40;
pub const CLONE: u64 = 56;
pub const FORK: u64 = 57;
pub const VFORK: u64 = 58;
pub const EXECVE: u64 = 59;
pub const EXIT: u64 = 60;
pub const WAIT4: u64 = 61;
pub const KILL: u64 = 62;
pub const UNAME: u64 = 63;
pub const FCNTL: u64 = 72;
pub const FSYNC: u64 = 74;
pub const FDATASYNC: u64 = 75;
pub const FTRUNCATE: u64 = 77;
pub const GETDENTS: u64 = 78;
pub const GETCWD: u64 = 79;
pub const CHDIR: u64 = 80;
pub const FCHDIR: u64 = 81;
pub const RENAME: u64 = 82;
pub const MKDIR: u64 = 83;
pub const RMDIR: u64 = 84;
pub const CREAT: u64 = 85;
pub const UNLINK: u64 = 87;
pub const READLINK: u64 = 89;
pub const CHMOD: u64 = 90;
pub const FCHMOD: u64 = 91;
pub const CHOWN: u64 = 92;
pub const FCHOWN: u64 = 93;
pub const UMASK: u64 = 95;
pub const GETTIMEOFDAY: u64 = 96;
pub const GETRLIMIT: u64 = 97;
pub const SYSINFO: u64 = 99;
pub const TIMES: u64 = 100;
pub const GETUID: u64 = 102;
pub const SYSLOG: u64 = 103;
pub const GETGID: u64 = 104;
pub const SETUID: u64 = 105;
pub const SETGID: u64 = 106;
pub const GETEUID: u64 = 107;
pub const GETEGID: u64 = 108;
pub const SETPGID: u64 = 109;
pub const GETPPID: u64 = 110;
pub const GETPGRP: u64 = 111;
pub const SETSID: u64 = 112;
pub const SCHED_SETPARAM: u64 = 142;
pub const SCHED_SETSCHEDULER: u64 = 144;
pub const SCHED_GETSCHEDULER: u64 = 145;
pub const SCHED_GET_PRIORITY_MAX: u64 = 146;
pub const SCHED_GET_PRIORITY_MIN: u64 = 147;
pub const MLOCK: u64 = 149;
pub const MUNLOCK: u64 = 150;
pub const MLOCKALL: u64 = 151;
pub const MUNLOCKALL: u64 = 152;
pub const PRCTL: u64 = 157;
pub const ARCH_PRCTL: u64 = 158;
pub const GETTID: u64 = 186;
pub const FUTEX: u64 = 202;
pub const SCHED_SETAFFINITY: u64 = 203;
pub const SCHED_GETAFFINITY: u64 = 204;
pub const GETDENTS64: u64 = 217;
pub const SET_TID_ADDRESS: u64 = 218;
pub const TIME: u64 = 201;
pub const EPOLL_CREATE: u64 = 213;
pub const CLOCK_GETTIME: u64 = 228;
pub const CLOCK_GETRES: u64 = 229;
pub const CLOCK_NANOSLEEP: u64 = 230;
pub const EXIT_GROUP: u64 = 231;
pub const EPOLL_WAIT: u64 = 232;
pub const EPOLL_CTL: u64 = 233;
pub const TGKILL: u64 = 234;
pub const TKILL: u64 = 200;
pub const SET_ROBUST_LIST: u64 = 273;
pub const GET_ROBUST_LIST: u64 = 274;
pub const OPENAT: u64 = 257;
pub const MKDIRAT: u64 = 258;
pub const NEWFSTATAT: u64 = 262;
pub const UNLINKAT: u64 = 263;
pub const READLINKAT: u64 = 267;
pub const FACCESSAT: u64 = 269;
pub const PSELECT6: u64 = 270;
pub const PPOLL: u64 = 271;
pub const EPOLL_PWAIT: u64 = 281;
pub const EVENTFD: u64 = 284;
pub const EVENTFD2: u64 = 290;
pub const EPOLL_CREATE1: u64 = 291;
pub const DUP3: u64 = 292;
pub const PIPE2: u64 = 293;
pub const PRLIMIT64: u64 = 302;
pub const GETRANDOM: u64 = 318;
pub const MEMBARRIER: u64 = 324;
pub const STATX: u64 = 332;
pub const RSEQ: u64 = 334;
pub const SETRLIMIT: u64 = 160;
pub const SIGALTSTACK: u64 = 131;
pub const RT_SIGSUSPEND: u64 = 130;
pub const GETPGID: u64 = 121;
pub const GETSID: u64 = 124;
pub const TIMERFD_CREATE: u64 = 283;
pub const TIMERFD_SETTIME: u64 = 286;
pub const TIMERFD_GETTIME: u64 = 287;

// --- Sockets ---
pub const SOCKET: u64 = 41;
pub const CONNECT: u64 = 42;
pub const ACCEPT: u64 = 43;
pub const SENDTO: u64 = 44;
pub const RECVFROM: u64 = 45;
pub const SENDMSG: u64 = 46;
pub const RECVMSG: u64 = 47;
pub const SHUTDOWN: u64 = 48;
pub const BIND: u64 = 49;
pub const LISTEN: u64 = 50;
pub const GETSOCKNAME: u64 = 51;
pub const GETPEERNAME: u64 = 52;
pub const SOCKETPAIR: u64 = 53;
pub const SETSOCKOPT: u64 = 54;
pub const GETSOCKOPT: u64 = 55;
pub const ACCEPT4: u64 = 288;

// --- Signaux ---
pub const PAUSE: u64 = 34;
pub const ALARM: u64 = 37;
pub const RT_SIGPENDING: u64 = 127;
pub const SETITIMER: u64 = 38;
pub const GETITIMER: u64 = 36;

/// Nom symbolique d'un numero (trace et diagnostic).
pub fn name(number: u64) -> &'static str {
    match number {
        READ => "read",
        WRITE => "write",
        OPEN => "open",
        CLOSE => "close",
        STAT => "stat",
        FSTAT => "fstat",
        LSTAT => "lstat",
        POLL => "poll",
        LSEEK => "lseek",
        MMAP => "mmap",
        MPROTECT => "mprotect",
        MUNMAP => "munmap",
        BRK => "brk",
        RT_SIGACTION => "rt_sigaction",
        RT_SIGPROCMASK => "rt_sigprocmask",
        IOCTL => "ioctl",
        PREAD64 => "pread64",
        PWRITE64 => "pwrite64",
        READV => "readv",
        WRITEV => "writev",
        ACCESS => "access",
        PIPE => "pipe",
        SELECT => "select",
        SCHED_YIELD => "sched_yield",
        MREMAP => "mremap",
        MADVISE => "madvise",
        DUP => "dup",
        DUP2 => "dup2",
        NANOSLEEP => "nanosleep",
        GETPID => "getpid",
        CLONE => "clone",
        FORK => "fork",
        EXECVE => "execve",
        EXIT => "exit",
        WAIT4 => "wait4",
        KILL => "kill",
        UNAME => "uname",
        FCNTL => "fcntl",
        FTRUNCATE => "ftruncate",
        GETDENTS64 => "getdents64",
        GETCWD => "getcwd",
        CHDIR => "chdir",
        RENAME => "rename",
        MKDIR => "mkdir",
        UNLINK => "unlink",
        READLINK => "readlink",
        GETTIMEOFDAY => "gettimeofday",
        GETRLIMIT => "getrlimit",
        SYSINFO => "sysinfo",
        GETUID => "getuid",
        GETGID => "getgid",
        GETEUID => "geteuid",
        GETEGID => "getegid",
        PRCTL => "prctl",
        ARCH_PRCTL => "arch_prctl",
        GETTID => "gettid",
        FUTEX => "futex",
        SCHED_GETAFFINITY => "sched_getaffinity",
        SET_TID_ADDRESS => "set_tid_address",
        TIME => "time",
        CLOCK_GETTIME => "clock_gettime",
        CLOCK_GETRES => "clock_getres",
        CLOCK_NANOSLEEP => "clock_nanosleep",
        EXIT_GROUP => "exit_group",
        EPOLL_WAIT => "epoll_wait",
        EPOLL_CTL => "epoll_ctl",
        TGKILL => "tgkill",
        TKILL => "tkill",
        SET_ROBUST_LIST => "set_robust_list",
        OPENAT => "openat",
        NEWFSTATAT => "newfstatat",
        UNLINKAT => "unlinkat",
        READLINKAT => "readlinkat",
        FACCESSAT => "faccessat",
        PPOLL => "ppoll",
        EPOLL_PWAIT => "epoll_pwait",
        EVENTFD => "eventfd",
        EVENTFD2 => "eventfd2",
        TIMERFD_CREATE => "timerfd_create",
        TIMERFD_SETTIME => "timerfd_settime",
        TIMERFD_GETTIME => "timerfd_gettime",
        EPOLL_CREATE1 => "epoll_create1",
        DUP3 => "dup3",
        PIPE2 => "pipe2",
        PRLIMIT64 => "prlimit64",
        GETRANDOM => "getrandom",
        STATX => "statx",
        RSEQ => "rseq",
        MEMBARRIER => "membarrier",
        SIGALTSTACK => "sigaltstack",
        RT_SIGRETURN => "rt_sigreturn",
        RT_SIGSUSPEND => "rt_sigsuspend",
        RT_SIGPENDING => "rt_sigpending",
        PAUSE => "pause",
        ALARM => "alarm",
        VFORK => "vfork",
        SOCKET => "socket",
        CONNECT => "connect",
        ACCEPT => "accept",
        SENDTO => "sendto",
        RECVFROM => "recvfrom",
        SENDMSG => "sendmsg",
        RECVMSG => "recvmsg",
        SETITIMER => "setitimer",
        GETITIMER => "getitimer",
        SHUTDOWN => "shutdown",
        BIND => "bind",
        LISTEN => "listen",
        GETSOCKNAME => "getsockname",
        GETPEERNAME => "getpeername",
        SOCKETPAIR => "socketpair",
        SETSOCKOPT => "setsockopt",
        GETSOCKOPT => "getsockopt",
        ACCEPT4 => "accept4",
        GETPPID => "getppid",
        MSYNC => "msync",
        _ => "?",
    }
}

/// Appels systeme implementes, regroupes par famille (commande `syscalls`).
pub fn print_implemented() {
    let groups: [(&str, &[u64]); 9] = [
        ("E/S", &[READ, WRITE, OPEN, OPENAT, CLOSE, LSEEK, READV, WRITEV, PREAD64, PWRITE64, DUP, DUP2, DUP3, PIPE, PIPE2, FCNTL, IOCTL, FTRUNCATE]),
        ("fichiers", &[STAT, LSTAT, FSTAT, NEWFSTATAT, STATX, ACCESS, FACCESSAT, READLINK, READLINKAT, GETDENTS64, GETCWD, CHDIR, MKDIR, UNLINK, RENAME]),
        ("attente", &[POLL, PPOLL, SELECT, EPOLL_CREATE1, EPOLL_CTL, EPOLL_WAIT, EVENTFD2, TIMERFD_CREATE, TIMERFD_SETTIME, TIMERFD_GETTIME]),
        ("memoire", &[BRK, MMAP, MUNMAP, MPROTECT, MREMAP, MADVISE, MSYNC]),
        ("taches", &[CLONE, FORK, VFORK, EXECVE, WAIT4, EXIT, EXIT_GROUP, GETPID, GETPPID, GETTID, SET_TID_ADDRESS, SCHED_YIELD, FUTEX, NANOSLEEP, CLOCK_NANOSLEEP, ARCH_PRCTL, PRCTL]),
        ("signaux", &[RT_SIGACTION, RT_SIGPROCMASK, RT_SIGRETURN, RT_SIGSUSPEND, RT_SIGPENDING, SIGALTSTACK, KILL, TKILL, TGKILL, PAUSE, ALARM, SETITIMER, GETITIMER]),
        ("reseau", &[SOCKET, CONNECT, BIND, SENDTO, RECVFROM, SENDMSG, RECVMSG, SHUTDOWN, GETSOCKNAME, GETPEERNAME, SOCKETPAIR, SETSOCKOPT, GETSOCKOPT]),
        ("temps", &[CLOCK_GETTIME, CLOCK_GETRES, GETTIMEOFDAY, TIME]),
        ("systeme", &[UNAME, SYSINFO, GETRLIMIT, PRLIMIT64, GETRANDOM, GETUID, GETEUID, GETGID, GETEGID, SCHED_GETAFFINITY]),
    ];
    let mut total = 0;
    for (title, numbers) in groups.iter() {
        crate::print!("  {:<9}: ", title);
        for (index, number) in numbers.iter().enumerate() {
            if index > 0 {
                crate::print!(" ");
            }
            crate::print!("{}", name(*number));
        }
        crate::println!("");
        total += numbers.len();
    }
    crate::println!("  total    : {} appels implementes", total);
}
