//! Entrees/sorties POSIX : descripteurs, RAMFS, peripheriques, attente.
//!
//! Le RAMFS de l'OS est expose sous les structures binaires exactes de Linux
//! (`struct stat`, `struct linux_dirent64`, `struct statx`) : une libc les
//! deserialise champ par champ, un decalage d'octet suffit a faire croire a un
//! fichier vide ou a un type inattendu.
//!
//! Les ioctls implementes sont ceux qu'une pile graphique interroge au
//! demarrage : `FBIOGET_VSCREENINFO`/`FBIOGET_FSCREENINFO` (geometrie et pas de
//! ligne du framebuffer), `TIOCGWINSZ` et `TCGETS` (la libc s'en sert pour
//! decider si la sortie est un terminal), et les `EVIOC*` de base pour evdev.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::drivers::keyboard::{self, Key};
use crate::fs::ramfs::{self, NodeKind};
use crate::kernel::abi::{errno, user_read, user_read_u64, user_write};
use crate::kernel::fd::{device_for_path, FdKind, FileDesc};
use crate::kernel::task;

/// `AT_FDCWD` : chemin relatif au repertoire courant.
pub const AT_FDCWD: i32 = -100;

const O_ACCMODE: u32 = 3;
const O_WRONLY: u32 = 1;
const O_RDWR: u32 = 2;
const O_CREAT: u32 = 0o100;
const O_EXCL: u32 = 0o200;
const O_TRUNC: u32 = 0o1000;
const O_APPEND: u32 = 0o2000;
const O_NONBLOCK: u32 = 0o4000;
const O_DIRECTORY: u32 = 0o200000;
const O_CLOEXEC: u32 = 0o2000000;

const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o40000;
const S_IFCHR: u32 = 0o20000;
const S_IFIFO: u32 = 0o10000;

// --- Chemins ----------------------------------------------------------------

/// Rend un chemin absolu a partir du repertoire courant du processus.
fn absolute(path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    let cwd = task::current_process().borrow().cwd;
    let mut base = ramfs::path_string(ramfs::fs(), cwd);
    if !base.ends_with('/') {
        base.push('/');
    }
    base.push_str(path);
    base
}

/// Resout un chemin utilisateur en index de nœud RAMFS.
fn resolve(path: &str) -> Option<usize> {
    let cwd = task::current_process().borrow().cwd;
    ramfs::fs().resolve(path, cwd)
}

// --- Lecture / ecriture ------------------------------------------------------

/// Ecrit sur la console : ecran (VGA ou bureau) et port serie de debug.
fn console_write(data: &[u8]) {
    for &byte in data {
        crate::serial_print!("{}", byte as char);
    }
    let text = String::from_utf8_lossy(data);
    crate::print!("{}", text);
}

/// Lit la console (clavier). Bloque jusqu'a disposer d'au moins un octet.
fn console_read(max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        while let Some(key) = keyboard::try_key() {
            match key {
                Key::Char(byte) => out.push(byte),
                Key::Enter => out.push(b'\n'),
                Key::Tab => out.push(b'\t'),
                Key::Backspace => {
                    out.pop();
                }
                _ => {}
            }
            if out.len() >= max {
                return out;
            }
        }
        if !out.is_empty() {
            return out;
        }
        // Rien a lire : on rend la main plutot que de monopoliser le CPU.
        task::yield_now();
        crate::arch::x86_64::cpu::hlt();
    }
}

/// `read`.
pub fn sys_read(fd: i32, buffer: u64, count: usize) -> i64 {
    if count == 0 {
        return 0;
    }
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return -errno::EBADF,
    };

    match kind {
        FdKind::Console => {
            let data = console_read(count);
            if !user_write(buffer, &data) {
                return -errno::EFAULT;
            }
            data.len() as i64
        }
        FdKind::Null => 0,
        FdKind::Zero => {
            let zeros = alloc::vec![0u8; count];
            if user_write(buffer, &zeros) { count as i64 } else { -errno::EFAULT }
        }
        FdKind::Random => {
            let mut data = alloc::vec![0u8; count.min(4096)];
            crate::net::security::tls::rng::fill(&mut data);
            let len = data.len();
            if user_write(buffer, &data) { len as i64 } else { -errno::EFAULT }
        }
        FdKind::File(node) => {
            let offset = process.borrow().files.get(fd).map(|d| d.offset).unwrap_or(0);
            let fs = ramfs::fs();
            let content = &fs.nodes[node].content;
            if offset >= content.len() {
                return 0;
            }
            let end = core::cmp::min(content.len(), offset + count);
            let slice = content[offset..end].to_vec();
            if !user_write(buffer, &slice) {
                return -errno::EFAULT;
            }
            if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
                desc.offset = end;
            }
            (end - offset) as i64
        }
        FdKind::Dir(_) => -errno::EISDIR,
        FdKind::Framebuffer => -errno::EINVAL,
        FdKind::InputKeyboard => read_input_keyboard(buffer, count),
        FdKind::InputMouse => read_input_mouse(buffer, count),
        FdKind::Pipe(shared, readable) => {
            if !readable {
                return -errno::EBADF;
            }
            let mut guard = shared.borrow_mut();
            if guard.is_empty() {
                return -errno::EAGAIN;
            }
            let len = core::cmp::min(count, guard.len());
            let data: Vec<u8> = guard.drain(..len).collect();
            drop(guard);
            if user_write(buffer, &data) { len as i64 } else { -errno::EFAULT }
        }
        FdKind::Epoll(_) => -errno::EINVAL,
    }
}

/// `write`.
pub fn sys_write(fd: i32, buffer: u64, count: usize) -> i64 {
    if count == 0 {
        return 0;
    }
    let data = match user_read(buffer, count) {
        Some(data) => data,
        None => return -errno::EFAULT,
    };
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return -errno::EBADF,
    };

    match kind {
        FdKind::Console => {
            console_write(&data);
            count as i64
        }
        FdKind::Null => count as i64,
        FdKind::Zero => count as i64,
        FdKind::Random => count as i64,
        FdKind::File(node) => {
            let (offset, append) = {
                let borrowed = process.borrow();
                let desc = borrowed.files.get(fd).unwrap();
                (desc.offset, desc.flags & O_APPEND != 0)
            };
            let fs = ramfs::fs();
            let content = &mut fs.nodes[node].content;
            let start = if append { content.len() } else { offset };
            if start + data.len() > ramfs::MAX_FILE_SIZE {
                return -errno::EFBIG;
            }
            if content.len() < start {
                content.resize(start, 0);
            }
            let end = start + data.len();
            if content.len() < end {
                content.resize(end, 0);
            }
            content[start..end].copy_from_slice(&data);
            if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
                desc.offset = end;
            }
            data.len() as i64
        }
        FdKind::Dir(_) => -errno::EISDIR,
        FdKind::Framebuffer => -errno::EINVAL,
        FdKind::InputKeyboard | FdKind::InputMouse => count as i64,
        FdKind::Pipe(shared, readable) => {
            if readable {
                return -errno::EBADF;
            }
            shared.borrow_mut().extend_from_slice(&data);
            count as i64
        }
        FdKind::Epoll(_) => -errno::EINVAL,
    }
}

/// `readv` : lectures vectorisees (`struct iovec { void *base; size_t len; }`).
pub fn sys_readv(fd: i32, iov: u64, count: usize) -> i64 {
    let mut total = 0i64;
    for index in 0..count {
        let base = match user_read_u64(iov + (index * 16) as u64) {
            Some(value) => value,
            None => return -errno::EFAULT,
        };
        let len = match user_read_u64(iov + (index * 16) as u64 + 8) {
            Some(value) => value as usize,
            None => return -errno::EFAULT,
        };
        if len == 0 {
            continue;
        }
        let result = sys_read(fd, base, len);
        if result < 0 {
            return if total > 0 { total } else { result };
        }
        total += result;
        if (result as usize) < len {
            break;
        }
    }
    total
}

/// `writev` : le chemin qu'emprunte tout `printf` de musl.
pub fn sys_writev(fd: i32, iov: u64, count: usize) -> i64 {
    let mut total = 0i64;
    for index in 0..count {
        let base = match user_read_u64(iov + (index * 16) as u64) {
            Some(value) => value,
            None => return -errno::EFAULT,
        };
        let len = match user_read_u64(iov + (index * 16) as u64 + 8) {
            Some(value) => value as usize,
            None => return -errno::EFAULT,
        };
        if len == 0 {
            continue;
        }
        let result = sys_write(fd, base, len);
        if result < 0 {
            return if total > 0 { total } else { result };
        }
        total += result;
    }
    total
}

/// `pread64`.
pub fn sys_pread(fd: i32, buffer: u64, count: usize, offset: i64) -> i64 {
    let process = task::current_process();
    let saved = match process.borrow().files.get(fd) {
        Some(desc) => desc.offset,
        None => return -errno::EBADF,
    };
    if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
        desc.offset = offset.max(0) as usize;
    }
    let result = sys_read(fd, buffer, count);
    if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
        desc.offset = saved;
    }
    result
}

/// `pwrite64`.
pub fn sys_pwrite(fd: i32, buffer: u64, count: usize, offset: i64) -> i64 {
    let process = task::current_process();
    let saved = match process.borrow().files.get(fd) {
        Some(desc) => desc.offset,
        None => return -errno::EBADF,
    };
    if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
        desc.offset = offset.max(0) as usize;
    }
    let result = sys_write(fd, buffer, count);
    if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
        desc.offset = saved;
    }
    result
}

// --- Ouverture / fermeture ---------------------------------------------------

/// `openat` (et `open`, avec `AT_FDCWD`).
pub fn sys_openat(dirfd: i32, path_addr: u64, flags: u32, mode: u32) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => path,
        None => return -errno::EFAULT,
    };
    let path = if dirfd != AT_FDCWD && !path.starts_with('/') {
        // Chemin relatif a un descripteur de repertoire ouvert.
        let process = task::current_process();
        let node = match process.borrow().files.get(dirfd) {
            Some(desc) => match desc.kind {
                FdKind::Dir(node) | FdKind::File(node) => Some(node),
                _ => None,
            },
            None => return -errno::EBADF,
        };
        match node {
            Some(node) => {
                let mut base = ramfs::path_string(ramfs::fs(), node);
                if !base.ends_with('/') {
                    base.push('/');
                }
                base.push_str(&path);
                base
            }
            None => absolute(&path),
        }
    } else {
        absolute(&path)
    };

    let process = task::current_process();

    // Peripheriques synthetiques d'abord : ils n'existent pas dans le RAMFS.
    if let Some(kind) = device_for_path(&path) {
        // Ouvrir le framebuffer, c'est demander un ecran : on bascule la carte
        // en mode graphique lineaire si ce n'est pas deja fait. Sans cela,
        // `mmap` renverrait des pages valides mais invisibles (la carte serait
        // restee en mode texte).
        if matches!(kind, FdKind::Framebuffer) && !crate::drivers::gfx::is_active() {
            crate::drivers::gfx::enter();
        }
        let mut desc = FileDesc::new(kind);
        desc.flags = flags;
        desc.cloexec = flags & O_CLOEXEC != 0;
        let fd = process.borrow_mut().files.insert(desc);
        return fd as i64;
    }

    let existing = resolve(&path);
    let node = match existing {
        Some(node) => {
            if flags & (O_CREAT | O_EXCL) == (O_CREAT | O_EXCL) {
                return -errno::EEXIST;
            }
            node
        }
        None => {
            if flags & O_CREAT == 0 {
                return -errno::ENOENT;
            }
            let cwd = process.borrow().cwd;
            let fs = ramfs::fs();
            let (parent, name) = match fs.resolve_parent_name(&path, cwd) {
                Some(value) => value,
                None => return -errno::ENOENT,
            };
            match fs.touch_at(parent, name) {
                Ok(node) => {
                    fs.nodes[node].mode = (mode & 0o777) as u16;
                    node
                }
                Err(_) => return -errno::ENOSPC,
            }
        }
    };

    let fs = ramfs::fs();
    let is_dir = fs.nodes[node].kind == NodeKind::Dir;
    if flags & O_DIRECTORY != 0 && !is_dir {
        return -errno::ENOTDIR;
    }
    let access = flags & O_ACCMODE;
    if is_dir && (access == O_WRONLY || access == O_RDWR) {
        return -errno::EISDIR;
    }
    if flags & O_TRUNC != 0 && !is_dir {
        fs.nodes[node].content.clear();
    }

    let mut desc = FileDesc::new(if is_dir { FdKind::Dir(node) } else { FdKind::File(node) });
    desc.flags = flags;
    desc.cloexec = flags & O_CLOEXEC != 0;
    if flags & O_APPEND != 0 {
        desc.offset = fs.nodes[node].content.len();
    }
    let fd = process.borrow_mut().files.insert(desc);
    fd as i64
}

/// `close`.
pub fn sys_close(fd: i32) -> i64 {
    let process = task::current_process();
    let closed = process.borrow_mut().files.close(fd);
    if closed { 0 } else { -errno::EBADF }
}

/// `lseek`.
pub fn sys_lseek(fd: i32, offset: i64, whence: u32) -> i64 {
    const SEEK_SET: u32 = 0;
    const SEEK_CUR: u32 = 1;
    const SEEK_END: u32 = 2;
    let process = task::current_process();
    let mut borrowed = process.borrow_mut();
    let desc = match borrowed.files.get_mut(fd) {
        Some(desc) => desc,
        None => return -errno::EBADF,
    };
    let size = match desc.kind {
        FdKind::File(node) | FdKind::Dir(node) => ramfs::fs().nodes[node].content.len() as i64,
        FdKind::Console | FdKind::Pipe(_, _) => return -errno::ESPIPE,
        _ => 0,
    };
    let base = match whence {
        SEEK_SET => 0,
        SEEK_CUR => desc.offset as i64,
        SEEK_END => size,
        _ => return -errno::EINVAL,
    };
    let target = base + offset;
    if target < 0 {
        return -errno::EINVAL;
    }
    desc.offset = target as usize;
    target
}

/// `dup`.
pub fn sys_dup(fd: i32) -> i64 {
    let process = task::current_process();
    let mut borrowed = process.borrow_mut();
    let desc = match borrowed.files.get(fd) {
        Some(desc) => desc.clone(),
        None => return -errno::EBADF,
    };
    borrowed.files.insert(desc) as i64
}

/// `dup2` / `dup3`.
pub fn sys_dup2(old: i32, new: i32) -> i64 {
    if new < 0 {
        return -errno::EBADF;
    }
    if old == new {
        return new as i64;
    }
    let process = task::current_process();
    let mut borrowed = process.borrow_mut();
    let desc = match borrowed.files.get(old) {
        Some(desc) => desc.clone(),
        None => return -errno::EBADF,
    };
    borrowed.files.set(new as usize, desc);
    new as i64
}

/// `pipe` / `pipe2`.
pub fn sys_pipe(addr: u64, flags: u32) -> i64 {
    use alloc::rc::Rc;
    use core::cell::RefCell;
    let shared = Rc::new(RefCell::new(Vec::new()));
    let process = task::current_process();
    let mut borrowed = process.borrow_mut();
    let mut read_end = FileDesc::new(FdKind::Pipe(shared.clone(), true));
    let mut write_end = FileDesc::new(FdKind::Pipe(shared, false));
    read_end.flags = flags;
    write_end.flags = flags;
    read_end.cloexec = flags & O_CLOEXEC != 0;
    write_end.cloexec = flags & O_CLOEXEC != 0;
    let read_fd = borrowed.files.insert(read_end);
    let write_fd = borrowed.files.insert(write_end);
    drop(borrowed);
    if !user_write(addr, &read_fd.to_le_bytes()) || !user_write(addr + 4, &write_fd.to_le_bytes()) {
        return -errno::EFAULT;
    }
    0
}

/// `fcntl`.
pub fn sys_fcntl(fd: i32, command: u32, arg: u64) -> i64 {
    const F_DUPFD: u32 = 0;
    const F_GETFD: u32 = 1;
    const F_SETFD: u32 = 2;
    const F_GETFL: u32 = 3;
    const F_SETFL: u32 = 4;
    const F_DUPFD_CLOEXEC: u32 = 1030;
    const FD_CLOEXEC: u64 = 1;

    let process = task::current_process();
    let mut borrowed = process.borrow_mut();
    match command {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            let mut desc = match borrowed.files.get(fd) {
                Some(desc) => desc.clone(),
                None => return -errno::EBADF,
            };
            desc.cloexec = command == F_DUPFD_CLOEXEC;
            borrowed.files.insert_at_least(desc, arg as usize) as i64
        }
        F_GETFD => match borrowed.files.get(fd) {
            Some(desc) => {
                if desc.cloexec { FD_CLOEXEC as i64 } else { 0 }
            }
            None => -errno::EBADF,
        },
        F_SETFD => match borrowed.files.get_mut(fd) {
            Some(desc) => {
                desc.cloexec = arg & FD_CLOEXEC != 0;
                0
            }
            None => -errno::EBADF,
        },
        F_GETFL => match borrowed.files.get(fd) {
            Some(desc) => desc.flags as i64,
            None => -errno::EBADF,
        },
        F_SETFL => match borrowed.files.get_mut(fd) {
            Some(desc) => {
                desc.flags = arg as u32;
                0
            }
            None => -errno::EBADF,
        },
        _ => 0,
    }
}

// --- Metadonnees -------------------------------------------------------------

/// Remplit un `struct stat` x86-64 (144 octets) pour un nœud RAMFS.
fn fill_stat(node: usize) -> [u8; 144] {
    let fs = ramfs::fs();
    let entry = &fs.nodes[node];
    let mode = (entry.mode as u32 & 0o7777)
        | if entry.kind == NodeKind::Dir { S_IFDIR } else { S_IFREG };
    stat_bytes(node as u64, mode, entry.uid as u32, entry.gid as u32, entry.content.len() as u64)
}

/// Compose les 144 octets d'un `struct stat`.
fn stat_bytes(inode: u64, mode: u32, uid: u32, gid: u32, size: u64) -> [u8; 144] {
    let mut buffer = [0u8; 144];
    let now = crate::kernel::abi::unix_time();
    buffer[0..8].copy_from_slice(&1u64.to_le_bytes()); // st_dev
    buffer[8..16].copy_from_slice(&inode.to_le_bytes()); // st_ino
    buffer[16..24].copy_from_slice(&1u64.to_le_bytes()); // st_nlink
    buffer[24..28].copy_from_slice(&mode.to_le_bytes()); // st_mode
    buffer[28..32].copy_from_slice(&uid.to_le_bytes()); // st_uid
    buffer[32..36].copy_from_slice(&gid.to_le_bytes()); // st_gid
    buffer[48..56].copy_from_slice(&size.to_le_bytes()); // st_size
    buffer[56..64].copy_from_slice(&4096u64.to_le_bytes()); // st_blksize
    buffer[64..72].copy_from_slice(&size.div_ceil(512).to_le_bytes()); // st_blocks
    for offset in [72usize, 88, 104] {
        buffer[offset..offset + 8].copy_from_slice(&now.to_le_bytes());
    }
    buffer
}

/// `stat` / `lstat` (pas de lien symbolique dans le RAMFS : meme resultat).
pub fn sys_stat_path(path_addr: u64, out: u64, _no_follow: bool) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    if let Some(kind) = device_for_path(&path) {
        let mode = match kind {
            FdKind::Framebuffer | FdKind::InputKeyboard | FdKind::InputMouse => S_IFCHR | 0o660,
            _ => S_IFCHR | 0o666,
        };
        let buffer = stat_bytes(1, mode, 0, 0, 0);
        return if user_write(out, &buffer) { 0 } else { -errno::EFAULT };
    }
    match resolve(&path) {
        Some(node) => {
            let buffer = fill_stat(node);
            if user_write(out, &buffer) { 0 } else { -errno::EFAULT }
        }
        None => -errno::ENOENT,
    }
}

/// `fstat`.
pub fn sys_fstat(fd: i32, out: u64) -> i64 {
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return -errno::EBADF,
    };
    let buffer = match kind {
        FdKind::File(node) | FdKind::Dir(node) => fill_stat(node),
        FdKind::Console => stat_bytes(0, S_IFCHR | 0o620, 0, 0, 0),
        FdKind::Pipe(_, _) => stat_bytes(0, S_IFIFO | 0o600, 0, 0, 0),
        FdKind::Framebuffer => {
            let (width, height) = crate::drivers::gfx::resolution();
            stat_bytes(1, S_IFCHR | 0o660, 0, 0, (width * height * 4) as u64)
        }
        _ => stat_bytes(0, S_IFCHR | 0o666, 0, 0, 0),
    };
    if user_write(out, &buffer) { 0 } else { -errno::EFAULT }
}

/// `newfstatat`.
pub fn sys_newfstatat(dirfd: i32, path_addr: u64, out: u64, flags: u32) -> i64 {
    const AT_EMPTY_PATH: u32 = 0x1000;
    let path = crate::kernel::abi::resolve_user_path(path_addr).unwrap_or_default();
    if path.is_empty() || flags & AT_EMPTY_PATH != 0 {
        return sys_fstat(dirfd, out);
    }
    sys_stat_path(path_addr, out, false)
}

/// `statx` : `struct statx` de 256 octets.
pub fn sys_statx(dirfd: i32, path_addr: u64, _flags: u32, _mask: u32, out: u64) -> i64 {
    let path = crate::kernel::abi::resolve_user_path(path_addr).unwrap_or_default();
    let node = if path.is_empty() {
        let process = task::current_process();
        let found = {
            let borrowed = process.borrow();
            match borrowed.files.get(dirfd) {
                Some(desc) => match desc.kind {
                    FdKind::File(node) | FdKind::Dir(node) => Ok(Some(node)),
                    _ => Ok(None),
                },
                None => Err(()),
            }
        };
        match found {
            Ok(node) => node,
            Err(()) => return -errno::EBADF,
        }
    } else {
        resolve(&absolute(&path))
    };

    let (mode, size, uid, gid, inode) = match node {
        Some(node) => {
            let fs = ramfs::fs();
            let entry = &fs.nodes[node];
            let mode = (entry.mode as u32 & 0o7777)
                | if entry.kind == NodeKind::Dir { S_IFDIR } else { S_IFREG };
            (mode, entry.content.len() as u64, entry.uid as u32, entry.gid as u32, node as u64)
        }
        None if !path.is_empty() && device_for_path(&absolute(&path)).is_some() => {
            (S_IFCHR | 0o666, 0, 0, 0, 1)
        }
        None => return -errno::ENOENT,
    };

    let mut buffer = [0u8; 256];
    // STATX_BASIC_STATS : tous les champs de base sont renseignes.
    buffer[0..4].copy_from_slice(&0x0000_07FFu32.to_le_bytes()); // stx_mask
    buffer[4..8].copy_from_slice(&4096u32.to_le_bytes()); // stx_blksize
    buffer[16..20].copy_from_slice(&1u32.to_le_bytes()); // stx_nlink
    buffer[20..24].copy_from_slice(&uid.to_le_bytes());
    buffer[24..28].copy_from_slice(&gid.to_le_bytes());
    buffer[28..30].copy_from_slice(&(mode as u16).to_le_bytes());
    buffer[32..40].copy_from_slice(&inode.to_le_bytes());
    buffer[40..48].copy_from_slice(&size.to_le_bytes());
    buffer[48..56].copy_from_slice(&size.div_ceil(512).to_le_bytes());
    if user_write(out, &buffer) { 0 } else { -errno::EFAULT }
}

/// `access` / `faccessat`.
pub fn sys_access(path_addr: u64, _mode: u32) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    if device_for_path(&path).is_some() || resolve(&path).is_some() {
        0
    } else {
        -errno::ENOENT
    }
}

/// `readlink` : le RAMFS n'a pas de liens symboliques, sauf `/proc/self/exe`.
pub fn sys_readlink(path_addr: u64, buffer: u64, size: usize) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    if path == "/proc/self/exe" {
        let name = task::current_process().borrow().name.clone();
        let bytes = name.as_bytes();
        let len = bytes.len().min(size);
        if !user_write(buffer, &bytes[..len]) {
            return -errno::EFAULT;
        }
        return len as i64;
    }
    -errno::EINVAL
}

/// `getdents64` : `struct linux_dirent64` a la suite dans le tampon.
pub fn sys_getdents64(fd: i32, buffer: u64, size: usize) -> i64 {
    let process = task::current_process();
    let (node, start) = match process.borrow().files.get(fd) {
        Some(desc) => match desc.kind {
            FdKind::Dir(node) => (node, desc.offset),
            _ => return -errno::ENOTDIR,
        },
        None => return -errno::EBADF,
    };

    let fs = ramfs::fs();
    let mut children: Vec<(usize, String, bool)> = Vec::new();
    for index in 0..ramfs::MAX_NODES {
        if fs.nodes[index].used && fs.nodes[index].parent == node && index != node {
            let entry = &fs.nodes[index];
            children.push((index, entry.name_str().to_string(), entry.kind == NodeKind::Dir));
        }
    }

    let mut out: Vec<u8> = Vec::new();
    let mut consumed = start;
    for &(inode, ref name, is_dir) in children.iter().skip(start) {
        let record = (19 + name.len() + 1 + 7) & !7; // aligne sur 8 octets
        if out.len() + record > size {
            break;
        }
        let mut entry = alloc::vec![0u8; record];
        entry[0..8].copy_from_slice(&(inode as u64).to_le_bytes()); // d_ino
        entry[8..16].copy_from_slice(&((consumed + 1) as u64).to_le_bytes()); // d_off
        entry[16..18].copy_from_slice(&(record as u16).to_le_bytes()); // d_reclen
        entry[18] = if is_dir { 4 } else { 8 }; // DT_DIR / DT_REG
        entry[19..19 + name.len()].copy_from_slice(name.as_bytes());
        out.extend_from_slice(&entry);
        consumed += 1;
    }

    if out.is_empty() {
        return 0;
    }
    if !user_write(buffer, &out) {
        return -errno::EFAULT;
    }
    if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
        desc.offset = consumed;
    }
    out.len() as i64
}

/// `getcwd`.
pub fn sys_getcwd(buffer: u64, size: usize) -> i64 {
    let cwd = task::current_process().borrow().cwd;
    let mut path = ramfs::path_string(ramfs::fs(), cwd);
    if path.is_empty() {
        path = "/".to_string();
    }
    let mut bytes = path.into_bytes();
    bytes.push(0);
    if bytes.len() > size {
        return -errno::ERANGE;
    }
    if !user_write(buffer, &bytes) {
        return -errno::EFAULT;
    }
    bytes.len() as i64
}

/// `chdir`.
pub fn sys_chdir(path_addr: u64) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    match resolve(&path) {
        Some(node) if ramfs::fs().nodes[node].kind == NodeKind::Dir => {
            task::current_process().borrow_mut().cwd = node;
            0
        }
        Some(_) => -errno::ENOTDIR,
        None => -errno::ENOENT,
    }
}

/// `mkdir` / `mkdirat`.
pub fn sys_mkdir(path_addr: u64) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    let cwd = task::current_process().borrow().cwd;
    let fs = ramfs::fs();
    let (parent, name) = match fs.resolve_parent_name(&path, cwd) {
        Some(value) => value,
        None => return -errno::ENOENT,
    };
    if fs.find_child(parent, name).is_some() {
        return -errno::EEXIST;
    }
    match fs.mkdir_at(parent, name) {
        Ok(_) => 0,
        Err(_) => -errno::ENOSPC,
    }
}

/// `unlink` / `unlinkat`.
pub fn sys_unlink(path_addr: u64) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    match resolve(&path) {
        Some(node) if node != 0 => {
            let fs = ramfs::fs();
            if fs.nodes[node].kind == NodeKind::Dir && !fs.is_empty_dir(node) {
                return -errno::ENOTEMPTY;
            }
            fs.nodes[node].used = false;
            fs.nodes[node].content = Vec::new();
            0
        }
        Some(_) => -errno::EBUSY,
        None => -errno::ENOENT,
    }
}

/// `rename`.
pub fn sys_rename(from_addr: u64, to_addr: u64) -> i64 {
    let from = match crate::kernel::abi::resolve_user_path(from_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    let to = match crate::kernel::abi::resolve_user_path(to_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    let node = match resolve(&from) {
        Some(node) if node != 0 => node,
        Some(_) => return -errno::EBUSY,
        None => return -errno::ENOENT,
    };
    let cwd = task::current_process().borrow().cwd;
    let fs = ramfs::fs();
    let (parent, name) = match fs.resolve_parent_name(&to, cwd) {
        Some(value) => value,
        None => return -errno::ENOENT,
    };
    fs.nodes[node].parent = parent;
    if !fs.nodes[node].set_name(name) {
        return -errno::ENAMETOOLONG;
    }
    0
}

/// `ftruncate`.
pub fn sys_ftruncate(fd: i32, length: usize) -> i64 {
    let process = task::current_process();
    let node = match process.borrow().files.get(fd) {
        Some(desc) => match desc.kind {
            FdKind::File(node) => node,
            _ => return -errno::EINVAL,
        },
        None => return -errno::EBADF,
    };
    if length > ramfs::MAX_FILE_SIZE {
        return -errno::EFBIG;
    }
    ramfs::fs().nodes[node].content.resize(length, 0);
    0
}

// --- ioctl -------------------------------------------------------------------

const TCGETS: u64 = 0x5401;
const TCSETS: u64 = 0x5402;
const TIOCGWINSZ: u64 = 0x5413;
const TIOCGPGRP: u64 = 0x540F;
const FIONREAD: u64 = 0x541B;
const FIONBIO: u64 = 0x5421;

const FBIOGET_VSCREENINFO: u64 = 0x4600;
const FBIOPUT_VSCREENINFO: u64 = 0x4601;
const FBIOGET_FSCREENINFO: u64 = 0x4602;
const FBIOPAN_DISPLAY: u64 = 0x4606;
const FBIOBLANK: u64 = 0x4611;

/// `ioctl`.
pub fn sys_ioctl(fd: i32, request: u64, arg: u64) -> i64 {
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return -errno::EBADF,
    };

    match kind {
        FdKind::Console => match request {
            TCGETS => {
                // `struct termios` **version noyau** : quatre drapeaux, `c_line`,
                // puis `c_cc[19]` — soit 36 octets, pas un de plus.
                //
                // Les libc declarent une structure plus grande (60 octets chez
                // glibc comme chez musl, avec `c_cc[32]` et les vitesses) et se
                // chargent de la conversion. Ecrire cette taille-la depuis le
                // noyau deborde le tampon de pile de l'appelant : glibc le
                // detecte par son canari et tue le programme avec
                // « stack smashing detected ».
                let mut buffer = [0u8; 36];
                buffer[0..4].copy_from_slice(&0o002402u32.to_le_bytes()); // ICRNL|IXON|BRKINT
                buffer[4..8].copy_from_slice(&0o000005u32.to_le_bytes()); // OPOST|ONLCR
                buffer[8..12].copy_from_slice(&0o000277u32.to_le_bytes()); // CS8|CREAD|B38400
                buffer[12..16].copy_from_slice(&0o105073u32.to_le_bytes()); // ISIG|ICANON|ECHO
                buffer[17] = 3; // VINTR
                buffer[17 + 2] = 0x7f; // VERASE
                buffer[17 + 6] = 1; // VMIN
                if user_write(arg, &buffer) { 0 } else { -errno::EFAULT }
            }
            TCSETS | FIONBIO => 0,
            TIOCGWINSZ => {
                // `struct winsize` : lignes, colonnes, pixels.
                let mut buffer = [0u8; 8];
                buffer[0..2].copy_from_slice(&25u16.to_le_bytes());
                buffer[2..4].copy_from_slice(&80u16.to_le_bytes());
                if user_write(arg, &buffer) { 0 } else { -errno::EFAULT }
            }
            TIOCGPGRP => {
                if user_write(arg, &1u32.to_le_bytes()) { 0 } else { -errno::EFAULT }
            }
            FIONREAD => {
                if user_write(arg, &0u32.to_le_bytes()) { 0 } else { -errno::EFAULT }
            }
            _ => -errno::ENOTTY,
        },
        FdKind::Framebuffer => match request {
            FBIOGET_VSCREENINFO => {
                let buffer = fb_var_screeninfo();
                if user_write(arg, &buffer) { 0 } else { -errno::EFAULT }
            }
            FBIOGET_FSCREENINFO => {
                let buffer = fb_fix_screeninfo();
                if user_write(arg, &buffer) { 0 } else { -errno::EFAULT }
            }
            // La resolution est imposee par le materiel : on accepte sans rien
            // changer (Qt et SDL reessaient sinon indefiniment).
            FBIOPUT_VSCREENINFO | FBIOPAN_DISPLAY | FBIOBLANK => 0,
            _ => -errno::ENOTTY,
        },
        FdKind::InputKeyboard | FdKind::InputMouse => evdev_ioctl(request, arg, matches!(kind, FdKind::InputKeyboard)),
        _ => -errno::ENOTTY,
    }
}

/// `struct fb_var_screeninfo` (160 octets) decrivant le mode courant.
fn fb_var_screeninfo() -> [u8; 160] {
    let (width, height) = crate::drivers::gfx::resolution();
    let mut buffer = [0u8; 160];
    let mut put = |offset: usize, value: u32| {
        buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    put(0, width as u32); // xres
    put(4, height as u32); // yres
    put(8, width as u32); // xres_virtual
    put(12, height as u32); // yres_virtual
    put(24, 32); // bits_per_pixel
    // Canaux XRGB8888 : rouge en 16, vert en 8, bleu en 0.
    put(32, 16); put(36, 8); // red offset/length
    put(44, 8); put(48, 8); // green
    put(56, 0); put(60, 8); // blue
    put(68, 24); put(72, 0); // transp
    put(100, 1_000_000_000 / 60); // pixclock indicatif
    buffer
}

/// `struct fb_fix_screeninfo` (80 octets) : adresse et pas de ligne.
fn fb_fix_screeninfo() -> [u8; 80] {
    let (width, height) = crate::drivers::gfx::resolution();
    let mut buffer = [0u8; 80];
    let id = b"bouchaudfb";
    buffer[..id.len()].copy_from_slice(id);
    let phys = crate::drivers::gfx::lfb_phys().unwrap_or(0);
    buffer[16..24].copy_from_slice(&phys.to_le_bytes()); // smem_start
    buffer[24..28].copy_from_slice(&((width * height * 4) as u32).to_le_bytes()); // smem_len
    buffer[28..32].copy_from_slice(&0u32.to_le_bytes()); // FB_TYPE_PACKED_PIXELS
    buffer[36..40].copy_from_slice(&2u32.to_le_bytes()); // FB_VISUAL_TRUECOLOR
    buffer[48..52].copy_from_slice(&((width * 4) as u32).to_le_bytes()); // line_length
    buffer
}

/// ioctls evdev minimaux : version, identifiant, nom.
fn evdev_ioctl(request: u64, arg: u64, keyboard_device: bool) -> i64 {
    const EVIOCGVERSION: u64 = 0x8004_4501;
    // EVIOCGNAME(len) et EVIOCGBIT(ev, len) encodent la taille : on ne compare
    // donc que le type et le numero de commande.
    let number = (request >> 8) & 0xFF;
    let command = request & 0xFF;
    if request == EVIOCGVERSION {
        return if user_write(arg, &0x0001_0001u32.to_le_bytes()) { 0 } else { -errno::EFAULT };
    }
    if number == 0x45 && command == 0x06 {
        // EVIOCGNAME
        let name: &[u8] = if keyboard_device { b"bouchaud-keyboard\0" } else { b"bouchaud-mouse\0" };
        return if user_write(arg, name) { name.len() as i64 } else { -errno::EFAULT };
    }
    if number == 0x45 && command == 0x02 {
        // EVIOCGID : bustype, vendor, product, version.
        let mut buffer = [0u8; 8];
        buffer[0..2].copy_from_slice(&0x0011u16.to_le_bytes()); // BUS_I8042
        return if user_write(arg, &buffer) { 0 } else { -errno::EFAULT };
    }
    0
}

// --- Peripheriques d'entree --------------------------------------------------

/// Compose un `struct input_event` (24 octets sur x86-64).
fn input_event(kind: u16, code: u16, value: i32) -> [u8; 24] {
    let mut buffer = [0u8; 24];
    let seconds = crate::kernel::abi::unix_time();
    buffer[0..8].copy_from_slice(&seconds.to_le_bytes());
    buffer[16..18].copy_from_slice(&kind.to_le_bytes());
    buffer[18..20].copy_from_slice(&code.to_le_bytes());
    buffer[20..24].copy_from_slice(&value.to_le_bytes());
    buffer
}

/// Lecture non bloquante d'evenements clavier au format evdev.
fn read_input_keyboard(buffer: u64, count: usize) -> i64 {
    const EV_KEY: u16 = 1;
    const EV_SYN: u16 = 0;
    let mut out: Vec<u8> = Vec::new();
    while out.len() + 48 <= count {
        match keyboard::try_key() {
            Some(Key::Char(byte)) => {
                out.extend_from_slice(&input_event(EV_KEY, byte as u16, 1));
                out.extend_from_slice(&input_event(EV_KEY, byte as u16, 0));
                out.extend_from_slice(&input_event(EV_SYN, 0, 0));
            }
            Some(Key::Enter) => {
                out.extend_from_slice(&input_event(EV_KEY, 28, 1)); // KEY_ENTER
                out.extend_from_slice(&input_event(EV_KEY, 28, 0));
                out.extend_from_slice(&input_event(EV_SYN, 0, 0));
            }
            Some(_) => {}
            None => break,
        }
    }
    if out.is_empty() {
        return -errno::EAGAIN;
    }
    if user_write(buffer, &out) { out.len() as i64 } else { -errno::EFAULT }
}

/// Lecture d'evenements souris : position absolue et bouton gauche.
fn read_input_mouse(buffer: u64, count: usize) -> i64 {
    const EV_KEY: u16 = 1;
    const EV_ABS: u16 = 3;
    const EV_SYN: u16 = 0;
    const ABS_X: u16 = 0;
    const ABS_Y: u16 = 1;
    const BTN_LEFT: u16 = 0x110;

    static mut LAST: (usize, usize, bool) = (0, 0, false);
    let (x, y) = crate::drivers::mouse::pos();
    let down = crate::drivers::mouse::left_down();
    let previous = unsafe { LAST };
    if previous == (x, y, down) {
        return -errno::EAGAIN;
    }
    unsafe { LAST = (x, y, down) };

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&input_event(EV_ABS, ABS_X, x as i32));
    out.extend_from_slice(&input_event(EV_ABS, ABS_Y, y as i32));
    if previous.2 != down {
        out.extend_from_slice(&input_event(EV_KEY, BTN_LEFT, if down { 1 } else { 0 }));
    }
    out.extend_from_slice(&input_event(EV_SYN, 0, 0));
    if out.len() > count {
        out.truncate(count / 24 * 24);
    }
    if out.is_empty() {
        return -errno::EAGAIN;
    }
    if user_write(buffer, &out) { out.len() as i64 } else { -errno::EFAULT }
}

// --- Attente d'evenements ----------------------------------------------------

const POLLIN: u32 = 0x001;
const POLLOUT: u32 = 0x004;

/// Un descripteur est-il pret en lecture ?
fn readable(fd: i32) -> bool {
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return false,
    };
    match kind {
        FdKind::Console => keyboard::try_scancode().is_some(),
        FdKind::File(_) | FdKind::Dir(_) | FdKind::Zero | FdKind::Random | FdKind::Null => true,
        FdKind::Pipe(shared, true) => !shared.borrow().is_empty(),
        FdKind::InputKeyboard => keyboard::try_scancode().is_some(),
        FdKind::InputMouse => true,
        _ => false,
    }
}

/// `poll` / `ppoll` : `struct pollfd { int fd; short events; short revents; }`.
pub fn sys_poll(fds: u64, count: usize, timeout_ms: i32) -> i64 {
    let deadline = if timeout_ms < 0 {
        u64::MAX
    } else {
        crate::kernel::timer::ticks()
            + ((timeout_ms as u64) * crate::kernel::timer::TICKS_PER_SECOND).div_ceil(1000)
    };

    loop {
        let mut ready = 0i64;
        for index in 0..count {
            let base = fds + (index * 8) as u64;
            let fd = match user_read(base, 4) {
                Some(bytes) => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                None => return -errno::EFAULT,
            };
            let events = match user_read(base + 4, 2) {
                Some(bytes) => u16::from_le_bytes([bytes[0], bytes[1]]) as u32,
                None => return -errno::EFAULT,
            };
            let mut revents = 0u32;
            if fd >= 0 {
                if events & POLLIN != 0 && readable(fd) {
                    revents |= POLLIN;
                }
                if events & POLLOUT != 0 {
                    revents |= POLLOUT; // les ecritures ne bloquent jamais ici
                }
            }
            if revents != 0 {
                ready += 1;
            }
            user_write(base + 6, &(revents as u16).to_le_bytes());
        }
        if ready > 0 || crate::kernel::timer::ticks() >= deadline {
            return ready;
        }
        task::yield_now();
        crate::arch::x86_64::cpu::hlt();
    }
}

/// `select` / `pselect6`, ramene a une attente de lisibilite.
pub fn sys_select(nfds: i32, read_set: u64, _write_set: u64, _except_set: u64, timeout: u64) -> i64 {
    let deadline = if timeout == 0 {
        u64::MAX
    } else {
        let seconds = user_read_u64(timeout).unwrap_or(0);
        let micros = user_read_u64(timeout + 8).unwrap_or(0);
        let ms = seconds * 1000 + micros / 1000;
        crate::kernel::timer::ticks() + (ms * crate::kernel::timer::TICKS_PER_SECOND).div_ceil(1000)
    };

    loop {
        let mut ready = 0i64;
        if read_set != 0 {
            let words = (nfds.max(0) as usize).div_ceil(64);
            for word in 0..words {
                let bits = user_read_u64(read_set + (word * 8) as u64).unwrap_or(0);
                let mut result = 0u64;
                for bit in 0..64 {
                    let fd = (word * 64 + bit) as i32;
                    if fd >= nfds {
                        break;
                    }
                    if bits & (1 << bit) != 0 && readable(fd) {
                        result |= 1 << bit;
                        ready += 1;
                    }
                }
                user_write(read_set + (word * 8) as u64, &result.to_le_bytes());
            }
        }
        if ready > 0 || crate::kernel::timer::ticks() >= deadline {
            return ready;
        }
        task::yield_now();
        crate::arch::x86_64::cpu::hlt();
    }
}

/// `epoll_create` / `epoll_create1`.
pub fn sys_epoll_create() -> i64 {
    use alloc::rc::Rc;
    use core::cell::RefCell;
    let process = task::current_process();
    let desc = FileDesc::new(FdKind::Epoll(Rc::new(RefCell::new(Vec::new()))));
    let fd = process.borrow_mut().files.insert(desc);
    fd as i64
}

/// `epoll_ctl`.
pub fn sys_epoll_ctl(epfd: i32, operation: u32, fd: i32, event: u64) -> i64 {
    const EPOLL_CTL_ADD: u32 = 1;
    const EPOLL_CTL_DEL: u32 = 2;
    const EPOLL_CTL_MOD: u32 = 3;

    let process = task::current_process();
    let list = match process.borrow().files.get(epfd) {
        Some(desc) => match &desc.kind {
            FdKind::Epoll(list) => list.clone(),
            _ => return -errno::EINVAL,
        },
        None => return -errno::EBADF,
    };

    match operation {
        EPOLL_CTL_ADD | EPOLL_CTL_MOD => {
            // `struct epoll_event` est *packed* sur x86-64 : events (4) puis
            // data (8) sans remplissage.
            let events = match user_read(event, 4) {
                Some(bytes) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                None => return -errno::EFAULT,
            };
            let data = user_read_u64(event + 4).unwrap_or(0);
            let mut list = list.borrow_mut();
            list.retain(|entry| entry.0 != fd);
            list.push((fd, events, data));
            0
        }
        EPOLL_CTL_DEL => {
            list.borrow_mut().retain(|entry| entry.0 != fd);
            0
        }
        _ => -errno::EINVAL,
    }
}

/// `epoll_wait` / `epoll_pwait`.
pub fn sys_epoll_wait(epfd: i32, events: u64, max: usize, timeout_ms: i32) -> i64 {
    let process = task::current_process();
    let list = match process.borrow().files.get(epfd) {
        Some(desc) => match &desc.kind {
            FdKind::Epoll(list) => list.clone(),
            _ => return -errno::EINVAL,
        },
        None => return -errno::EBADF,
    };

    let deadline = if timeout_ms < 0 {
        u64::MAX
    } else {
        crate::kernel::timer::ticks()
            + ((timeout_ms as u64) * crate::kernel::timer::TICKS_PER_SECOND).div_ceil(1000)
    };

    loop {
        let mut written = 0usize;
        for &(fd, wanted, data) in list.borrow().iter() {
            if written >= max {
                break;
            }
            if wanted & POLLIN != 0 && readable(fd) {
                let base = events + (written * 12) as u64;
                user_write(base, &POLLIN.to_le_bytes());
                user_write(base + 4, &data.to_le_bytes());
                written += 1;
            }
        }
        if written > 0 || crate::kernel::timer::ticks() >= deadline {
            return written as i64;
        }
        task::yield_now();
        crate::arch::x86_64::cpu::hlt();
    }
}

/// `eventfd` : implemente comme un tube (suffisant pour un reveil de boucle).
pub fn sys_eventfd() -> i64 {
    use alloc::rc::Rc;
    use core::cell::RefCell;
    let shared = Rc::new(RefCell::new(Vec::new()));
    let process = task::current_process();
    let desc = FileDesc::new(FdKind::Pipe(shared, true));
    let fd = process.borrow_mut().files.insert(desc);
    fd as i64
}
