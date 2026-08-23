//! Mini-runtime WebAssembly de Bouchaud OS (no_std), bati sur l'interpreteur
//! `wasmi`. Permet d'executer des modules `.wasm` compiles depuis n'importe
//! quel langage (Rust, C, Zig, TinyGo, AssemblyScript...).
//!
//! Trois surfaces d'utilisation :
//!   - **OS natif** : `run_bytes(&wasm)` / `run_program(...)` instancient le
//!     module, cablent un petit ABI hote (`env.print*`) + WASI preview1,
//!     appellent le point d'entree (`_start` / `main` / `run`) et renvoient la
//!     sortie. Utilise par les commandes shell `wasm` et `python`.
//!   - **Web (API JS `WebAssembly`)** : `Module`/`Instance` persistants exposes
//!     a l'interpreteur JS (voir `gui::js`).
//!
//! **WASI preview1 complet cote fichiers** : les appels (`path_open`,
//! `fd_read`, `fd_readdir`, `path_create_directory`...) sont traduits vers le
//! RAMFS avec les permissions Unix de la session courante. Le repertoire
//! preouvert fd=3 est `/` : un programme WASI voit le meme systeme de fichiers
//! que le shell. C'est ce qui permet a l'interpreteur Python embarque d'ouvrir
//! des scripts, d'importer des modules depuis `/usr/lib/python/site-packages`
//! (paquets installes par `pip`) et d'ecrire des fichiers.
//!
//! **Mode interactif** : `run_program(..., interactive=true)` cable stdout/
//! stderr directement sur la console (affichage au fil de l'eau) et stdin sur
//! le clavier (lecture bloquante ligne a ligne) -> REPL Python, `input()`...
//! En mode graphique HD le clavier appartient au window manager : stdin rend
//! alors EOF au lieu de bloquer.
//!
//! Securite : l'execution est bornee par du "fuel" (metrage d'instructions)
//! pour qu'un module en boucle infinie ne gele pas le noyau ; le mode
//! interactif recharge le budget a chaque lecture clavier.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use wasmi::core::{Trap, ValueType};
use wasmi::{Caller, Config, Engine, Extern, ExternType, Linker, Memory, Module as WModule, Store, Value as WVal};

use crate::fs::ramfs::{self, NodeKind, PERM_R, PERM_W};

/// Budget d'instructions par invocation (anti-boucle infinie) des modules
/// `.wasm` generiques (commande `wasm`, appels depuis JS).
const FUEL: u64 = 200_000_000;

/// Budget pour l'interpreteur Python : un vrai interprete consomme des
/// milliards d'instructions (bootstrap importlib, stdlib figee...). Le mode
/// interactif recharge en plus ce budget a chaque lecture clavier.
pub const PYTHON_FUEL: u64 = 50_000_000_000;

// ---------------------------------------------------------------------------
// Codes d'erreur WASI preview1 (differents des errno POSIX !)
// ---------------------------------------------------------------------------
mod errno {
    pub const SUCCESS: i32 = 0;
    pub const ACCES: i32 = 2;
    pub const BADF: i32 = 8;
    pub const EXIST: i32 = 20;
    pub const INVAL: i32 = 28;
    pub const ISDIR: i32 = 31;
    pub const NAMETOOLONG: i32 = 37;
    pub const NOENT: i32 = 44;
    pub const NOSPC: i32 = 51;
    pub const NOSYS: i32 = 52;
    pub const NOTDIR: i32 = 54;
    pub const NOTEMPTY: i32 = 55;
}

/// Types de fichiers WASI (`filetype`).
mod ftype {
    pub const CHARACTER_DEVICE: u8 = 2;
    pub const DIRECTORY: u8 = 3;
    pub const REGULAR_FILE: u8 = 4;
}

/// Descripteur ouvert par le module WASM (fd >= 3).
struct FdEntry {
    /// Inode RAMFS.
    node: usize,
    /// Position de lecture/ecriture.
    pos: usize,
    dir: bool,
    append: bool,
}

/// Etat hote partage avec le module pendant son execution.
pub struct HostState {
    /// Sortie texte accumulee (mode non interactif).
    pub out: String,
    /// Code de sortie demande via `proc_exit`, le cas echeant.
    pub exit_code: Option<i32>,
    /// Entree standard bufferisee (fd 0), consommee par `fd_read`.
    stdin: Vec<u8>,
    stdin_pos: usize,
    /// Arguments (`argv[0]` = nom du programme).
    args: Vec<String>,
    /// Variables d'environnement (PYTHONPATH, PWD...).
    env: Vec<(String, String)>,
    /// stdout/err vers la console au fil de l'eau + stdin clavier bloquant.
    interactive: bool,
    /// Table des descripteurs : 0..2 stdio, 3 = preouverture de `/`,
    /// 4 = preouverture de `.` (repertoire courant du shell) -- c'est via `.`
    /// que la libc WASI resout les chemins relatifs.
    fds: Vec<Option<FdEntry>>,
}

impl HostState {
    fn new() -> HostState {
        Self::with_cwd(0)
    }

    fn with_cwd(cwd_node: usize) -> HostState {
        HostState {
            out: String::new(),
            exit_code: None,
            stdin: Vec::new(),
            stdin_pos: 0,
            args: Vec::new(),
            env: Vec::new(),
            interactive: false,
            fds: vec![
                None,
                None,
                None,
                // fd 3 : racine `/` (inode 0), fd 4 : `.` (cwd du shell).
                Some(FdEntry { node: 0, pos: 0, dir: true, append: false }),
                Some(FdEntry { node: cwd_node, pos: 0, dir: true, append: false }),
            ],
        }
    }
}

/// Resultat d'une execution complete.
pub struct RunResult {
    pub output: String,
    pub result: Option<i64>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

// ----------------------------------------------------------------------------
// Helpers memoire hote
// ----------------------------------------------------------------------------

fn caller_mem(caller: &mut Caller<'_, HostState>) -> Option<Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(m)) => Some(m),
        _ => None,
    }
}

fn read_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    let mem = caller_mem(caller)?;
    let data = mem.data(&*caller);
    let start = ptr as usize;
    let end = start.checked_add(len.max(0) as usize)?;
    if end > data.len() {
        return None;
    }
    Some(data[start..end].to_vec())
}

fn read_str(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<String> {
    let b = read_bytes(caller, ptr, len)?;
    Some(String::from_utf8_lossy(&b).to_string())
}

fn write_mem(caller: &mut Caller<'_, HostState>, ptr: i32, bytes: &[u8]) {
    if let Some(mem) = caller_mem(caller) {
        let _ = mem.write(&mut *caller, ptr as usize, bytes);
    }
}

fn write_u32(caller: &mut Caller<'_, HostState>, ptr: i32, v: u32) {
    write_mem(caller, ptr, &v.to_le_bytes());
}

fn write_u64(caller: &mut Caller<'_, HostState>, ptr: i32, v: u64) {
    write_mem(caller, ptr, &v.to_le_bytes());
}

/// Sortie du module : console directe en interactif, tampon sinon.
fn push_out(caller: &mut Caller<'_, HostState>, bytes: &[u8]) {
    if caller.data().interactive {
        for chunk in core::str::from_utf8(bytes).ok().map(|s| s.to_string()).into_iter() {
            crate::print!("{}", chunk);
        }
        if core::str::from_utf8(bytes).is_err() {
            for &b in bytes {
                crate::print!("{}", b as char);
            }
        }
        return;
    }
    match core::str::from_utf8(bytes) {
        Ok(s) => caller.data_mut().out.push_str(s),
        Err(_) => {
            for &b in bytes {
                caller.data_mut().out.push(b as char);
            }
        }
    }
}

// PRNG deterministe pour WASI random_get (suffisant pour un OS experimental).
fn prng_byte() -> u8 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static S: AtomicU64 = AtomicU64::new(0x9E3779B97F4A7C15);
    let mut x = S.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    S.store(x, Ordering::Relaxed);
    (x >> 24) as u8
}

/// Horloge en nanosecondes basee sur le PIT (resolution ~1/18 s).
fn clock_ns() -> u64 {
    crate::kernel::timer::ticks().wrapping_mul(1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND)
}

// ----------------------------------------------------------------------------
// Traduction WASI <-> RAMFS
// ----------------------------------------------------------------------------

fn map_fs_err(e: &str) -> i32 {
    match e {
        "permission denied" => errno::ACCES,
        "introuvable" => errno::NOENT,
        _ => errno::NOENT,
    }
}

/// Resout `path` relativement au noeud du descripteur-repertoire `dirfd`.
fn resolve_at(st: &HostState, dirfd: i32, path: &str) -> Result<usize, i32> {
    let base = match st.fds.get(dirfd as usize).and_then(|e| e.as_ref()) {
        Some(e) if e.dir => e.node,
        Some(_) => return Err(errno::NOTDIR),
        None => return Err(errno::BADF),
    };
    ramfs::fs().resolve_checked(path, base).map_err(map_fs_err)
}

/// Resout le parent de `path` (relatif a `dirfd`) et le nom de la feuille.
fn resolve_parent_at<'a>(st: &HostState, dirfd: i32, path: &'a str) -> Result<(usize, &'a str), i32> {
    let base = match st.fds.get(dirfd as usize).and_then(|e| e.as_ref()) {
        Some(e) if e.dir => e.node,
        Some(_) => return Err(errno::NOTDIR),
        None => return Err(errno::BADF),
    };
    ramfs::fs().resolve_parent_name_checked(path, base).map_err(|e| match e {
        "permission denied" => errno::ACCES,
        "chemin invalide" => errno::INVAL,
        _ => errno::NOENT,
    })
}

/// Encodage de la structure `filestat` (64 octets) d'un inode RAMFS.
fn filestat_of_node(node: usize) -> [u8; 64] {
    let fs = ramfs::fs();
    let n = &fs.nodes[node];
    let (ft, size) = match n.kind {
        NodeKind::Dir => (ftype::DIRECTORY, 0u64),
        NodeKind::File => (ftype::REGULAR_FILE, n.content.len() as u64),
    };
    filestat_raw(node as u64, ft, size)
}

fn filestat_raw(ino: u64, ft: u8, size: u64) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0..8].copy_from_slice(&1u64.to_le_bytes()); // dev
    b[8..16].copy_from_slice(&ino.to_le_bytes());
    b[16] = ft;
    b[24..32].copy_from_slice(&1u64.to_le_bytes()); // nlink
    b[32..40].copy_from_slice(&size.to_le_bytes());
    // atim/mtim/ctim : pas d'horodatage des fichiers dans le RAMFS (a 0).
    b
}

/// Alloue un fd libre (>= 5, apres les stdio et les deux preouvertures).
fn alloc_fd(st: &mut HostState, entry: FdEntry) -> i32 {
    for (i, slot) in st.fds.iter_mut().enumerate().skip(5) {
        if slot.is_none() {
            *slot = Some(entry);
            return i as i32;
        }
    }
    st.fds.push(Some(entry));
    (st.fds.len() - 1) as i32
}

/// Lecture clavier bloquante d'une ligne, injectee dans le tampon stdin.
/// Recharge aussi le fuel : chaque interaction utilisateur redonne du budget.
fn refill_stdin_interactive(caller: &mut Caller<'_, HostState>) {
    if crate::drivers::gfx::is_active() {
        return; // mode graphique : pas de lecture clavier directe -> EOF
    }
    let mut buf = [0u8; 256];
    let n = crate::drivers::keyboard::read_line(&mut buf);
    let st = caller.data_mut();
    st.stdin.truncate(0);
    st.stdin_pos = 0;
    st.stdin.extend_from_slice(&buf[..n]);
    st.stdin.push(b'\n');
    let _ = caller.add_fuel(PYTHON_FUEL);
}

// ----------------------------------------------------------------------------
// WASI preview1
// ----------------------------------------------------------------------------

/// Copie `data` vers les iovecs et ecrit le total dans `ndone`. Renvoie le
/// nombre d'octets copies.
fn scatter_to_iovs(caller: &mut Caller<'_, HostState>, iovs: i32, iovs_len: i32, data: &[u8], ndone: i32) -> usize {
    let mem = match caller_mem(caller) {
        Some(m) => m,
        None => return 0,
    };
    let mut copied = 0usize;
    let base = iovs.max(0) as usize;
    for k in 0..(iovs_len.max(0) as usize) {
        let (p, l) = {
            let d = mem.data(&*caller);
            let rec = base + k * 8;
            if rec + 8 > d.len() {
                break;
            }
            let p = u32::from_le_bytes([d[rec], d[rec + 1], d[rec + 2], d[rec + 3]]) as usize;
            let l = u32::from_le_bytes([d[rec + 4], d[rec + 5], d[rec + 6], d[rec + 7]]) as usize;
            (p, l)
        };
        if copied >= data.len() {
            break;
        }
        let n = l.min(data.len() - copied);
        write_mem(caller, p as i32, &data[copied..copied + n]);
        copied += n;
        if n < l {
            break;
        }
    }
    write_u32(caller, ndone, copied as u32);
    copied
}

/// Rassemble les octets designes par les iovecs.
fn gather_from_iovs(caller: &mut Caller<'_, HostState>, iovs: i32, iovs_len: i32) -> Vec<u8> {
    let mem = match caller_mem(caller) {
        Some(m) => m,
        None => return Vec::new(),
    };
    let mut collected: Vec<u8> = Vec::new();
    let d = mem.data(&*caller);
    let base = iovs.max(0) as usize;
    for k in 0..(iovs_len.max(0) as usize) {
        let rec = base + k * 8;
        if rec + 8 > d.len() {
            break;
        }
        let p = u32::from_le_bytes([d[rec], d[rec + 1], d[rec + 2], d[rec + 3]]) as usize;
        let l = u32::from_le_bytes([d[rec + 4], d[rec + 5], d[rec + 6], d[rec + 7]]) as usize;
        if p + l <= d.len() {
            collected.extend_from_slice(&d[p..p + l]);
        }
    }
    collected
}

fn wasi_fd_read(caller: &mut Caller<'_, HostState>, fd: i32, iovs: i32, iovs_len: i32, nread: i32) -> i32 {
    if fd == 0 {
        // stdin : tampon fourni, puis clavier en interactif, puis EOF.
        let exhausted = {
            let st = caller.data();
            st.stdin_pos >= st.stdin.len()
        };
        if exhausted && caller.data().interactive {
            refill_stdin_interactive(caller);
        }
        let avail: Vec<u8> = {
            let st = caller.data();
            st.stdin[st.stdin_pos.min(st.stdin.len())..].to_vec()
        };
        let n = scatter_to_iovs(caller, iovs, iovs_len, &avail, nread);
        caller.data_mut().stdin_pos += n;
        return errno::SUCCESS;
    }
    // Fichier ouvert.
    let (node, pos) = match caller.data().fds.get(fd as usize).and_then(|e| e.as_ref()) {
        Some(e) if !e.dir => (e.node, e.pos),
        Some(_) => return errno::ISDIR,
        None => return errno::BADF,
    };
    let data: Vec<u8> = {
        let fs = ramfs::fs();
        let c = &fs.nodes[node].content;
        c[pos.min(c.len())..].to_vec()
    };
    let n = scatter_to_iovs(caller, iovs, iovs_len, &data, nread);
    if let Some(Some(e)) = caller.data_mut().fds.get_mut(fd as usize) {
        e.pos += n;
    }
    errno::SUCCESS
}

fn wasi_fd_write(caller: &mut Caller<'_, HostState>, fd: i32, iovs: i32, iovs_len: i32, nwritten: i32) -> i32 {
    let bytes = gather_from_iovs(caller, iovs, iovs_len);
    if fd == 1 || fd == 2 {
        push_out(caller, &bytes);
        write_u32(caller, nwritten, bytes.len() as u32);
        return errno::SUCCESS;
    }
    let (node, mut pos, append) = match caller.data().fds.get(fd as usize).and_then(|e| e.as_ref()) {
        Some(e) if !e.dir => (e.node, e.pos, e.append),
        Some(_) => return errno::ISDIR,
        None => return errno::BADF,
    };
    // `/dev/web` : ecrire une URL declenche une requete du noyau au lieu
    // d'atterrir dans le RAMFS (voir `lang::pyweb`).
    if crate::lang::pyweb::device_write(node, &bytes) {
        write_u32(caller, nwritten, bytes.len() as u32);
        return errno::SUCCESS;
    }
    {
        let fs = ramfs::fs();
        if !fs.can(node, PERM_W) {
            return errno::ACCES;
        }
        let content = &mut fs.nodes[node].content;
        if append {
            pos = content.len();
        }
        let end = pos + bytes.len();
        if end > ramfs::MAX_FILE_SIZE {
            return errno::NOSPC;
        }
        if content.len() < end {
            content.resize(end, 0);
        }
        content[pos..end].copy_from_slice(&bytes);
    }
    if let Some(Some(e)) = caller.data_mut().fds.get_mut(fd as usize) {
        e.pos = pos + bytes.len();
    }
    write_u32(caller, nwritten, bytes.len() as u32);
    errno::SUCCESS
}

fn wasi_path_open(
    caller: &mut Caller<'_, HostState>,
    dirfd: i32,
    path: &str,
    oflags: i32,
    rights_base: i64,
    fdflags: i32,
) -> Result<i32, i32> {
    const O_CREAT: i32 = 1;
    const O_DIRECTORY: i32 = 2;
    const O_EXCL: i32 = 4;
    const O_TRUNC: i32 = 8;
    const RIGHT_FD_WRITE: i64 = 1 << 6;
    let want_write = rights_base & RIGHT_FD_WRITE != 0 || oflags & (O_CREAT | O_TRUNC) != 0;

    let existing = resolve_at(caller.data(), dirfd, path);
    let node = match existing {
        Ok(n) => {
            if oflags & O_CREAT != 0 && oflags & O_EXCL != 0 {
                return Err(errno::EXIST);
            }
            n
        }
        Err(errno::NOENT) if oflags & O_CREAT != 0 => {
            let (parent, name) = resolve_parent_at(caller.data(), dirfd, path)?;
            let fs = ramfs::fs();
            if fs.nodes[parent].kind != NodeKind::Dir {
                return Err(errno::NOTDIR);
            }
            if !fs.can(parent, PERM_W) {
                return Err(errno::ACCES);
            }
            fs.touch_at(parent, name).map_err(|e| match e {
                "invalid name" => errno::NAMETOOLONG,
                "no free inode" => errno::NOSPC,
                _ => errno::NOENT,
            })?
        }
        Err(e) => return Err(e),
    };

    let fs = ramfs::fs();
    let is_dir = fs.nodes[node].kind == NodeKind::Dir;
    if oflags & O_DIRECTORY != 0 && !is_dir {
        return Err(errno::NOTDIR);
    }
    if is_dir && want_write {
        return Err(errno::ISDIR);
    }
    if !is_dir && !fs.can(node, PERM_R) && !want_write {
        return Err(errno::ACCES);
    }
    if want_write {
        if !fs.can(node, PERM_W) {
            return Err(errno::ACCES);
        }
        if oflags & O_TRUNC != 0 {
            fs.nodes[node].content.clear();
        }
    }
    const FDFLAG_APPEND: i32 = 1;
    Ok(alloc_fd(
        caller.data_mut(),
        FdEntry { node, pos: 0, dir: is_dir, append: fdflags & FDFLAG_APPEND != 0 },
    ))
}

/// Entrees d'un repertoire pour `fd_readdir` : (nom, inode, type).
fn dir_entries(node: usize) -> Vec<(String, u64, u8)> {
    let fs = ramfs::fs();
    let mut out: Vec<(String, u64, u8)> = Vec::new();
    out.push((".".to_string(), node as u64, ftype::DIRECTORY));
    out.push(("..".to_string(), fs.nodes[node].parent as u64, ftype::DIRECTORY));
    for i in 0..ramfs::MAX_NODES {
        if fs.nodes[i].used && i != node && fs.nodes[i].parent == node {
            let ft = match fs.nodes[i].kind {
                NodeKind::Dir => ftype::DIRECTORY,
                NodeKind::File => ftype::REGULAR_FILE,
            };
            out.push((fs.nodes[i].name_str().to_string(), i as u64, ft));
        }
    }
    out
}

fn build_linker(engine: &Engine) -> Linker<HostState> {
    let mut l = Linker::new(engine);

    // --- ABI "env" minimal (modules custom / freestanding) ---
    l.func_wrap("env", "print", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
        if let Some(b) = read_bytes(&mut caller, ptr, len) {
            push_out(&mut caller, &b);
        }
    })
    .ok();
    l.func_wrap("env", "print_int", |mut caller: Caller<'_, HostState>, v: i32| {
        let s = format!("{}", v);
        let b = s.into_bytes();
        push_out(&mut caller, &b);
    })
    .ok();
    l.func_wrap("env", "putchar", |mut caller: Caller<'_, HostState>, c: i32| {
        if let Some(ch) = core::char::from_u32(c as u32) {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            let b = s.as_bytes().to_vec();
            push_out(&mut caller, &b);
        }
    })
    .ok();

    // --- WASI preview1 : processus ---
    l.func_wrap(
        "wasi_snapshot_preview1",
        "proc_exit",
        |mut caller: Caller<'_, HostState>, code: i32| -> Result<(), Trap> {
            // Arret immediat (trap volontaire) : ne pas laisser le module
            // deroular jusqu'a l'`unreachable` place apres l'appel par la libc.
            caller.data_mut().exit_code = Some(code);
            Err(Trap::new("proc_exit"))
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "args_sizes_get",
        |mut caller: Caller<'_, HostState>, count_ptr: i32, size_ptr: i32| -> i32 {
            let args = &caller.data().args;
            let count = args.len() as u32;
            let size: u32 = args.iter().map(|a| a.len() as u32 + 1).sum();
            write_u32(&mut caller, count_ptr, count);
            write_u32(&mut caller, size_ptr, size);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "args_get",
        |mut caller: Caller<'_, HostState>, argv: i32, argv_buf: i32| -> i32 {
            let args = caller.data().args.clone();
            let mut off = argv_buf;
            for (i, a) in args.iter().enumerate() {
                write_u32(&mut caller, argv + (i as i32) * 4, off as u32);
                let mut b = a.as_bytes().to_vec();
                b.push(0);
                write_mem(&mut caller, off, &b);
                off += b.len() as i32;
            }
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "environ_sizes_get",
        |mut caller: Caller<'_, HostState>, count_ptr: i32, size_ptr: i32| -> i32 {
            let env = &caller.data().env;
            let count = env.len() as u32;
            let size: u32 = env.iter().map(|(k, v)| (k.len() + v.len() + 2) as u32).sum();
            write_u32(&mut caller, count_ptr, count);
            write_u32(&mut caller, size_ptr, size);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "environ_get",
        |mut caller: Caller<'_, HostState>, envp: i32, env_buf: i32| -> i32 {
            let env = caller.data().env.clone();
            let mut off = env_buf;
            for (i, (k, v)) in env.iter().enumerate() {
                write_u32(&mut caller, envp + (i as i32) * 4, off as u32);
                let mut b = Vec::with_capacity(k.len() + v.len() + 2);
                b.extend_from_slice(k.as_bytes());
                b.push(b'=');
                b.extend_from_slice(v.as_bytes());
                b.push(0);
                write_mem(&mut caller, off, &b);
                off += b.len() as i32;
            }
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "random_get",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
            let n = len.max(0) as usize;
            let buf: Vec<u8> = (0..n).map(|_| prng_byte()).collect();
            write_mem(&mut caller, ptr, &buf);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "clock_time_get",
        |mut caller: Caller<'_, HostState>, _id: i32, _prec: i64, time_ptr: i32| -> i32 {
            write_u64(&mut caller, time_ptr, clock_ns());
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap("wasi_snapshot_preview1", "sched_yield", |_c: Caller<'_, HostState>| -> i32 { errno::SUCCESS })
        .ok();
    // `poll_oneoff` : gere les abonnements horloge (time.sleep !) en attendant
    // reellement sur le PIT ; les autres abonnements sont declares prets.
    l.func_wrap(
        "wasi_snapshot_preview1",
        "poll_oneoff",
        |mut caller: Caller<'_, HostState>, in_: i32, out: i32, nsubs: i32, nevents: i32| -> i32 {
            let n = nsubs.max(0) as usize;
            let subs = match read_bytes(&mut caller, in_, (n * 48) as i32) {
                Some(b) => b,
                None => return errno::INVAL,
            };
            let mut events: Vec<u8> = Vec::with_capacity(n * 32);
            for k in 0..n {
                let s = &subs[k * 48..(k + 1) * 48];
                let userdata = u64::from_le_bytes(s[0..8].try_into().unwrap());
                let tag = s[8];
                if tag == 0 {
                    // Horloge : timeout ns (relatif sauf flag ABSTIME bit 0).
                    let timeout = u64::from_le_bytes(s[24..32].try_into().unwrap());
                    let flags = u16::from_le_bytes(s[40..42].try_into().unwrap());
                    let now = clock_ns();
                    let deadline = if flags & 1 != 0 { timeout } else { now.saturating_add(timeout) };
                    while clock_ns() < deadline {
                        crate::kernel::task::wait_for_interrupt_releasing_bkl();
                    }
                }
                let mut ev = [0u8; 32];
                ev[0..8].copy_from_slice(&userdata.to_le_bytes());
                // error=0 (u16), type = tag (u8)
                ev[10] = tag;
                events.extend_from_slice(&ev);
            }
            write_mem(&mut caller, out, &events);
            write_u32(&mut caller, nevents, n as u32);
            errno::SUCCESS
        },
    )
    .ok();

    // --- WASI preview1 : descripteurs ---
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_read",
        |mut caller: Caller<'_, HostState>, fd: i32, iovs: i32, iovs_len: i32, nread: i32| -> i32 {
            wasi_fd_read(&mut caller, fd, iovs, iovs_len, nread)
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_write",
        |mut caller: Caller<'_, HostState>, fd: i32, iovs: i32, iovs_len: i32, nwritten: i32| -> i32 {
            wasi_fd_write(&mut caller, fd, iovs, iovs_len, nwritten)
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_close",
        |mut caller: Caller<'_, HostState>, fd: i32| -> i32 {
            if fd <= 2 {
                return errno::SUCCESS;
            }
            match caller.data_mut().fds.get_mut(fd as usize) {
                Some(slot @ Some(_)) => {
                    *slot = None;
                    errno::SUCCESS
                }
                _ => errno::BADF,
            }
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_seek",
        |mut caller: Caller<'_, HostState>, fd: i32, offset: i64, whence: i32, newoffset: i32| -> i32 {
            let (node, pos) = match caller.data().fds.get(fd as usize).and_then(|e| e.as_ref()) {
                Some(e) if !e.dir => (e.node, e.pos),
                Some(_) => return errno::ISDIR,
                None => return errno::BADF,
            };
            let size = ramfs::fs().nodes[node].content.len() as i64;
            let new = match whence {
                0 => offset,              // SET
                1 => pos as i64 + offset, // CUR
                2 => size + offset,       // END
                _ => return errno::INVAL,
            };
            if new < 0 {
                return errno::INVAL;
            }
            if let Some(Some(e)) = caller.data_mut().fds.get_mut(fd as usize) {
                e.pos = new as usize;
            }
            write_u64(&mut caller, newoffset, new as u64);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap("wasi_snapshot_preview1", "fd_sync", |_c: Caller<'_, HostState>, _fd: i32| -> i32 { errno::SUCCESS })
        .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_fdstat_get",
        |mut caller: Caller<'_, HostState>, fd: i32, buf: i32| -> i32 {
            let mut b = [0u8; 24];
            let (ft, append) = if fd <= 2 {
                (ftype::CHARACTER_DEVICE, false)
            } else {
                match caller.data().fds.get(fd as usize).and_then(|e| e.as_ref()) {
                    Some(e) => (if e.dir { ftype::DIRECTORY } else { ftype::REGULAR_FILE }, e.append),
                    None => return errno::BADF,
                }
            };
            b[0] = ft;
            if append {
                b[2] = 1; // fdflags: APPEND
            }
            // Tous les droits accordes (le controle reel = permissions RAMFS),
            // SAUF seek/tell sur les stdio : c'est ainsi que la libc WASI
            // detecte un terminal (`isatty`) -> sortie Python line-buffered.
            const RIGHT_FD_SEEK: u64 = 1 << 2;
            const RIGHT_FD_TELL: u64 = 1 << 5;
            let rights = if fd <= 2 { u64::MAX & !(RIGHT_FD_SEEK | RIGHT_FD_TELL) } else { u64::MAX };
            b[8..16].copy_from_slice(&rights.to_le_bytes());
            b[16..24].copy_from_slice(&rights.to_le_bytes());
            write_mem(&mut caller, buf, &b);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_fdstat_set_flags",
        |mut caller: Caller<'_, HostState>, fd: i32, flags: i32| -> i32 {
            match caller.data_mut().fds.get_mut(fd as usize) {
                Some(Some(e)) => {
                    e.append = flags & 1 != 0;
                    errno::SUCCESS
                }
                _ => {
                    if fd <= 2 {
                        errno::SUCCESS
                    } else {
                        errno::BADF
                    }
                }
            }
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_filestat_get",
        |mut caller: Caller<'_, HostState>, fd: i32, buf: i32| -> i32 {
            let b = if fd <= 2 {
                filestat_raw(fd as u64, ftype::CHARACTER_DEVICE, 0)
            } else {
                match caller.data().fds.get(fd as usize).and_then(|e| e.as_ref()) {
                    Some(e) => filestat_of_node(e.node),
                    None => return errno::BADF,
                }
            };
            write_mem(&mut caller, buf, &b);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_filestat_set_size",
        |caller: Caller<'_, HostState>, fd: i32, size: i64| -> i32 {
            let node = match caller.data().fds.get(fd as usize).and_then(|e| e.as_ref()) {
                Some(e) if !e.dir => e.node,
                Some(_) => return errno::ISDIR,
                None => return errno::BADF,
            };
            let size = size.max(0) as usize;
            if size > ramfs::MAX_FILE_SIZE {
                return errno::NOSPC;
            }
            let fs = ramfs::fs();
            if !fs.can(node, PERM_W) {
                return errno::ACCES;
            }
            fs.nodes[node].content.resize(size, 0);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_prestat_get",
        |mut caller: Caller<'_, HostState>, fd: i32, buf: i32| -> i32 {
            let name: &[u8] = match fd {
                3 => b"/",
                4 => b".",
                _ => return errno::BADF,
            };
            let mut b = [0u8; 8];
            b[0] = 0; // preopen de type repertoire
            b[4..8].copy_from_slice(&(name.len() as u32).to_le_bytes());
            write_mem(&mut caller, buf, &b);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_prestat_dir_name",
        |mut caller: Caller<'_, HostState>, fd: i32, path: i32, len: i32| -> i32 {
            let name: &[u8] = match fd {
                3 => b"/",
                4 => b".",
                _ => return errno::BADF,
            };
            if (len as usize) < name.len() {
                return errno::INVAL;
            }
            write_mem(&mut caller, path, name);
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_readdir",
        |mut caller: Caller<'_, HostState>, fd: i32, buf: i32, buf_len: i32, cookie: i64, bufused: i32| -> i32 {
            let node = match caller.data().fds.get(fd as usize).and_then(|e| e.as_ref()) {
                Some(e) if e.dir => e.node,
                Some(_) => return errno::NOTDIR,
                None => return errno::BADF,
            };
            let entries = dir_entries(node);
            let cap = buf_len.max(0) as usize;
            let mut out: Vec<u8> = Vec::with_capacity(cap.min(4096));
            for (i, (name, ino, ft)) in entries.iter().enumerate().skip(cookie.max(0) as usize) {
                let mut dirent = [0u8; 24];
                dirent[0..8].copy_from_slice(&((i + 1) as u64).to_le_bytes()); // d_next
                dirent[8..16].copy_from_slice(&ino.to_le_bytes());
                dirent[16..20].copy_from_slice(&(name.len() as u32).to_le_bytes());
                dirent[20] = *ft;
                out.extend_from_slice(&dirent);
                out.extend_from_slice(name.as_bytes());
                if out.len() >= cap {
                    out.truncate(cap); // tampon plein : la libc rappellera
                    break;
                }
            }
            write_mem(&mut caller, buf, &out);
            write_u32(&mut caller, bufused, out.len() as u32);
            errno::SUCCESS
        },
    )
    .ok();

    // --- WASI preview1 : chemins ---
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_open",
        |mut caller: Caller<'_, HostState>,
         dirfd: i32,
         _dirflags: i32,
         path: i32,
         path_len: i32,
         oflags: i32,
         rights_base: i64,
         _rights_inheriting: i64,
         fdflags: i32,
         opened_fd: i32|
         -> i32 {
            let p = match read_str(&mut caller, path, path_len) {
                Some(s) => s,
                None => return errno::INVAL,
            };
            match wasi_path_open(&mut caller, dirfd, &p, oflags, rights_base, fdflags) {
                Ok(fd) => {
                    write_u32(&mut caller, opened_fd, fd as u32);
                    errno::SUCCESS
                }
                Err(e) => e,
            }
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_filestat_get",
        |mut caller: Caller<'_, HostState>, dirfd: i32, _flags: i32, path: i32, path_len: i32, buf: i32| -> i32 {
            let p = match read_str(&mut caller, path, path_len) {
                Some(s) => s,
                None => return errno::INVAL,
            };
            match resolve_at(caller.data(), dirfd, &p) {
                Ok(node) => {
                    let b = filestat_of_node(node);
                    write_mem(&mut caller, buf, &b);
                    errno::SUCCESS
                }
                Err(e) => e,
            }
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_filestat_set_times",
        |_c: Caller<'_, HostState>, _fd: i32, _fl: i32, _p: i32, _pl: i32, _atim: i64, _mtim: i64, _fst: i32| -> i32 {
            errno::SUCCESS // pas d'horodatage dans le RAMFS : no-op
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_create_directory",
        |mut caller: Caller<'_, HostState>, dirfd: i32, path: i32, path_len: i32| -> i32 {
            let p = match read_str(&mut caller, path, path_len) {
                Some(s) => s,
                None => return errno::INVAL,
            };
            let (parent, name) = match resolve_parent_at(caller.data(), dirfd, &p) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let fs = ramfs::fs();
            if !fs.can(parent, PERM_W) {
                return errno::ACCES;
            }
            match fs.mkdir_at(parent, name) {
                Ok(_) => errno::SUCCESS,
                Err("already exists") => errno::EXIST,
                Err("no free inode") => errno::NOSPC,
                Err("invalid name") => errno::NAMETOOLONG,
                Err(_) => errno::NOENT,
            }
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_unlink_file",
        |mut caller: Caller<'_, HostState>, dirfd: i32, path: i32, path_len: i32| -> i32 {
            let p = match read_str(&mut caller, path, path_len) {
                Some(s) => s,
                None => return errno::INVAL,
            };
            let node = match resolve_at(caller.data(), dirfd, &p) {
                Ok(n) => n,
                Err(e) => return e,
            };
            let fs = ramfs::fs();
            if fs.nodes[node].kind == NodeKind::Dir {
                return errno::ISDIR;
            }
            if !fs.can(fs.nodes[node].parent, PERM_W) {
                return errno::ACCES;
            }
            fs.nodes[node].used = false;
            fs.nodes[node].content = Vec::new();
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_remove_directory",
        |mut caller: Caller<'_, HostState>, dirfd: i32, path: i32, path_len: i32| -> i32 {
            let p = match read_str(&mut caller, path, path_len) {
                Some(s) => s,
                None => return errno::INVAL,
            };
            let node = match resolve_at(caller.data(), dirfd, &p) {
                Ok(n) => n,
                Err(e) => return e,
            };
            let fs = ramfs::fs();
            if node == 0 {
                return errno::ACCES;
            }
            if fs.nodes[node].kind != NodeKind::Dir {
                return errno::NOTDIR;
            }
            if !fs.is_empty_dir(node) {
                return errno::NOTEMPTY;
            }
            if !fs.can(fs.nodes[node].parent, PERM_W) {
                return errno::ACCES;
            }
            fs.nodes[node].used = false;
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_rename",
        |mut caller: Caller<'_, HostState>, old_fd: i32, old_p: i32, old_len: i32, new_fd: i32, new_p: i32, new_len: i32| -> i32 {
            let (op, np) = match (read_str(&mut caller, old_p, old_len), read_str(&mut caller, new_p, new_len)) {
                (Some(a), Some(b)) => (a, b),
                _ => return errno::INVAL,
            };
            let node = match resolve_at(caller.data(), old_fd, &op) {
                Ok(n) => n,
                Err(e) => return e,
            };
            let (new_parent, new_name) = match resolve_parent_at(caller.data(), new_fd, &np) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let fs = ramfs::fs();
            if !fs.can(fs.nodes[node].parent, PERM_W) || !fs.can(new_parent, PERM_W) {
                return errno::ACCES;
            }
            if let Some(dst) = fs.find_child(new_parent, new_name) {
                if dst == node {
                    return errno::SUCCESS;
                }
                if fs.nodes[dst].kind == NodeKind::Dir {
                    return errno::EXIST;
                }
                // Ecrase le fichier destination, comme rename(2).
                fs.nodes[dst].used = false;
                fs.nodes[dst].content = Vec::new();
            }
            fs.nodes[node].parent = new_parent;
            if !fs.nodes[node].set_name(new_name) {
                return errno::NAMETOOLONG;
            }
            errno::SUCCESS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_link",
        |_c: Caller<'_, HostState>, _ofd: i32, _ofl: i32, _op: i32, _ol: i32, _nfd: i32, _np: i32, _nl: i32| -> i32 {
            errno::NOSYS // pas de liens durs dans le RAMFS
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "path_readlink",
        |_c: Caller<'_, HostState>, _fd: i32, _p: i32, _pl: i32, _b: i32, _bl: i32, _bu: i32| -> i32 {
            errno::INVAL // pas de liens symboliques dans le RAMFS
        },
    )
    .ok();

    l
}

// ----------------------------------------------------------------------------
// Conversions de valeurs
// ----------------------------------------------------------------------------

fn zero_val(t: &ValueType) -> WVal {
    match t {
        ValueType::I32 => WVal::I32(0),
        ValueType::I64 => WVal::I64(0),
        ValueType::F32 => WVal::F32(0.0f32.into()),
        ValueType::F64 => WVal::F64(0.0f64.into()),
        ValueType::FuncRef => WVal::FuncRef(wasmi::FuncRef::null()),
        ValueType::ExternRef => WVal::ExternRef(wasmi::ExternRef::null()),
    }
}

fn coerce(t: &ValueType, a: f64) -> WVal {
    match t {
        ValueType::I32 => WVal::I32(a as i64 as i32),
        ValueType::I64 => WVal::I64(a as i64),
        ValueType::F32 => WVal::F32((a as f32).into()),
        ValueType::F64 => WVal::F64(a.into()),
        ValueType::FuncRef => WVal::FuncRef(wasmi::FuncRef::null()),
        ValueType::ExternRef => WVal::ExternRef(wasmi::ExternRef::null()),
    }
}

fn wval_to_f64(v: &WVal) -> f64 {
    match v {
        WVal::I32(x) => *x as f64,
        WVal::I64(x) => *x as f64,
        WVal::F32(x) => f32::from(*x) as f64,
        WVal::F64(x) => f64::from(*x),
        _ => 0.0,
    }
}

fn wval_to_i64(v: &WVal) -> Option<i64> {
    Some(match v {
        WVal::I32(x) => *x as i64,
        WVal::I64(x) => *x,
        WVal::F32(x) => f32::from(*x) as i64,
        WVal::F64(x) => f64::from(*x) as i64,
        _ => return None,
    })
}

fn make_engine() -> Engine {
    let mut config = Config::default();
    config.consume_fuel(true);
    Engine::new(&config)
}

/// Cache du dernier module parse : wasmi traduit tout le module a chaque
/// `Module::new`, ce qui coute plusieurs dizaines de secondes pour les 16 Mo
/// de l'interpreteur Python. Les octets etant `include_bytes!` (adresse
/// statique stable), on reutilise l'`Engine` + `Module` d'une invocation a
/// l'autre : seul le `Store` (memoire lineaire, etat) est recree. Noyau
/// mono-thread : le `static mut` est sur.
static mut MODULE_CACHE: Option<(usize, usize, Engine, WModule)> = None;

fn engine_and_module(bytes: &[u8]) -> Result<(&'static Engine, &'static WModule), String> {
    let key = (bytes.as_ptr() as usize, bytes.len());
    unsafe {
        let cache = &mut *core::ptr::addr_of_mut!(MODULE_CACHE);
        let hit = matches!(cache, Some((p, l, _, _)) if *p == key.0 && *l == key.1);
        if !hit {
            let engine = make_engine();
            let module = WModule::new(&engine, bytes).map_err(|e| format!("module invalide : {}", e))?;
            *cache = Some((key.0, key.1, engine, module));
        }
        match &*core::ptr::addr_of!(MODULE_CACHE) {
            Some((_, _, engine, module)) => Ok((engine, module)),
            None => Err("cache module indisponible".into()),
        }
    }
}

// ----------------------------------------------------------------------------
// Surface OS native
// ----------------------------------------------------------------------------

/// Instancie et execute un module `.wasm` : appelle `_start` / `main` / `run`,
/// renvoie la sortie texte et l'eventuel resultat numerique.
pub fn run_bytes(bytes: &[u8]) -> RunResult {
    run_program(bytes, Vec::new(), Vec::new(), Vec::new(), false, FUEL, 0)
}

/// Compatibilite : `run_bytes` + argv + stdin bufferise.
pub fn run_bytes_with_io(bytes: &[u8], args: Vec<String>, stdin: Vec<u8>) -> RunResult {
    run_program(bytes, args, Vec::new(), stdin, false, FUEL, 0)
}

/// Execution complete d'un programme WASI : argv, environnement, stdin
/// (bufferise, complete au clavier si `interactive`), acces fichiers RAMFS,
/// `cwd_node` = inode du repertoire courant (preouverture `.`).
pub fn run_program(
    bytes: &[u8],
    args: Vec<String>,
    env: Vec<(String, String)>,
    stdin: Vec<u8>,
    interactive: bool,
    fuel: u64,
    cwd_node: usize,
) -> RunResult {
    let (engine, module) = match engine_and_module(bytes) {
        Ok(v) => v,
        Err(e) => return RunResult { output: String::new(), result: None, exit_code: None, error: Some(e) },
    };
    let mut host = HostState::with_cwd(cwd_node);
    host.args = args;
    host.env = env;
    host.stdin = stdin;
    host.interactive = interactive;
    let mut store = Store::new(engine, host);
    let _ = store.add_fuel(fuel);
    let linker = build_linker(engine);
    let instance = match linker.instantiate(&mut store, &module).and_then(|pre| pre.start(&mut store)) {
        Ok(i) => i,
        Err(e) => {
            return RunResult { output: String::new(), result: None, exit_code: None, error: Some(format!("instanciation : {}", e)) }
        }
    };

    let mut result = None;
    let mut error = None;
    let entry = ["_start", "main", "run"].iter().find_map(|n| instance.get_func(&store, *n));
    if let Some(f) = entry {
        let ty = f.ty(&store);
        let params: Vec<WVal> = ty.params().iter().map(zero_val).collect();
        let mut results: Vec<WVal> = ty.results().iter().map(zero_val).collect();
        match f.call(&mut store, &params, &mut results) {
            Ok(()) => result = results.first().and_then(wval_to_i64),
            // Un `proc_exit` volontaire remonte comme un trap : pas une erreur.
            Err(_) if store.data().exit_code.is_some() => {}
            Err(e) => error = Some(format!("execution : {}", e)),
        }
    } else {
        error = Some("aucun point d'entree exporte (_start / main / run)".to_string());
    }

    let host = store.data();
    RunResult { output: host.out.clone(), result, exit_code: host.exit_code, error }
}

/// Valide un binaire WebAssembly (utilise par `WebAssembly.validate`).
pub fn validate(bytes: &[u8]) -> bool {
    let engine = make_engine();
    WModule::new(&engine, &bytes[..]).is_ok()
}

// ----------------------------------------------------------------------------
// Surface persistante : Instance (API JS WebAssembly)
// ----------------------------------------------------------------------------

/// Module WebAssembly instancie et persistant : conserve sa memoire lineaire et
/// son etat entre les appels. Expose la liste des fonctions exportees.
pub struct Instance {
    store: Store<HostState>,
    instance: wasmi::Instance,
    funcs: Vec<String>,
}

/// Instancie un module pour un usage persistant (appels repetes depuis JS).
pub fn instantiate(bytes: &[u8]) -> Result<Instance, String> {
    let engine = make_engine();
    let module = WModule::new(&engine, &bytes[..]).map_err(|e| format!("module invalide : {}", e))?;
    let funcs: Vec<String> = module
        .exports()
        .filter(|e| matches!(e.ty(), ExternType::Func(_)))
        .map(|e| e.name().to_string())
        .collect();
    let mut store = Store::new(&engine, HostState::new());
    let _ = store.add_fuel(FUEL);
    let linker = build_linker(&engine);
    let instance = linker
        .instantiate(&mut store, &module)
        .and_then(|pre| pre.start(&mut store))
        .map_err(|e| format!("instanciation : {}", e))?;
    Ok(Instance { store, instance, funcs })
}

impl Instance {
    /// Noms des fonctions exportees par le module.
    pub fn export_funcs(&self) -> &[String] {
        &self.funcs
    }

    /// Appelle une fonction exportee avec des arguments numeriques (coercion
    /// vers le type WASM attendu). Renvoie le premier resultat en `f64`.
    pub fn call(&mut self, name: &str, args: &[f64]) -> Result<Option<f64>, String> {
        let f = self
            .instance
            .get_func(&self.store, name)
            .ok_or_else(|| format!("fonction exportee absente : {}", name))?;
        // Recharge du fuel a chaque appel (chaque invocation a son budget).
        let _ = self.store.add_fuel(FUEL);
        let ty = f.ty(&self.store);
        let params: Vec<WVal> = ty
            .params()
            .iter()
            .enumerate()
            .map(|(i, t)| coerce(t, args.get(i).copied().unwrap_or(0.0)))
            .collect();
        let mut results: Vec<WVal> = ty.results().iter().map(zero_val).collect();
        f.call(&mut self.store, &params, &mut results).map_err(|e| format!("{}", e))?;
        Ok(results.first().map(wval_to_f64))
    }

    /// Sortie texte accumulee (env.print*, fd_write) depuis l'instanciation.
    pub fn output(&self) -> &str {
        &self.store.data().out
    }
}

// ----------------------------------------------------------------------------
// Selftest : module minimal genere a la main (pas de fichier requis)
// ----------------------------------------------------------------------------

/// Verifie la chaine complete parse -> validate -> instantiate -> call sur un
/// module exportant `add(i32, i32) -> i32` encode en dur.
pub fn selftest() -> Result<(), &'static str> {
    // (module (func (export "add") (param i32 i32) (result i32)
    //   local.get 0  local.get 1  i32.add))
    const ADD_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f, // type: (i32,i32)->i32
        0x03, 0x02, 0x01, 0x00, // func: 1 fonction, type 0
        0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00, // export "add" func 0
        0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b, // code
    ];
    if !validate(ADD_WASM) {
        return Err("validation");
    }
    let mut inst = instantiate(ADD_WASM).map_err(|_| "instantiation")?;
    match inst.call("add", &[2.0, 3.0]) {
        Ok(Some(v)) if v == 5.0 => Ok(()),
        Ok(_) => Err("resultat"),
        Err(_) => Err("call"),
    }
}
