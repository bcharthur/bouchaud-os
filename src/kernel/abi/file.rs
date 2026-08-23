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
use crate::fs::backing;
use crate::fs::ramfs::{self, NodeKind};
use crate::kernel::abi::{errno, user_read, user_read_u64, user_write, verrous};
use crate::kernel::fd::{device_for_path, FdKind, FileDesc};
use crate::kernel::input;
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
/// `S_IFSOCK`. Une socket doit se declarer comme telle : `S_ISSOCK` est la
/// seule facon pour un programme de verifier qu'un descripteur herite est bien
/// un canal de communication et non un fichier quelconque. Ladybird s'en sert
/// avant d'adopter la socket que son lanceur lui passe
/// (`LibCore/SystemServerTakeover.cpp`), et refuse le descripteur si `fstat`
/// repond autre chose.
const S_IFSOCK: u32 = 0o140000;

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
    // En mode non interactif, `print!` recopie deja tout sur COM1 : ecrire ici
    // en plus afficherait chaque ligne du programme en double.
    if !crate::drivers::vga::serial_mirror() {
        for &byte in data {
            crate::serial_print!("{}", byte as char);
        }
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
        task::attends_un_tick();
    }
}

/// Drapeaux d'ouverture d'un descripteur (0 s'il n'existe pas).
///
/// Lus a chaque tour d'une attente et non une fois pour toutes : `fcntl` peut
/// basculer un descripteur en non bloquant depuis un autre thread pendant
/// qu'on attend dessus.
fn fd_flags(fd: i32) -> u32 {
    task::current_process()
        .borrow()
        .files
        .get(fd)
        .map(|desc| desc.flags)
        .unwrap_or(0)
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
        // Pas de capture : la sortie audio ne se lit pas. Rendre 0 vaut « fin
        // de fichier », ce qu'un programme sait interpreter, alors qu'une
        // erreur le ferait renoncer.
        FdKind::Audio => 0,
        FdKind::Zero => {
            let zeros = alloc::vec![0u8; count];
            if user_write(buffer, &zeros) {
                count as i64
            } else {
                -errno::EFAULT
            }
        }
        FdKind::Random => {
            let mut data = alloc::vec![0u8; count.min(4096)];
            crate::net::security::tls::rng::fill(&mut data);
            let len = data.len();
            if user_write(buffer, &data) {
                len as i64
            } else {
                -errno::EFAULT
            }
        }
        FdKind::File(node) => {
            let offset = process
                .borrow()
                .files
                .get(fd)
                .map(|d| d.offset)
                .unwrap_or(0);
            let total = backing::logical_len(node);
            if offset >= total {
                return 0;
            }
            let wanted = core::cmp::min(count, total - offset);
            let mut data = alloc::vec![0u8; wanted];
            let got = backing::read_at(node, offset, &mut data);
            data.truncate(got);
            if !user_write(buffer, &data) {
                return -errno::EFAULT;
            }
            if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
                desc.offset = offset + got;
            }
            got as i64
        }
        // Instantane : un fichier ordinaire, mais dont le contenu vit dans le
        // descripteur et non dans le RAMFS. La glibc lit `/proc/self/maps` par
        // `getdelim`, donc en plusieurs `read` successifs : le decalage doit
        // avancer comme pour un vrai fichier.
        FdKind::Instantane(ref contenu) => {
            let offset = process
                .borrow()
                .files
                .get(fd)
                .map(|d| d.offset)
                .unwrap_or(0);
            if offset >= contenu.len() {
                return 0;
            }
            let fin = core::cmp::min(contenu.len(), offset + count);
            if !user_write(buffer, &contenu[offset..fin]) {
                return -errno::EFAULT;
            }
            if let Some(desc) = process.borrow_mut().files.get_mut(fd) {
                desc.offset = fin;
            }
            (fin - offset) as i64
        }
        FdKind::Dir(_) => -errno::EISDIR,
        FdKind::Framebuffer | FdKind::VirtualTerminal => -errno::EINVAL,
        FdKind::InputKeyboard => read_input(buffer, count, input::Device::Keyboard),
        FdKind::InputMouse => read_input(buffer, count, input::Device::Mouse),
        FdKind::Pipe(shared, readable) => {
            if !readable {
                return -errno::EBADF;
            }
            let non_bloquant = fd_flags(fd) & O_NONBLOCK != 0;
            loop {
                // L'emprunt du tampon ne doit jamais survivre a l'attente :
                // l'ecrivain qu'on attend a besoin du meme `RefCell`.
                let (vide, plus_d_ecrivain) = {
                    let state = shared.borrow();
                    (state.buffer.is_empty(), state.writers == 0)
                };
                if !vide {
                    break;
                }
                // Tube vide et plus personne pour ecrire : c'est la fin de
                // fichier, pas une attente.
                if plus_d_ecrivain {
                    return 0;
                }
                if non_bloquant {
                    return -errno::EAGAIN;
                }
                // Un `read` bloquant attend. Sans cela, tout code qui lit un
                // tube sans passer par `poll` — c'est-a-dire l'immense
                // majorite — recevrait EAGAIN sur un tube parfaitement valide.
                // BOUCHAUD_SMP_BLOCKING_IO_FIX_V1: une attente bloquante doit
                // marquer la tache Blocked afin que schedule() libere le BKL global
                // pendant que le producteur/consommateur progresse sur un autre CPU.
                task::attends_un_tick();
            }
            let mut state = shared.borrow_mut();
            let len = core::cmp::min(count, state.buffer.len());
            let data: Vec<u8> = state.buffer.drain(..len).collect();
            drop(state);
            if user_write(buffer, &data) {
                len as i64
            } else {
                -errno::EFAULT
            }
        }
        FdKind::EventFd(state) => {
            if count < 8 {
                return -errno::EINVAL;
            }
            let mut state = state.borrow_mut();
            if state.counter == 0 {
                return -errno::EAGAIN;
            }
            // Mode normal : on rend le compteur entier et on le vide. Mode
            // semaphore : une unite a la fois.
            let value = if state.semaphore { 1 } else { state.counter };
            state.counter -= value;
            drop(state);
            if user_write(buffer, &value.to_le_bytes()) {
                8
            } else {
                -errno::EFAULT
            }
        }
        FdKind::TimerFd(state) => {
            if count < 8 {
                return -errno::EINVAL;
            }
            let expired = {
                let mut state = state.borrow_mut();
                refresh_timerfd(&mut state);
                let value = state.expirations;
                state.expirations = 0;
                value
            };
            if expired == 0 {
                return -errno::EAGAIN;
            }
            if user_write(buffer, &expired.to_le_bytes()) {
                8
            } else {
                -errno::EFAULT
            }
        }
        FdKind::Socket(_) => crate::kernel::abi::net::sys_recvfrom(fd, buffer, count, 0, 0, 0),
        FdKind::SocketPair(inbox, _) => {
            // Une lecture bloquante attend que l'autre bout ecrive. Rendre
            // `EAGAIN` tout de suite, comme on le faisait, obligeait chaque
            // appelant a boucler lui-meme — et surtout, cela faisait echouer le
            // premier `recvmsg` d'un dialogue entre deux processus, celui qui
            // arrive avant que le pair ait eu la main.
            if inbox.borrow().octets.is_empty() && fd_flags(fd) & O_NONBLOCK == 0 {
                let echeance =
                    crate::kernel::timer::ticks() + crate::kernel::timer::ms_to_ticks(2000);
                while inbox.borrow().octets.is_empty() && crate::kernel::timer::ticks() < echeance {
                    task::attends_un_tick();
                }
            }
            let mut guard = inbox.borrow_mut();
            if guard.octets.is_empty() {
                return -errno::EAGAIN;
            }
            let len = core::cmp::min(count, guard.octets.len());
            let data: Vec<u8> = guard.octets.drain(..len).collect();
            drop(guard);
            if user_write(buffer, &data) {
                len as i64
            } else {
                -errno::EFAULT
            }
        }
        FdKind::Epoll(_) => -errno::EINVAL,
    }
}

/// Met a jour le compteur d'expirations d'un `timerfd` en fonction de l'heure.
fn refresh_timerfd(state: &mut crate::kernel::fd::TimerFdState) {
    if state.deadline == 0 {
        return;
    }
    let now = crate::kernel::timer::ticks();
    if now < state.deadline {
        return;
    }
    if state.interval == 0 {
        state.expirations += 1;
        state.deadline = 0; // one-shot : desarme
    } else {
        let late = now - state.deadline;
        let count = late / state.interval + 1;
        state.expirations += count;
        state.deadline += count * state.interval;
    }
}

/// `write`.
/// `write`.
pub fn sys_write(fd: i32, buffer: u64, count: usize) -> i64 {
    if count == 0 {
        return 0;
    }
    let data = match user_read(buffer, count) {
        Some(data) => data,
        None => return -errno::EFAULT,
    };
    ecrit_octets(fd, &data)
}

/// `write`, une fois les octets deja en memoire noyau.
///
/// Separe de [`sys_write`] pour que `sendfile` puisse ecrire vers n'importe
/// quelle sorte de descripteur sans repasser par un tampon utilisateur : la
/// source d'un `sendfile` est un fichier, sa destination est le plus souvent
/// une socket ou un tube, et les deux bouts vivent dans le noyau.
pub fn ecrit_octets(fd: i32, data: &[u8]) -> i64 {
    let count = data.len();
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
            console_write(&data);
            count as i64
        }
        FdKind::Null => count as i64,
        FdKind::Zero => count as i64,
        FdKind::Random => count as i64,
        // `/proc/self/maps` decrit un etat, il ne le recoit pas.
        FdKind::Instantane(_) => -errno::EBADF,
        FdKind::Audio => {
            if !crate::drivers::ac97::pret() && !crate::drivers::ac97::init() {
                return -errno::ENODEV;
            }
            let ecrits = crate::drivers::ac97::ecrit(&data);
            if ecrits == 0 {
                // Les tampons sont pleins. En mode bloquant on attend qu'un
                // tampon se libere plutot que de rendre une erreur : c'est ce
                // qu'attend un programme qui pousse du son en boucle.
                let bloquant = {
                    let borrowed = process.borrow();
                    borrowed
                        .files
                        .get(fd)
                        .map(|d| d.flags & O_NONBLOCK == 0)
                        .unwrap_or(true)
                };
                if !bloquant {
                    return -errno::EAGAIN;
                }
                let echeance =
                    crate::kernel::timer::ticks() + crate::kernel::timer::ms_to_ticks(200);
                while crate::drivers::ac97::libres() == 0
                    && crate::kernel::timer::ticks() < echeance
                {
                    task::attends_un_tick();
                }
                let ecrits = crate::drivers::ac97::ecrit(&data);
                if ecrits == 0 {
                    -errno::EAGAIN
                } else {
                    ecrits as i64
                }
            } else {
                ecrits as i64
            }
        }
        FdKind::File(node) => {
            if backing::is_disk_backed(node) {
                return -errno::EROFS;
            }
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
        FdKind::Framebuffer | FdKind::VirtualTerminal => -errno::EINVAL,
        FdKind::InputKeyboard | FdKind::InputMouse => count as i64,
        FdKind::Pipe(shared, readable) => {
            if readable {
                return -errno::EBADF;
            }
            let non_bloquant = fd_flags(fd) & O_NONBLOCK != 0;
            let place = match attends_place(
                || {
                    let state = shared.borrow();
                    if state.readers == 0 {
                        Capacite::Rompu
                    } else {
                        Capacite::Place(state.place())
                    }
                },
                non_bloquant,
            ) {
                Ok(place) => place,
                Err(erreur) => return erreur,
            };
            // Ecriture courte : on ecrit ce qui tient et on le dit. C'est la
            // semantique POSIX d'un tube presque plein, et c'est elle qui donne
            // sa contre-pression a l'appelant — un `write` qui rendrait `count`
            // sans avoir tout ecrit ferait perdre des octets en silence.
            let mut state = shared.borrow_mut();
            let ecrits = core::cmp::min(place, data.len());
            state.buffer.extend_from_slice(&data[..ecrits]);
            ecrits as i64
        }
        FdKind::EventFd(state) => {
            if count < 8 {
                return -errno::EINVAL;
            }
            let mut value = [0u8; 8];
            value.copy_from_slice(&data[..8]);
            let value = u64::from_le_bytes(value);
            // u64::MAX est reserve par l'ABI (valeur interdite en ecriture).
            if value == u64::MAX {
                return -errno::EINVAL;
            }
            state.borrow_mut().counter += value;
            8
        }
        FdKind::TimerFd(_) => -errno::EINVAL,
        FdKind::Socket(_) => crate::kernel::abi::net::envoie_octets(fd, data, 0, 0, 0),
        FdKind::SocketPair(_, outbox) => {
            let non_bloquant = fd_flags(fd) & O_NONBLOCK != 0;
            let place = match attends_place(
                || {
                    let canal = outbox.borrow();
                    if canal.lecteurs == 0 {
                        Capacite::Rompu
                    } else {
                        Capacite::Place(canal.place())
                    }
                },
                non_bloquant,
            ) {
                Ok(place) => place,
                Err(erreur) => return erreur,
            };
            let mut canal = outbox.borrow_mut();
            let ecrits = core::cmp::min(place, data.len());
            canal.octets.extend_from_slice(&data[..ecrits]);
            ecrits as i64
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
        // Ecriture courte : le canal est plein. Passer au vecteur suivant
        // laisserait un trou au milieu du flux — l'appelant croirait avoir
        // transmis une suite continue alors qu'il manquerait sa fin. On rend ce
        // qui est parti, et c'est a lui de reprendre a cet endroit.
        if (result as usize) < len {
            break;
        }
    }
    total
}

/// `sendfile`.
///
/// Copie jusqu'a `count` octets de `in_fd` vers `out_fd` sans passer par
/// l'espace utilisateur. Si `offset_ptr` n'est pas nul, la lecture commence a
/// `*offset_ptr`, la position de `in_fd` n'est PAS modifiee, et `*offset_ptr`
/// avance du nombre d'octets lus ; sinon la position du descripteur sert de
/// point de depart et avance.
///
/// Linux exige que `in_fd` designe quelque chose de projetable en memoire,
/// c'est-a-dire un fichier ordinaire, et rend EINVAL sinon. Bouchaud repond de
/// meme : un tube ou une socket en source rend EINVAL, pas un resultat partiel
/// qui laisserait croire a une copie.
///
/// L'appel manquait, et RequestServer s'en sert precisement pour servir une
/// reponse HTTP depuis son cache disque
/// (`Core::System::transfer_file_through_socket`). Au run 32474068384, au
/// SECOND demarrage -- celui ou le cache existe deja --, l'ENOSYS remontait
/// jusqu'au JavaScript sous la forme « RequestServer encountered an error
/// reading a cached HTTP response », et le test HTTPS echouait alors qu'il
/// avait reussi au demarrage precedent.
pub fn sys_sendfile(out_fd: i32, in_fd: i32, offset_ptr: u64, count: usize) -> i64 {
    if count == 0 {
        return 0;
    }
    let process = task::current_process();

    let (source, position_courante) = {
        let borrowed = process.borrow();
        match borrowed.files.get(in_fd) {
            Some(desc) => (desc.kind.clone(), desc.offset),
            None => return -errno::EBADF,
        }
    };
    if process.borrow().files.get(out_fd).is_none() {
        return -errno::EBADF;
    }

    // La source doit etre un fichier : c'est la regle de Linux, et la seule
    // qui permette de lire a un decalage sans consommer un flux.
    let node = match source {
        FdKind::File(node) => node,
        _ => return -errno::EINVAL,
    };

    let explicite = offset_ptr != 0;
    let depart = if explicite {
        match user_read_u64(offset_ptr) {
            Some(valeur) => valeur as usize,
            None => return -errno::EFAULT,
        }
    } else {
        position_courante
    };

    let total = backing::logical_len(node);
    if depart >= total {
        return 0;
    }
    let voulu = core::cmp::min(count, total - depart);

    // Par tranches : `count` vient de l'application et peut valoir la taille
    // entiere d'une reponse HTTP. Une seule allocation de cette taille dans le
    // tas noyau serait un cout inutile, et un refus d'allocation transformerait
    // une copie en panne.
    const TRANCHE: usize = 64 * 1024;
    let mut tampon = alloc::vec![0u8; core::cmp::min(voulu, TRANCHE)];
    let mut envoyes = 0usize;

    while envoyes < voulu {
        let bloc = core::cmp::min(TRANCHE, voulu - envoyes);
        let lus = backing::read_at(node, depart + envoyes, &mut tampon[..bloc]);
        if lus == 0 {
            break;
        }
        let ecrits = ecrit_octets(out_fd, &tampon[..lus]);
        if ecrits < 0 {
            // Rien n'est encore parti : l'erreur est celle de l'appel. Si en
            // revanche des octets ont deja ete transmis, c'est ce compte qui
            // fait foi -- un appelant doit pouvoir reprendre la ou il en est.
            if envoyes == 0 {
                return ecrits;
            }
            break;
        }
        envoyes += ecrits as usize;
        if (ecrits as usize) < lus {
            // Ecriture partielle : la destination est pleine.
            break;
        }
    }

    if explicite {
        if !user_write(offset_ptr, &((depart + envoyes) as u64).to_le_bytes()) {
            return -errno::EFAULT;
        }
    } else if let Some(desc) = process.borrow_mut().files.get_mut(in_fd) {
        desc.offset = depart + envoyes;
    }

    envoyes as i64
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
/// `memfd_create` : un fichier anonyme en memoire.
///
/// Sans nom dans l'arborescence, mais mappable en `MAP_SHARED` — et c'est tout
/// l'interet. Un moteur web multi-processus s'en sert pour ses tampons
/// d'image : le processus qui dessine et celui qui compose voient la meme
/// memoire physique, sans copie. Le nœud etant anonyme, il disparait quand le
/// dernier descripteur se ferme.
pub fn sys_memfd_create(nom_addr: u64, _flags: u32) -> i64 {
    let nom = match super::user_string(nom_addr) {
        Some(nom) => nom,
        None => return -errno::EFAULT,
    };
    let idx = match crate::fs::ramfs::fs().cree_anonyme(&nom) {
        Ok(idx) => idx,
        Err(_) => return -errno::ENFILE,
    };
    let process = task::current_process();
    let mut borrowed = process.borrow_mut();
    // `MFD_CLOEXEC` vaut 1 : c'est le seul drapeau que les appelants posent en
    // pratique, et le respecter evite qu'un tampon fuie dans un `execve`.
    let mut desc = FileDesc::new(FdKind::File(idx));
    desc.cloexec = _flags & 1 != 0;
    let fd = borrowed.files.insert(desc);
    if fd < 0 {
        -errno::EMFILE
    } else {
        fd as i64
    }
}

/// Traduit l'echec d'une creation de nœud en numero d'erreur POSIX.
///
/// Ces echecs rendaient tous ENOSPC. Un nom trop long se presentait donc a
/// l'application comme un disque plein, ce qui envoie chercher la cause a
/// l'oppose de la verite : au run 32427953935, chaque telechargement de
/// Ladybird echouait sur un nom de 67 octets et le journal annoncait ENOSPC
/// sur un disque de 1166 Mio dont 4 % des inodes seulement etaient pris. La
/// couche WASI faisait deja cette distinction ; l'ABI native, non.
fn errno_creation(raison: &'static str) -> i64 {
    match raison {
        "invalid name" => errno::ENAMETOOLONG,
        "no free inode" => errno::ENOSPC,
        "parent not a directory" => errno::ENOTDIR,
        "already exists" => errno::EEXIST,
        _ => errno::ENOSPC,
    }
}

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
        // Un ecran virtuel ne demande rien au materiel : la carte est deja
        // programmee par le gestionnaire de fenetres, qui la garde. Appeler
        // `enter()` ici reinitialiserait BGA et le double-tampon du bureau au
        // beau milieu d'une session — c'est-a-dire ferait clignoter l'ecran
        // chaque fois qu'une application s'ouvre.
        if matches!(kind, FdKind::Framebuffer)
            && ecran_virtuel().is_none()
            && !crate::drivers::gfx::is_active()
        {
            crate::drivers::gfx::enter();
        }
        // Un client du gestionnaire de fenetres ne lit pas les entrees : c'est
        // le bureau qui possede le clavier et la souris, et qui lui transmet ce
        // qui le concerne, converti dans le repere de sa fenetre.
        //
        // Le refus est ici, dans le noyau, et non dans la configuration du
        // client. Les deux consommateurs puisent dans la *meme* file de
        // scancodes : les laisser coexister ne partagerait pas les touches, il
        // en perdrait une sur deux de chaque cote. C'est le genre de defaut qui
        // se diagnostique en une heure et se reproduit a chaque nouveau client
        // — a moins que le systeme ne le rende impossible.
        if matches!(kind, FdKind::InputKeyboard | FdKind::InputMouse) && ecran_virtuel().is_some() {
            return -errno::EACCES;
        }
        // De meme pour la souris : son IRQ n'est armee qu'a l'entree du bureau.
        // Sans cela, /dev/input/event1 resterait muet. On prend ensuite un
        // instantane de son etat pour que la premiere lecture parte de zero.
        if matches!(kind, FdKind::InputMouse) {
            crate::drivers::mouse::init();
            input::sync_mouse();
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
                Err(raison) => return -errno_creation(raison),
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
    if !is_dir
        && backing::is_disk_backed(node)
        && (access == O_WRONLY || access == O_RDWR || flags & O_TRUNC != 0)
    {
        return -errno::EROFS;
    }
    if flags & O_TRUNC != 0 && !is_dir {
        fs.nodes[node].content.clear();
    }

    let mut desc = FileDesc::new(if is_dir {
        FdKind::Dir(node)
    } else {
        FdKind::File(node)
    });
    desc.flags = flags;
    desc.cloexec = flags & O_CLOEXEC != 0;
    if flags & O_APPEND != 0 {
        desc.offset = backing::disk_len(node).unwrap_or(fs.nodes[node].content.len());
    }
    let fd = process.borrow_mut().files.insert(desc);
    fd as i64
}

/// `close`.
pub fn sys_close(fd: i32) -> i64 {
    let process = task::current_process();

    // POSIX : fermer n'importe quel descripteur d'un processus sur un fichier
    // relache TOUS les verrous d'enregistrement de ce processus sur ce fichier,
    // meme s'il lui en reste d'autres ouverts dessus. SQLite compte dessus pour
    // rendre la base a la fermeture ; sans cela un verrou survit au `close` et
    // la reouverture suivante se croit bloquee par un fantome.
    let verrouille = {
        let borrowed = process.borrow();
        match borrowed.files.get(fd) {
            Some(desc) => match desc.kind {
                FdKind::File(node) => Some((node, borrowed.pid)),
                _ => None,
            },
            None => None,
        }
    };

    let closed = process.borrow_mut().files.close(fd);
    if let Some((node, pid)) = verrouille {
        verrous::libere_fichier(node, pid);
    }
    if closed {
        0
    } else {
        -errno::EBADF
    }
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
        FdKind::File(node) => backing::logical_len(node) as i64,
        FdKind::Dir(_) => 0,
        FdKind::Instantane(ref contenu) => contenu.len() as i64,
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
    let shared = crate::kernel::fd::new_pipe();
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
    const F_GETLK: u32 = 5;
    const F_SETLK: u32 = 6;
    const F_SETLKW: u32 = 7;
    const F_DUPFD_CLOEXEC: u32 = 1030;
    const FD_CLOEXEC: u64 = 1;

    // Les verrous d'enregistrement prennent leur propre chemin : ils lisent et
    // reecrivent une structure utilisateur, et `F_SETLKW` attend, ce qu'on ne
    // peut pas faire en tenant le `RefCell` du processus.
    if command == F_GETLK || command == F_SETLK || command == F_SETLKW {
        return fcntl_verrou(fd, command, arg);
    }

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
                if desc.cloexec {
                    FD_CLOEXEC as i64
                } else {
                    0
                }
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
        // Une commande qu'on ne sait pas honorer doit le dire. Repondre « 0 »
        // a tout faisait croire a chaque appelant que sa demande avait ete
        // prise en compte -- c'est ainsi que les verrous d'enregistrement
        // semblaient fonctionner alors que rien n'etait pose.
        _ => -errno::EINVAL,
    }
}

/// `struct flock` de l'ABI Linux x86-64, 32 octets :
///
/// ```text
/// 0   i16  l_type      F_RDLCK / F_WRLCK / F_UNLCK
/// 2   i16  l_whence    SEEK_SET / SEEK_CUR / SEEK_END
/// 4        (bourrage d'alignement pour l_start)
/// 8   i64  l_start
/// 16  i64  l_len       0 = jusqu'a la fin du fichier
/// 24  i32  l_pid       rempli par F_GETLK
/// 28       (bourrage de fin)
/// ```
const TAILLE_FLOCK: usize = 32;

/// `fcntl(F_GETLK/F_SETLK/F_SETLKW)` : verrous d'enregistrement POSIX.
///
/// Voir `crate::kernel::abi::verrous` pour le modele et pour ce dont SQLite
/// depend exactement.
fn fcntl_verrou(fd: i32, command: u32, arg: u64) -> i64 {
    const F_GETLK: u32 = 5;
    const F_SETLKW: u32 = 7;
    const SEEK_SET: i16 = 0;
    const SEEK_CUR: i16 = 1;
    const SEEK_END: i16 = 2;

    let brut = match user_read(arg, TAILLE_FLOCK) {
        Some(octets) => octets,
        None => return -errno::EFAULT,
    };
    let genre = i16::from_le_bytes([brut[0], brut[1]]);
    let origine = i16::from_le_bytes([brut[2], brut[3]]);
    let depart = i64::from_le_bytes(brut[8..16].try_into().unwrap());
    let longueur = i64::from_le_bytes(brut[16..24].try_into().unwrap());

    if genre != verrous::F_RDLCK && genre != verrous::F_WRLCK && genre != verrous::F_UNLCK {
        return -errno::EINVAL;
    }

    // Identite du fichier et contexte necessaires a la resolution de l_whence.
    let process = task::current_process();
    let (noeud, position, taille, pid) = {
        let borrowed = process.borrow();
        let desc = match borrowed.files.get(fd) {
            Some(desc) => desc,
            None => return -errno::EBADF,
        };
        let noeud = match desc.kind {
            FdKind::File(node) => node,
            // POSIX : les verrous d'enregistrement ne valent que pour les
            // fichiers ordinaires. Un tube ou un socket doit recevoir EINVAL,
            // pas un acquiescement muet.
            _ => return -errno::EINVAL,
        };
        (
            noeud,
            desc.offset as i64,
            backing::logical_len(noeud) as i64,
            borrowed.pid,
        )
    };

    let base = match origine {
        SEEK_SET => 0,
        SEEK_CUR => position,
        SEEK_END => taille,
        _ => return -errno::EINVAL,
    };

    // Une longueur negative decrit la plage qui PRECEDE l_start (POSIX.1-2001).
    let (debut_signe, longueur_absolue) = if longueur < 0 {
        (base + depart + longueur, -longueur)
    } else {
        (base + depart, longueur)
    };
    if debut_signe < 0 {
        return -errno::EINVAL;
    }
    let debut = debut_signe as u64;
    let longueur_absolue = longueur_absolue as u64;

    if command == F_GETLK {
        let mut sortie = brut.clone();
        match verrous::interroge(noeud, pid, genre, debut, longueur_absolue) {
            Some((genre_bloquant, debut_bloquant, longueur_bloquante, pid_bloquant)) => {
                sortie[0..2].copy_from_slice(&genre_bloquant.to_le_bytes());
                sortie[2..4].copy_from_slice(&SEEK_SET.to_le_bytes());
                sortie[8..16].copy_from_slice(&(debut_bloquant as i64).to_le_bytes());
                sortie[16..24].copy_from_slice(&(longueur_bloquante as i64).to_le_bytes());
                sortie[24..28].copy_from_slice(&(pid_bloquant as i32).to_le_bytes());
            }
            None => {
                // Rien ne bloque : c'est CETTE ecriture que SQLite lit pour
                // decider qu'il peut avancer.
                sortie[0..2].copy_from_slice(&verrous::F_UNLCK.to_le_bytes());
            }
        }
        if !user_write(arg, &sortie) {
            return -errno::EFAULT;
        }
        return 0;
    }

    // F_SETLK / F_SETLKW.
    //
    // `F_SETLKW` attend que la plage se libere. Bouchaud n'a pas de file
    // d'attente par verrou ; on rend la main a l'ordonnanceur, comme le fait
    // deja une lecture bloquante sur un tube. La patience est bornee : un
    // interblocage franc doit se voir en EDEADLK, pas figer le systeme.
    let patience = 5 * crate::kernel::timer::TICKS_PER_SECOND;
    let depart_attente = crate::kernel::timer::ticks();
    loop {
        match verrous::pose(noeud, pid, genre, debut, longueur_absolue) {
            verrous::Pose::Accorde => return 0,
            verrous::Pose::Occupe => {
                if command != F_SETLKW {
                    return -errno::EAGAIN;
                }
                if crate::kernel::timer::ticks().saturating_sub(depart_attente) > patience {
                    return -errno::EDEADLK;
                }
                // BOUCHAUD_SMP_BLOCKING_IO_FIX_V1: une attente bloquante doit
                // marquer la tache Blocked afin que schedule() libere le BKL global
                // pendant que le producteur/consommateur progresse sur un autre CPU.
                task::attends_un_tick();
            }
        }
    }
}

// --- Metadonnees -------------------------------------------------------------

/// Remplit un `struct stat` x86-64 (144 octets) pour un nœud RAMFS.
fn fill_stat(node: usize) -> [u8; 144] {
    let fs = ramfs::fs();
    let entry = &fs.nodes[node];
    let mode = (entry.mode as u32 & 0o7777)
        | if entry.kind == NodeKind::Dir {
            S_IFDIR
        } else {
            S_IFREG
        };
    let size = if entry.kind == NodeKind::Dir {
        0
    } else {
        backing::disk_len(node).unwrap_or(entry.content.len()) as u64
    };
    stat_bytes(node as u64, mode, entry.uid as u32, entry.gid as u32, size)
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
        // Un instantane est un fichier ordinaire, pas un peripherique : c'est
        // ce que `stat` doit dire, et avec sa taille. La glibc dimensionne le
        // tampon de `fopen` dessus ; annoncer 0 octet la ferait lire
        // caractere par caractere, et un `S_IFCHR` lui interdirait de chercher.
        let (mode, taille) = match kind {
            FdKind::Instantane(ref contenu) => (S_IFREG | 0o444, contenu.len() as u64),
            FdKind::Framebuffer | FdKind::InputKeyboard | FdKind::InputMouse => {
                (S_IFCHR | 0o660, 0)
            }
            _ => (S_IFCHR | 0o666, 0),
        };
        let buffer = stat_bytes(1, mode, 0, 0, taille);
        return if user_write(out, &buffer) {
            0
        } else {
            -errno::EFAULT
        };
    }
    match resolve(&path) {
        Some(node) => {
            let buffer = fill_stat(node);
            if user_write(out, &buffer) {
                0
            } else {
                -errno::EFAULT
            }
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
        // Les deux sortes de sockets. Sans ce cas, elles tombaient dans le
        // fourre-tout `S_IFCHR` ci-dessous et `S_ISSOCK` etait faux : le
        // lanceur de Ladybird passe ses services par `SOCKET_TAKEOVER`, et le
        // service refusait le descripteur avant meme d'ouvrir sa boucle
        // d'evenements.
        FdKind::Socket(_) | FdKind::SocketPair(_, _) => {
            stat_bytes(0, S_IFSOCK | 0o777, 0, 0, 0)
        }
        // Un fichier ordinaire, avec sa taille : la glibc dimensionne le tampon
        // de `fopen` dessus. Le declarer caractere ferait lire octet par octet.
        FdKind::Instantane(ref contenu) => {
            stat_bytes(0, S_IFREG | 0o444, 0, 0, contenu.len() as u64)
        }
        FdKind::Framebuffer => {
            let (_, hauteur, pas) = geometrie_ecran();
            stat_bytes(1, S_IFCHR | 0o660, 0, 0, (pas * hauteur) as u64)
        }
        _ => stat_bytes(0, S_IFCHR | 0o666, 0, 0, 0),
    };
    if user_write(out, &buffer) {
        0
    } else {
        -errno::EFAULT
    }
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
                | if entry.kind == NodeKind::Dir {
                    S_IFDIR
                } else {
                    S_IFREG
                };
            let size = if entry.kind == NodeKind::Dir {
                0
            } else {
                backing::disk_len(node).unwrap_or(entry.content.len()) as u64
            };
            (mode, size, entry.uid as u32, entry.gid as u32, node as u64)
        }
        None if !path.is_empty() && device_for_path(&absolute(&path)).is_some() => {
            match device_for_path(&absolute(&path)) {
                Some(FdKind::Instantane(contenu)) => {
                    (S_IFREG | 0o444, contenu.len() as u64, 0, 0, 1)
                }
                _ => (S_IFCHR | 0o666, 0, 0, 0, 1),
            }
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
    if user_write(out, &buffer) {
        0
    } else {
        -errno::EFAULT
    }
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
            children.push((
                index,
                entry.name_str().to_string(),
                entry.kind == NodeKind::Dir,
            ));
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
        Err(raison) => -errno_creation(raison),
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
            if backing::is_disk_backed(node) {
                return -errno::EROFS;
            }
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
        Some(node) if node != 0 && backing::is_disk_backed(node) => return -errno::EROFS,
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
    if backing::is_disk_backed(node) {
        return -errno::EROFS;
    }
    if length > ramfs::MAX_FILE_SIZE {
        return -errno::EFBIG;
    }
    ramfs::fs().nodes[node].content.resize(length, 0);
    0
}

// --- ioctl -------------------------------------------------------------------

// --- Sortie audio, protocole OSS --------------------------------------------
// Les numeros portent leur taille et leur sens de transfert, comme tout ioctl
// Linux : `0xC0045002` = lecture+ecriture d'un entier de 4 octets, groupe 'P',
// numero 2.
const SNDCTL_DSP_RESET: u64 = 0x0000_5000;
const SNDCTL_DSP_SYNC: u64 = 0x0000_5001;
const SNDCTL_DSP_SPEED: u64 = 0xC004_5002;
const SNDCTL_DSP_STEREO: u64 = 0xC004_5003;
const SNDCTL_DSP_GETBLKSIZE: u64 = 0xC004_5004;
const SNDCTL_DSP_SETFMT: u64 = 0xC004_5005;
const SNDCTL_DSP_CHANNELS: u64 = 0xC004_5006;
const SNDCTL_DSP_POST: u64 = 0x0000_5008;
const SNDCTL_DSP_GETFMTS: u64 = 0x8004_500B;
const SNDCTL_DSP_GETOSPACE: u64 = 0x800C_500C;
const SNDCTL_DSP_GETODELAY: u64 = 0x8004_5017;
/// PCM 8 bits non signe.
const AFMT_U8: u32 = 0x0000_0008;
/// PCM 16 bits signe, petit-boutiste.
const AFMT_S16_LE: u32 = 0x0000_0010;

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

// Console virtuelle : mode graphique et gestion des VT. Le plugin `linuxfb` de
// Qt ouvre /dev/tty0 et bascule le terminal en KD_GRAPHICS pour que le noyau
// cesse d'ecrire du texte par-dessus le framebuffer.
const KDGETMODE: u64 = 0x4B3B;
const KDSETMODE: u64 = 0x4B3A;
const KDGKBMODE: u64 = 0x4B44;
const KDSKBMODE: u64 = 0x4B45;
const VT_GETMODE: u64 = 0x5601;
const VT_SETMODE: u64 = 0x5602;
const VT_GETSTATE: u64 = 0x5603;
const VT_ACTIVATE: u64 = 0x5606;
const VT_WAITACTIVE: u64 = 0x5607;

/// `ioctl`.
pub fn sys_ioctl(fd: i32, request: u64, arg: u64) -> i64 {
    // Le numero d'ioctl est un `int` cote appelant. Ceux dont le bit de poids
    // fort est mis — c'est-a-dire tous ceux qui transferent des donnees vers
    // l'appelant, dont la famille OSS `SNDCTL_*` — arrivent donc **etendus en
    // signe** sur 64 bits : `0xC0045002` devient `0xFFFFFFFFC0045002`, et
    // aucune comparaison ne correspond plus. On ne garde que les 32 bits utiles.
    let request = request & 0xFFFF_FFFF;
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return -errno::EBADF,
    };

    match kind {
        // --- Sortie audio, protocole OSS -------------------------------------
        //
        // OSS plutot qu'ALSA : quatre ioctls suffisent a regler un flux, alors
        // qu'ALSA demande une machine a etats et une zone de controle partagee.
        // C'est aussi ce que SDL et la plupart des lecteurs essaient en premier
        // quand ils ne trouvent pas mieux.
        FdKind::Audio => {
            if !crate::drivers::ac97::pret() && !crate::drivers::ac97::init() {
                return -errno::ENODEV;
            }
            match request {
                SNDCTL_DSP_RESET | SNDCTL_DSP_SYNC | SNDCTL_DSP_POST => {
                    if request == SNDCTL_DSP_RESET {
                        crate::drivers::ac97::arrete();
                    }
                    0
                }
                SNDCTL_DSP_SPEED => {
                    let demande = match super::user_read_u32(arg) {
                        Some(v) => v,
                        None => return -errno::EFAULT,
                    };
                    let (_, voies, bits) = crate::drivers::ac97::format();
                    let (retenue, _, _) = crate::drivers::ac97::configure(demande, voies, bits);
                    if !user_write(arg, &retenue.to_le_bytes()) {
                        return -errno::EFAULT;
                    }
                    0
                }
                SNDCTL_DSP_CHANNELS => {
                    let demande = match super::user_read_u32(arg) {
                        Some(v) => v,
                        None => return -errno::EFAULT,
                    };
                    let (frequence, _, bits) = crate::drivers::ac97::format();
                    let (_, retenues, _) =
                        crate::drivers::ac97::configure(frequence, demande.min(255) as u8, bits);
                    if !user_write(arg, &(retenues as u32).to_le_bytes()) {
                        return -errno::EFAULT;
                    }
                    0
                }
                SNDCTL_DSP_STEREO => {
                    let demande = match super::user_read_u32(arg) {
                        Some(v) => v,
                        None => return -errno::EFAULT,
                    };
                    let (frequence, _, bits) = crate::drivers::ac97::format();
                    let voies = if demande != 0 { 2 } else { 1 };
                    let (_, retenues, _) = crate::drivers::ac97::configure(frequence, voies, bits);
                    if !user_write(arg, &(retenues as u32 - 1).to_le_bytes()) {
                        return -errno::EFAULT;
                    }
                    0
                }
                SNDCTL_DSP_SETFMT => {
                    let demande = match super::user_read_u32(arg) {
                        Some(v) => v,
                        None => return -errno::EFAULT,
                    };
                    let (frequence, voies, _) = crate::drivers::ac97::format();
                    // On ne sait produire que ces deux-la ; toute autre demande
                    // se voit repondre ce qu'on fera reellement, comme le veut
                    // le protocole.
                    let bits = if demande == AFMT_U8 { 8 } else { 16 };
                    let (_, _, retenus) = crate::drivers::ac97::configure(frequence, voies, bits);
                    let format = if retenus == 8 { AFMT_U8 } else { AFMT_S16_LE };
                    if !user_write(arg, &format.to_le_bytes()) {
                        return -errno::EFAULT;
                    }
                    0
                }
                SNDCTL_DSP_GETFMTS => {
                    if !user_write(arg, &(AFMT_U8 | AFMT_S16_LE).to_le_bytes()) {
                        return -errno::EFAULT;
                    }
                    0
                }
                SNDCTL_DSP_GETBLKSIZE => {
                    if !user_write(arg, &4096u32.to_le_bytes()) {
                        return -errno::EFAULT;
                    }
                    0
                }
                SNDCTL_DSP_GETOSPACE => {
                    // `struct audio_buf_info` : fragments libres, total, taille,
                    // octets libres.
                    let libres = crate::drivers::ac97::libres() as u32;
                    let octets = crate::drivers::ac97::place_disponible() as u32;
                    let mut buffer = [0u8; 16];
                    buffer[0..4].copy_from_slice(&libres.to_le_bytes());
                    buffer[4..8].copy_from_slice(&32u32.to_le_bytes());
                    buffer[8..12].copy_from_slice(&4096u32.to_le_bytes());
                    buffer[12..16].copy_from_slice(&octets.to_le_bytes());
                    if !user_write(arg, &buffer) {
                        return -errno::EFAULT;
                    }
                    0
                }
                SNDCTL_DSP_GETODELAY => {
                    // Octets encore en vol : c'est ce qui permet a un lecteur de
                    // savoir de combien l'image est en avance sur le son.
                    let (frequence, voies, bits) = crate::drivers::ac97::format();
                    let (_, _, _, _, en_vol, _) = crate::drivers::ac97::resume();
                    let par_tampon = 2048 * voies as u32 * (bits as u32 / 8) * frequence
                        / crate::drivers::ac97::FREQUENCE_NATIVE;
                    if !user_write(arg, &(en_vol as u32 * par_tampon).to_le_bytes()) {
                        return -errno::EFAULT;
                    }
                    0
                }
                _ => -errno::EINVAL,
            }
        }
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
                if user_write(arg, &buffer) {
                    0
                } else {
                    -errno::EFAULT
                }
            }
            TCSETS | FIONBIO => 0,
            TIOCGWINSZ => {
                // `struct winsize` : lignes, colonnes, pixels.
                let mut buffer = [0u8; 8];
                buffer[0..2].copy_from_slice(&25u16.to_le_bytes());
                buffer[2..4].copy_from_slice(&80u16.to_le_bytes());
                if user_write(arg, &buffer) {
                    0
                } else {
                    -errno::EFAULT
                }
            }
            TIOCGPGRP => {
                if user_write(arg, &1u32.to_le_bytes()) {
                    0
                } else {
                    -errno::EFAULT
                }
            }
            FIONREAD => {
                if user_write(arg, &0u32.to_le_bytes()) {
                    0
                } else {
                    -errno::EFAULT
                }
            }
            _ => -errno::ENOTTY,
        },
        FdKind::Framebuffer => match request {
            FBIOGET_VSCREENINFO => {
                let buffer = fb_var_screeninfo();
                if user_write(arg, &buffer) {
                    0
                } else {
                    -errno::EFAULT
                }
            }
            FBIOGET_FSCREENINFO => {
                let buffer = fb_fix_screeninfo();
                if user_write(arg, &buffer) {
                    0
                } else {
                    -errno::EFAULT
                }
            }
            // La resolution est imposee par le materiel : on accepte sans rien
            // changer (Qt et SDL reessaient sinon indefiniment).
            FBIOPUT_VSCREENINFO | FBIOPAN_DISPLAY | FBIOBLANK => 0,
            _ => -errno::ENOTTY,
        },
        FdKind::VirtualTerminal => vt_ioctl(request, arg),
        FdKind::InputKeyboard => evdev_ioctl(request, arg, input::Device::Keyboard),
        FdKind::InputMouse => evdev_ioctl(request, arg, input::Device::Mouse),
        FdKind::TimerFd(_) | FdKind::EventFd(_) => match request {
            FIONBIO | FIONREAD => 0,
            _ => -errno::ENOTTY,
        },
        // `FIONBIO` bascule un descripteur en mode non bloquant. C'est par la
        // que passe `socket.settimeout()` de Python et le mode asynchrone de
        // toute pile reseau : le refuser avec ENOTTY faisait remonter un
        // « Not a tty » sur une prise parfaitement valide.
        FdKind::Socket(_) | FdKind::SocketPair(_, _) | FdKind::Pipe(_, _) => match request {
            FIONBIO => {
                let actif = match crate::kernel::abi::user_read(arg, 4) {
                    Some(octets) => octets.iter().any(|&o| o != 0),
                    None => return -errno::EFAULT,
                };
                set_nonblocking(fd, actif);
                0
            }
            FIONREAD => {
                let en_attente = pending_bytes(&kind) as u32;
                if user_write(arg, &en_attente.to_le_bytes()) {
                    0
                } else {
                    -errno::EFAULT
                }
            }
            _ => -errno::ENOTTY,
        },
        _ => -errno::ENOTTY,
    }
}

/// Pose ou retire `O_NONBLOCK` sur un descripteur.
fn set_nonblocking(fd: i32, actif: bool) {
    let process = task::current_process();
    let mut borrowed = process.borrow_mut();
    if let Some(desc) = borrowed.files.get_mut(fd) {
        if actif {
            desc.flags |= O_NONBLOCK;
        } else {
            desc.flags &= !O_NONBLOCK;
        }
    }
}

/// Octets lisibles immediatement sur un descripteur tamponne (`FIONREAD`).
fn pending_bytes(kind: &FdKind) -> usize {
    match kind {
        FdKind::Pipe(shared, true) => shared.borrow().buffer.len(),
        FdKind::SocketPair(inbox, _) => inbox.borrow().octets.len(),
        // Une prise inet rendait 0 quoi qu'il arrive, y compris avec un
        // datagramme complet en attente. `net::octets_lisibles` applique la
        // regle de Linux : le tampon de reception sur un flux, la taille du
        // prochain datagramme sur un datagramme.
        FdKind::Socket(state) => crate::kernel::abi::net::octets_lisibles(state),
        _ => 0,
    }
}

/// ioctls de console virtuelle.
///
/// L'ecran appartient deja au framebuffer : basculer en KD_GRAPHICS n'a rien a
/// changer materiellement. Mais ces appels doivent **reussir** — le plugin
/// linuxfb interprete un echec comme « pas de console utilisable » et refuse de
/// demarrer, ou reactive un curseur texte par-dessus le rendu.
fn vt_ioctl(request: u64, arg: u64) -> i64 {
    const KD_TEXT: u32 = 0;
    const KD_GRAPHICS: u32 = 1;
    match request {
        KDGETMODE => {
            let mode = if crate::drivers::gfx::is_active() {
                KD_GRAPHICS
            } else {
                KD_TEXT
            };
            if user_write(arg, &mode.to_le_bytes()) {
                0
            } else {
                -errno::EFAULT
            }
        }
        KDSETMODE => 0,
        KDGKBMODE => {
            // K_XLATE : le mode par defaut d'une console Linux.
            if user_write(arg, &1u32.to_le_bytes()) {
                0
            } else {
                -errno::EFAULT
            }
        }
        KDSKBMODE => 0,
        VT_GETMODE => {
            // `struct vt_mode` : mode, waitv, relsig, acqsig, frsig.
            let buffer = [0u8; 8];
            if user_write(arg, &buffer) {
                0
            } else {
                -errno::EFAULT
            }
        }
        VT_SETMODE | VT_ACTIVATE | VT_WAITACTIVE => 0,
        VT_GETSTATE => {
            // `struct vt_stat` : v_active, v_signal, v_state. Une seule console.
            let mut buffer = [0u8; 6];
            buffer[0..2].copy_from_slice(&1u16.to_le_bytes());
            if user_write(arg, &buffer) {
                0
            } else {
                -errno::EFAULT
            }
        }
        TCGETS | TCSETS | TIOCGWINSZ | FIONBIO => 0,
        _ => -errno::ENOTTY,
    }
}

/// `struct fb_var_screeninfo` (160 octets) decrivant le mode courant.
fn fb_var_screeninfo() -> [u8; 160] {
    let (width, height, _) = geometrie_ecran();
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
    put(32, 16);
    put(36, 8); // red offset/length
    put(44, 8);
    put(48, 8); // green
    put(56, 0);
    put(60, 8); // blue
    put(68, 24);
    put(72, 0); // transp
    put(100, 1_000_000_000 / 60); // pixclock indicatif
    buffer
}

/// `struct fb_fix_screeninfo` (80 octets) : adresse et pas de ligne.
fn fb_fix_screeninfo() -> [u8; 80] {
    let (_width, height, pas) = geometrie_ecran();
    let mut buffer = [0u8; 80];
    let id = b"bouchaudfb";
    buffer[..id.len()].copy_from_slice(id);
    // `smem_start` est l'adresse *physique* du framebuffer. Un client qui a un
    // ecran virtuel n'a rien a faire de celle du materiel : la lui donner
    // reviendrait a lui indiquer ou taper pour contourner le compositeur.
    let phys = match ecran_virtuel() {
        Some(_) => 0,
        None => crate::drivers::gfx::lfb_phys().unwrap_or(0),
    };
    buffer[16..24].copy_from_slice(&phys.to_le_bytes()); // smem_start
    buffer[24..28].copy_from_slice(&((pas * height) as u32).to_le_bytes()); // smem_len
    buffer[28..32].copy_from_slice(&0u32.to_le_bytes()); // FB_TYPE_PACKED_PIXELS
    buffer[36..40].copy_from_slice(&2u32.to_le_bytes()); // FB_VISUAL_TRUECOLOR
    buffer[48..52].copy_from_slice(&(pas as u32).to_le_bytes()); // line_length
    buffer
}

/// L'ecran virtuel du processus courant, s'il en a un.
///
/// Voir `task::Process::ecran` : c'est la redirection qui fait de `/dev/fb0` la
/// surface d'une fenetre plutot que la memoire video.
pub fn ecran_virtuel() -> Option<crate::kernel::task::EcranVirtuel> {
    let process = task::current_process();
    let ecran = process.borrow().ecran;
    ecran
}

/// Geometrie que voit le processus courant : (largeur, hauteur, pas en octets).
fn geometrie_ecran() -> (usize, usize, usize) {
    match ecran_virtuel() {
        Some(ecran) => (
            ecran.largeur as usize,
            ecran.hauteur as usize,
            ecran.pas as usize,
        ),
        None => {
            let (largeur, hauteur) = crate::drivers::gfx::resolution();
            (largeur, hauteur, largeur * 4)
        }
    }
}

/// ioctls evdev.
///
/// Les codes `EVIOC*` encodent la taille du tampon dans les bits de poids fort
/// (`_IOR(type, nr, size)`), et `EVIOCGBIT(ev, len)` encode en plus le type
/// d'evenement dans le numero. On decompose donc la requete au lieu de la
/// comparer a des constantes figees.
fn evdev_ioctl(request: u64, arg: u64, device: input::Device) -> i64 {
    let number = ((request >> 8) & 0xFF) as u8;
    let command = (request & 0xFF) as u8;
    let size = ((request >> 16) & 0x3FFF) as usize;

    if number != b'E' {
        return -errno::ENOTTY;
    }

    match command {
        // EVIOCGVERSION : version du protocole evdev (1.0.1).
        0x01 => {
            if user_write(arg, &0x0001_0001u32.to_le_bytes()) {
                0
            } else {
                -errno::EFAULT
            }
        }
        // EVIOCGID : `struct input_id`.
        0x02 => {
            let id = input::device_id(device);
            if user_write(arg, &id) {
                0
            } else {
                -errno::EFAULT
            }
        }
        // EVIOCGNAME : nom du peripherique.
        0x06 => write_string(arg, input::device_name(device), size),
        // EVIOCGPHYS : emplacement physique.
        0x07 => write_string(arg, input::device_phys(device), size),
        // EVIOCGUNIQ : identifiant unique — aucun, comme la plupart des
        // claviers PS/2. On renvoie une chaine vide, pas une erreur.
        0x08 => write_string(arg, b"\0", size),
        // EVIOCGPROP : proprietes (INPUT_PROP_*). Aucune : souris classique.
        0x09 => {
            let empty = alloc::vec![0u8; size];
            if user_write(arg, &empty) {
                size as i64
            } else {
                -errno::EFAULT
            }
        }
        // EVIOCGKEY / EVIOCGLED / EVIOCGSW : etat courant des touches, des
        // diodes, des interrupteurs. Rien d'enfonce au moment de l'ouverture.
        0x18 | 0x19 | 0x1B => {
            let empty = alloc::vec![0u8; size];
            if user_write(arg, &empty) {
                size as i64
            } else {
                -errno::EFAULT
            }
        }
        // EVIOCGBIT(type, len) : bitmap des codes supportes.
        0x20..=0x3F => {
            let kind = (command - 0x20) as u16;
            let bits = input::capability_bits(device, kind, size);
            if user_write(arg, &bits) {
                bits.len() as i64
            } else {
                -errno::EFAULT
            }
        }
        // EVIOCGABS(axe) : pas d'axe absolu (c'est une souris relative).
        0x40..=0x7F => -errno::EINVAL,
        // EVIOCSCLOCKID : choix de l'horloge des horodatages. On n'en a qu'une.
        0xA0 => 0,
        // EVIOCGRAB : acces exclusif. Il n'y a qu'un client possible ici, donc
        // la demande est toujours satisfaite.
        0x90 => 0,
        // EVIOCREVOKE, EVIOCSMASK... : sans objet, mais un echec ferait
        // renoncer certains clients.
        _ => 0,
    }
}

/// Ecrit une chaine dans un tampon utilisateur borne, renvoie sa longueur.
fn write_string(addr: u64, text: &[u8], max: usize) -> i64 {
    let len = core::cmp::min(text.len(), max.max(1));
    if user_write(addr, &text[..len]) {
        len as i64
    } else {
        -errno::EFAULT
    }
}

// --- Peripheriques d'entree --------------------------------------------------

/// Lecture non bloquante d'un peripherique d'entree.
fn read_input(buffer: u64, count: usize, device: input::Device) -> i64 {
    if count < input::EVENT_SIZE {
        return -errno::EINVAL;
    }
    let events = match device {
        input::Device::Keyboard => input::read_keyboard(count),
        input::Device::Mouse => input::read_mouse(count),
    };
    if events.is_empty() {
        return -errno::EAGAIN;
    }
    if user_write(buffer, &events) {
        events.len() as i64
    } else {
        -errno::EFAULT
    }
}

// --- Attente d'evenements ----------------------------------------------------

const POLLIN: u32 = 0x001;
const POLLERR: u32 = 0x008;
const POLLHUP: u32 = 0x010;
const POLLOUT: u32 = 0x004;

/// Ce qu'un canal repond quand on lui demande s'il accepte des octets.
pub enum Capacite {
    /// Place disponible, en octets (0 = sature).
    Place(usize),
    /// Plus personne ne lit : ecrire n'a plus de sens.
    Rompu,
}

/// Delai au-dela duquel une ecriture bloquante renonce, en millisecondes.
///
/// POSIX ferait attendre indefiniment. Ce noyau ne le peut pas honnetement :
/// une ecriture bloquee n'est reveillee par rien d'autre que l'ordonnanceur, et
/// un processus mono-thread qui remplit son propre tube s'y perdrait sans
/// qu'aucun signal ne puisse l'en sortir. On attend donc longtemps — assez pour
/// que tout lecteur normal ait eu la main plusieurs fois — puis on rend
/// `EAGAIN`. C'est une divergence assumee, et elle est visible : le programme
/// recoit une erreur au lieu de se figer.
const ATTENTE_ECRITURE_MS: u64 = 5_000;

/// Attend qu'un canal ait de la place. Rend la place obtenue, ou l'erreur.
fn attends_place<F: Fn() -> Capacite>(etat: F, non_bloquant: bool) -> Result<usize, i64> {
    let echeance =
        crate::kernel::timer::ticks() + crate::kernel::timer::ms_to_ticks(ATTENTE_ECRITURE_MS);
    loop {
        match etat() {
            // Ecrire dans un canal que plus personne ne lit n'a pas de sens :
            // Linux leve SIGPIPE et rend EPIPE. Faute de SIGPIPE ici, on rend
            // l'erreur — c'est ce que voit de toute facon un programme qui
            // ignore le signal, comme le font Python et Qt.
            Capacite::Rompu => return Err(-errno::EPIPE),
            Capacite::Place(place) if place > 0 => return Ok(place),
            Capacite::Place(_) => {}
        }
        if non_bloquant {
            return Err(-errno::EAGAIN);
        }
        if crate::kernel::timer::ticks() >= echeance {
            return Err(-errno::EAGAIN);
        }
        // Ceder la main est ce qui permet au lecteur de vider le canal : c'est
        // tout le mecanisme de contre-pression sur un noyau a un seul cœur.
        // BOUCHAUD_SMP_BLOCKING_IO_FIX_V1: une attente bloquante doit
        // marquer la tache Blocked afin que schedule() libere le BKL global
        // pendant que le producteur/consommateur progresse sur un autre CPU.
        task::attends_un_tick();
    }
}

/// Un descripteur accepte-t-il des octets ? (pour `poll`/`select`)
///
/// La reponse etait « toujours oui », ce qui privait tout protocole de sa
/// contre-pression : un producteur interrogeait `poll`, recevait un feu vert
/// permanent, et remplissait le tampon jusqu'a la memoire. Les canaux bornes
/// repondent maintenant selon leur place reelle.
fn writable(fd: i32) -> bool {
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return false,
    };
    match kind {
        // Un tube dont plus personne ne lit est « pret » : l'ecriture doit
        // echouer tout de suite en EPIPE, pas attendre une place qui ne
        // servira jamais.
        FdKind::Pipe(state, false) => {
            let state = state.borrow();
            state.readers == 0 || state.place() > 0
        }
        FdKind::Pipe(_, true) => false,
        FdKind::SocketPair(_, outbox) => {
            let canal = outbox.borrow();
            canal.lecteurs == 0 || canal.place() > 0
        }
        // Les autres n'ont pas de tampon borne : ils acceptent toujours.
        _ => true,
    }
}

/// Bits que le noyau rend sans qu'on les demande : `POLLHUP`, `POLLERR`.
///
/// La distinction n'est pas decorative. Cote **lecture**, un pair disparu est
/// une fin de fichier ordinaire : `POLLHUP` seul, et le `read` rendra 0. Cote
/// **ecriture**, c'est une erreur : le `write` rendra `EPIPE`, et `POLLERR`
/// l'annonce — sans quoi un producteur verrait `POLLOUT` (la place est libre,
/// puisque personne ne consomme) et croirait pouvoir continuer.
fn etat_pair(fd: i32) -> u32 {
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return 0,
    };
    match kind {
        FdKind::Pipe(state, true) if state.borrow().writers == 0 => POLLHUP,
        FdKind::Pipe(state, false) if state.borrow().readers == 0 => POLLHUP | POLLERR,
        FdKind::SocketPair(_, outbox) if outbox.borrow().lecteurs == 0 => POLLHUP | POLLERR,
        _ => 0,
    }
}

/// Un descripteur est-il pret en lecture ?
fn readable(fd: i32) -> bool {
    let process = task::current_process();
    let kind = match process.borrow().files.get(fd) {
        Some(desc) => desc.kind.clone(),
        None => return false,
    };
    match kind {
        FdKind::Console => keyboard::has_pending(),
        FdKind::File(_)
        | FdKind::Dir(_)
        | FdKind::Zero
        | FdKind::Random
        | FdKind::Null
        | FdKind::Instantane(_) => true,
        // Un tube dont l'ecrivain a disparu est « pret » : la lecture rendra 0.
        // Le declarer bloque ferait tourner indefiniment une boucle `poll` qui
        // attend la fin de fichier.
        FdKind::Pipe(shared, true) => {
            let state = shared.borrow();
            !state.buffer.is_empty() || state.writers == 0
        }
        // Interroger sans consommer : `poll` ne doit pas voler l'evenement au
        // `read` qui va suivre.
        FdKind::InputKeyboard => input::keyboard_pending(),
        FdKind::InputMouse => input::mouse_pending(),
        FdKind::EventFd(state) => state.borrow().counter > 0,
        FdKind::TimerFd(state) => {
            let mut state = state.borrow_mut();
            refresh_timerfd(&mut state);
            state.expirations > 0
        }
        FdKind::Socket(state) => crate::kernel::abi::net::socket_readable(&state),
        FdKind::SocketPair(inbox, _) => !inbox.borrow().octets.is_empty(),
        _ => false,
    }
}

/// `poll` / `ppoll` : `struct pollfd { int fd; short events; short revents; }`.
pub fn sys_poll(fds: u64, count: usize, timeout_ms: i32) -> i64 {
    // BOUCHAUD_CPU_OPT_POLL_BACKOFF: backoff borne pour les boucles d'I/O sans evenement.
    let mut bouchaud_idle_rounds = 0u32;
    let deadline = if timeout_ms < 0 {
        u64::MAX
    } else {
        crate::kernel::timer::ticks() + crate::kernel::timer::ms_to_ticks(timeout_ms as u64)
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
                if events & POLLOUT != 0 && writable(fd) {
                    revents |= POLLOUT;
                }
                // `POLLHUP` et `POLLERR` ne se demandent pas : le noyau les
                // rend toujours. Un ecrivain dont le lecteur a ferme doit
                // l'apprendre de son `poll`, meme s'il n'a demande que
                // `POLLOUT` — sinon il boucle sur un canal mort.
                revents |= etat_pair(fd);
            }
            if revents != 0 {
                ready += 1;
            }
            user_write(base + 6, &(revents as u16).to_le_bytes());
        }
        if ready > 0 || crate::kernel::timer::ticks() >= deadline {
            return ready;
        }
        task::attends_io_adaptatif(&mut bouchaud_idle_rounds);
    }
}

/// `select` / `pselect6`, ramene a une attente de lisibilite.
pub fn sys_select(
    nfds: i32,
    read_set: u64,
    _write_set: u64,
    _except_set: u64,
    timeout: u64,
) -> i64 {
    // BOUCHAUD_CPU_OPT_POLL_BACKOFF: backoff borne pour les boucles d'I/O sans evenement.
    let mut bouchaud_idle_rounds = 0u32;
    let deadline = if timeout == 0 {
        u64::MAX
    } else {
        let seconds = user_read_u64(timeout).unwrap_or(0);
        let micros = user_read_u64(timeout + 8).unwrap_or(0);
        let ms = seconds * 1000 + micros / 1000;
        crate::kernel::timer::ticks() + crate::kernel::timer::ms_to_ticks(ms)
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
        task::attends_io_adaptatif(&mut bouchaud_idle_rounds);
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
    // BOUCHAUD_CPU_OPT_POLL_BACKOFF: backoff borne pour les boucles d'I/O sans evenement.
    let mut bouchaud_idle_rounds = 0u32;
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
        crate::kernel::timer::ticks() + crate::kernel::timer::ms_to_ticks(timeout_ms as u64)
    };

    loop {
        let mut written = 0usize;
        for &(fd, wanted, data) in list.borrow().iter() {
            if written >= max {
                break;
            }
            let mut prets = 0u32;
            if wanted & POLLIN != 0 && readable(fd) {
                prets |= POLLIN;
            }
            if wanted & POLLOUT != 0 && writable(fd) {
                prets |= POLLOUT;
            }
            prets |= etat_pair(fd);
            if prets != 0 {
                let base = events + (written * 12) as u64;
                user_write(base, &prets.to_le_bytes());
                user_write(base + 4, &data.to_le_bytes());
                written += 1;
            }
        }
        if written > 0 || crate::kernel::timer::ticks() >= deadline {
            return written as i64;
        }
        task::attends_io_adaptatif(&mut bouchaud_idle_rounds);
    }
}

/// `eventfd2` : compteur de reveils partage entre threads.
pub fn sys_eventfd(initial: u32, flags: u32) -> i64 {
    use alloc::rc::Rc;
    use core::cell::RefCell;
    const EFD_SEMAPHORE: u32 = 1;
    const EFD_CLOEXEC: u32 = 0o2000000;
    let state = Rc::new(RefCell::new(crate::kernel::fd::EventFdState {
        counter: initial as u64,
        semaphore: flags & EFD_SEMAPHORE != 0,
    }));
    let mut desc = FileDesc::new(FdKind::EventFd(state));
    desc.cloexec = flags & EFD_CLOEXEC != 0;
    let process = task::current_process();
    let fd = process.borrow_mut().files.insert(desc);
    fd as i64
}

/// `timerfd_create`.
pub fn sys_timerfd_create(flags: u32) -> i64 {
    use alloc::rc::Rc;
    use core::cell::RefCell;
    const TFD_CLOEXEC: u32 = 0o2000000;
    let state = Rc::new(RefCell::new(crate::kernel::fd::TimerFdState {
        deadline: 0,
        interval: 0,
        expirations: 0,
    }));
    let mut desc = FileDesc::new(FdKind::TimerFd(state));
    desc.cloexec = flags & TFD_CLOEXEC != 0;
    let process = task::current_process();
    let fd = process.borrow_mut().files.insert(desc);
    fd as i64
}

/// `timerfd_settime` : `struct itimerspec` = periode puis premiere echeance.
pub fn sys_timerfd_settime(fd: i32, flags: u32, new_value: u64, old_value: u64) -> i64 {
    const TFD_TIMER_ABSTIME: u32 = 1;
    let process = task::current_process();
    let state = match process.borrow().files.get(fd) {
        Some(desc) => match &desc.kind {
            FdKind::TimerFd(state) => state.clone(),
            _ => return -errno::EINVAL,
        },
        None => return -errno::EBADF,
    };

    if old_value != 0 {
        let previous = state.borrow();
        let remaining = previous
            .deadline
            .saturating_sub(crate::kernel::timer::ticks());
        write_itimerspec(old_value, previous.interval, remaining);
    }

    let interval_ns = match read_timespec_ns(new_value) {
        Some(value) => value,
        None => return -errno::EFAULT,
    };
    let value_ns = match read_timespec_ns(new_value + 16) {
        Some(value) => value,
        None => return -errno::EFAULT,
    };

    let mut state = state.borrow_mut();
    state.interval = crate::kernel::timer::ms_to_ticks(interval_ns / 1_000_000);
    state.expirations = 0;
    state.deadline = if value_ns == 0 {
        0 // desarmement
    } else if flags & TFD_TIMER_ABSTIME != 0 {
        // Echeance absolue sur l'horloge monotone.
        let target_ms = value_ns / 1_000_000;
        let now_ms = crate::kernel::timer::monotonic_ms();
        crate::kernel::timer::ticks()
            + crate::kernel::timer::ms_to_ticks(target_ms.saturating_sub(now_ms))
    } else {
        crate::kernel::timer::ticks()
            + crate::kernel::timer::ms_to_ticks(value_ns / 1_000_000).max(1)
    };
    0
}

/// `timerfd_gettime`.
pub fn sys_timerfd_gettime(fd: i32, out: u64) -> i64 {
    let process = task::current_process();
    let state = match process.borrow().files.get(fd) {
        Some(desc) => match &desc.kind {
            FdKind::TimerFd(state) => state.clone(),
            _ => return -errno::EINVAL,
        },
        None => return -errno::EBADF,
    };
    let state = state.borrow();
    let remaining = state.deadline.saturating_sub(crate::kernel::timer::ticks());
    write_itimerspec(out, state.interval, remaining);
    0
}

/// Lit un `struct timespec` et le convertit en nanosecondes.
fn read_timespec_ns(addr: u64) -> Option<u64> {
    let seconds = crate::kernel::abi::user_read_u64(addr)?;
    let nanos = crate::kernel::abi::user_read_u64(addr + 8)?;
    Some(seconds.saturating_mul(1_000_000_000).saturating_add(nanos))
}

/// Ecrit un `struct itimerspec` a partir de durees en ticks.
fn write_itimerspec(addr: u64, interval_ticks: u64, value_ticks: u64) {
    let per_second = crate::kernel::timer::TICKS_PER_SECOND;
    for (offset, ticks) in [(0u64, interval_ticks), (16, value_ticks)] {
        let seconds = ticks / per_second;
        let nanos = (ticks % per_second) * (1_000_000_000 / per_second);
        user_write(addr + offset, &seconds.to_le_bytes());
        user_write(addr + offset + 8, &nanos.to_le_bytes());
    }
}

// --- Statistiques de systeme de fichiers -------------------------------------

/// `statfs` / `fstatfs` : `struct statfs` de 120 octets (x86_64).
///
/// ## Pourquoi ce n'est pas un bouchon
///
/// Ladybird interroge l'espace disque par `statvfs`, et la glibc sert
/// `statvfs`/`fstatvfs` a partir de `statfs`/`fstatfs`. Le seul appelant du
/// portage est `LibHTTP/Cache/CacheIndex.cpp`, qui dimensionne le cache HTTP
/// sur l'espace **libre** :
///
///     auto disk_space = TRY(FileSystem::compute_disk_space(cache_directory));
///     auto maximum = compute_maximum_disk_cache_size(disk_space.free_bytes);
///
/// Rendre `0` ferait donc un cache de taille nulle, et rendre `ENOSYS` fait
/// echouer l'initialisation du cache. Les deux sont des reponses fausses, pas
/// une absence de reponse.
///
/// Sur Bouchaud le systeme de fichiers est en memoire vive : son espace libre
/// **est** la memoire libre. C'est ce que rend cette implementation, avec le
/// nombre reel de frames disponibles et le nombre reel de nœuds RAMFS encore
/// alloues. Aucune de ces valeurs n'est inventee.
fn statfs_bytes() -> [u8; 120] {
    const BLOC: u64 = 4096;
    let (_, frames_libres, frames_totales) = crate::kernel::vmm::frame_stats();
    let fs = crate::fs::ramfs::fs();
    let nœuds_utilises = fs.used_nodes() as u64;
    let nœuds_totaux = crate::fs::ramfs::MAX_NODES as u64;

    let mut buffer = [0u8; 120];
    // f_type : `RAMFS_MAGIC`, la valeur que Linux rend pour un tmpfs/ramfs.
    // Certains programmes s'en servent pour savoir qu'un chemin est volatil.
    buffer[0..8].copy_from_slice(&0x8584_58f6u64.to_le_bytes()); // f_type
    buffer[8..16].copy_from_slice(&BLOC.to_le_bytes()); // f_bsize
    buffer[16..24].copy_from_slice(&(frames_totales as u64).to_le_bytes()); // f_blocks
    buffer[24..32].copy_from_slice(&(frames_libres as u64).to_le_bytes()); // f_bfree
    buffer[32..40].copy_from_slice(&(frames_libres as u64).to_le_bytes()); // f_bavail
    buffer[40..48].copy_from_slice(&nœuds_totaux.to_le_bytes()); // f_files
    buffer[48..56].copy_from_slice(&(nœuds_totaux - nœuds_utilises).to_le_bytes()); // f_ffree
    // `f_fsid` ne fait que **huit** octets (deux entiers 32 bits), pas seize :
    // les champs suivants commencent donc a 64 et 72, et non a 72 et 80.
    // Decales, ils laissaient `f_namelen` a zero et faisaient passer 255 pour
    // `f_frsize` — or `compute_disk_space` calcule `f_bavail * f_frsize`, donc
    // l'espace libre etait annonce seize fois trop petit.
    // f_fsid (56..64) laisse a zero : un seul systeme de fichiers.
    buffer[64..72].copy_from_slice(&255u64.to_le_bytes()); // f_namelen
    buffer[72..80].copy_from_slice(&BLOC.to_le_bytes()); // f_frsize
    // f_flags (80..88) laisse a zero : aucun ST_RDONLY, ST_NOSUID ni ST_NOEXEC
    // n'est vrai ici, et 4096 s'y retrouvait par le meme decalage.
    buffer
}

/// `statfs(chemin, buf)`.
pub fn sys_statfs(path_addr: u64, out: u64) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    // Le chemin doit exister : c'est la seule difference observable avec
    // `fstatfs`, et un appelant qui teste un repertoire de cache absent doit
    // recevoir `ENOENT` plutot qu'une taille.
    if resolve(&path).is_none() && device_for_path(&path).is_none() {
        return -errno::ENOENT;
    }
    if user_write(out, &statfs_bytes()) { 0 } else { -errno::EFAULT }
}

/// `fstatfs(fd, buf)`.
pub fn sys_fstatfs(fd: i32, out: u64) -> i64 {
    let process = task::current_process();
    if process.borrow().files.get(fd).is_none() {
        return -errno::EBADF;
    }
    if user_write(out, &statfs_bytes()) { 0 } else { -errno::EFAULT }
}
