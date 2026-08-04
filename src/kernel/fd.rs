//! Descripteurs de fichiers d'un processus.
//!
//! Un descripteur pointe soit sur un nœud RAMFS, soit sur un peripherique
//! synthetique. Les peripheriques sont ceux dont une pile graphique attend
//! l'existence :
//!
//! - `/dev/fb0` : framebuffer, `mmap`-able et interrogeable par les ioctls
//!   `FBIOGET_*SCREENINFO` (plugin `linuxfb` de Qt, `fbdev` de SDL) ;
//! - `/dev/input/event0` et `event1` : evenements clavier et souris au format
//!   `struct input_event` d'evdev ;
//! - `/dev/null`, `/dev/zero`, `/dev/urandom`, `/dev/tty`.
//!
//! Les tubes (`pipe2`) sont necessaires des qu'une bibliotheque veut se
//! reveiller elle-meme depuis un autre thread : c'est exactement ce que fait la
//! boucle d'evenements de Qt (le « wakeup pipe »).

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

/// Nature d'un descripteur ouvert.
#[derive(Clone)]
pub enum FdKind {
    /// Console : lecture clavier, ecriture ecran/serie.
    Console,
    /// Fichier RAMFS (index de nœud).
    File(usize),
    /// Repertoire RAMFS parcouru par `getdents64`.
    Dir(usize),
    /// `/dev/null`.
    Null,
    /// `/dev/zero`.
    Zero,
    /// `/dev/urandom`, `/dev/random`.
    Random,
    /// `/dev/fb0`.
    Framebuffer,
    /// `/dev/input/event0` (clavier).
    InputKeyboard,
    /// `/dev/input/event1` (souris).
    InputMouse,
    /// Extremite d'un tube : (tampon partage, cote lecture ?).
    Pipe(Rc<RefCell<Vec<u8>>>, bool),
    /// Instance `epoll` : liste de (fd surveille, evenements demandes, donnee).
    Epoll(Rc<RefCell<Vec<(i32, u32, u64)>>>),
}

/// Un descripteur ouvert.
#[derive(Clone)]
pub struct FileDesc {
    pub kind: FdKind,
    /// Position de lecture/ecriture.
    pub offset: usize,
    /// Drapeaux d'ouverture (`O_*`).
    pub flags: u32,
    /// Descripteur ferme automatiquement par `execve` (FD_CLOEXEC).
    pub cloexec: bool,
}

impl FileDesc {
    pub fn new(kind: FdKind) -> Self {
        FileDesc { kind, offset: 0, flags: 0, cloexec: false }
    }
}

/// Table de descripteurs d'un processus.
pub struct FdTable {
    entries: Vec<Option<FileDesc>>,
}

impl FdTable {
    /// Table neuve avec 0/1/2 relies a la console, comme apres un `fork` d'init.
    pub fn new() -> Self {
        let mut entries = Vec::new();
        for _ in 0..3 {
            entries.push(Some(FileDesc::new(FdKind::Console)));
        }
        FdTable { entries }
    }

    /// Installe un descripteur au premier emplacement libre >= `min`.
    pub fn insert_at_least(&mut self, desc: FileDesc, min: usize) -> i32 {
        while self.entries.len() < min {
            self.entries.push(None);
        }
        for (index, slot) in self.entries.iter_mut().enumerate().skip(min) {
            if slot.is_none() {
                *slot = Some(desc);
                return index as i32;
            }
        }
        self.entries.push(Some(desc));
        (self.entries.len() - 1) as i32
    }

    /// Installe un descripteur au premier emplacement libre.
    pub fn insert(&mut self, desc: FileDesc) -> i32 {
        self.insert_at_least(desc, 0)
    }

    /// Force un descripteur a un numero precis (`dup2`).
    pub fn set(&mut self, fd: usize, desc: FileDesc) {
        while self.entries.len() <= fd {
            self.entries.push(None);
        }
        self.entries[fd] = Some(desc);
    }

    /// Descripteur en lecture seule.
    pub fn get(&self, fd: i32) -> Option<&FileDesc> {
        if fd < 0 {
            return None;
        }
        self.entries.get(fd as usize).and_then(|slot| slot.as_ref())
    }

    /// Descripteur modifiable (avancee de l'offset, drapeaux).
    pub fn get_mut(&mut self, fd: i32) -> Option<&mut FileDesc> {
        if fd < 0 {
            return None;
        }
        self.entries.get_mut(fd as usize).and_then(|slot| slot.as_mut())
    }

    /// Ferme un descripteur. `false` s'il n'etait pas ouvert.
    pub fn close(&mut self, fd: i32) -> bool {
        if fd < 0 {
            return false;
        }
        match self.entries.get_mut(fd as usize) {
            Some(slot) if slot.is_some() => {
                *slot = None;
                true
            }
            _ => false,
        }
    }

    /// Nombre de descripteurs ouverts.
    pub fn open_count(&self) -> usize {
        self.entries.iter().filter(|slot| slot.is_some()).count()
    }
}

/// Resout un chemin de peripherique en descripteur, ou `None` si ce n'en est
/// pas un (le chemin sera alors cherche dans le RAMFS).
pub fn device_for_path(path: &str) -> Option<FdKind> {
    match path {
        "/dev/null" => Some(FdKind::Null),
        "/dev/zero" => Some(FdKind::Zero),
        "/dev/random" | "/dev/urandom" => Some(FdKind::Random),
        "/dev/tty" | "/dev/console" | "/dev/stdin" | "/dev/stdout" | "/dev/stderr" => Some(FdKind::Console),
        "/dev/fb0" | "/dev/fb" | "/dev/graphics/fb0" => Some(FdKind::Framebuffer),
        "/dev/input/event0" => Some(FdKind::InputKeyboard),
        "/dev/input/event1" => Some(FdKind::InputMouse),
        _ => None,
    }
}
