//! Codes d'erreur POSIX, valeurs Linux/x86-64.
//!
//! Une valeur de retour negative d'appel systeme est `-errno` : c'est ainsi que
//! la libc distingue une erreur d'un resultat valide. Les valeurs doivent etre
//! exactement celles de Linux, sinon `errno` remonterait un message faux (ou,
//! pire, `EAGAIN` serait pris pour `ENOMEM` dans une boucle de retry).

pub const EPERM: i64 = 1;
pub const ENOENT: i64 = 2;
pub const ESRCH: i64 = 3;
pub const EINTR: i64 = 4;
pub const EIO: i64 = 5;
pub const ENXIO: i64 = 6;
pub const E2BIG: i64 = 7;
pub const ENOEXEC: i64 = 8;
pub const EBADF: i64 = 9;
pub const ECHILD: i64 = 10;
pub const EAGAIN: i64 = 11;
pub const ENOMEM: i64 = 12;
pub const EACCES: i64 = 13;
pub const EFAULT: i64 = 14;
pub const EBUSY: i64 = 16;
pub const EEXIST: i64 = 17;
pub const EXDEV: i64 = 18;
pub const ENODEV: i64 = 19;
pub const ENOTDIR: i64 = 20;
pub const EISDIR: i64 = 21;
pub const EINVAL: i64 = 22;
pub const ENFILE: i64 = 23;
pub const EMFILE: i64 = 24;
pub const ENOTTY: i64 = 25;
pub const EFBIG: i64 = 27;
pub const ENOSPC: i64 = 28;
pub const ESPIPE: i64 = 29;
pub const EROFS: i64 = 30;
pub const EMLINK: i64 = 31;
pub const EPIPE: i64 = 32;
pub const ERANGE: i64 = 34;
pub const ENAMETOOLONG: i64 = 36;
pub const ENOSYS: i64 = 38;
pub const ENOTEMPTY: i64 = 39;
pub const ELOOP: i64 = 40;
pub const ENODATA: i64 = 61;
pub const EPROTO: i64 = 71;
pub const EOVERFLOW: i64 = 75;
pub const ENOTSUP: i64 = 95;
pub const ETIMEDOUT: i64 = 110;

/// Libelle court d'un code d'erreur (diagnostic).
pub fn name(code: i64) -> &'static str {
    match code.abs() {
        EPERM => "EPERM",
        ENOENT => "ENOENT",
        EINTR => "EINTR",
        EIO => "EIO",
        EBADF => "EBADF",
        EAGAIN => "EAGAIN",
        ENOMEM => "ENOMEM",
        EACCES => "EACCES",
        EFAULT => "EFAULT",
        EEXIST => "EEXIST",
        ENOTDIR => "ENOTDIR",
        EISDIR => "EISDIR",
        EINVAL => "EINVAL",
        ENOTTY => "ENOTTY",
        ENOSPC => "ENOSPC",
        ESPIPE => "ESPIPE",
        EPIPE => "EPIPE",
        ENOSYS => "ENOSYS",
        ETIMEDOUT => "ETIMEDOUT",
        _ => "E?",
    }
}
