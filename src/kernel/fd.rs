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
    /// `/dev/dsp` : sortie audio PCM, facon OSS.
    Audio,
    /// Extremite d'un tube : (etat partage, cote lecture ?).
    Pipe(Rc<RefCell<PipeState>>, bool),
    /// Instance `epoll` : liste de (fd surveille, evenements demandes, donnee).
    Epoll(Rc<RefCell<Vec<(i32, u32, u64)>>>),
    /// `eventfd` : compteur 64 bits partage entre threads.
    EventFd(Rc<RefCell<EventFdState>>),
    /// `timerfd` : echeance et periode, en ticks du timer noyau.
    TimerFd(Rc<RefCell<TimerFdState>>),
    /// Socket reseau (TCP ou UDP).
    Socket(Rc<RefCell<crate::kernel::abi::net::SocketState>>),
    /// Extremite de `socketpair` : (tampon de lecture, tampon d'ecriture).
    ///
    /// Deux tampons et non un : un socket est bidirectionnel, chaque extremite
    /// lit dans celui que l'autre alimente.
    SocketPair(Rc<RefCell<Canal>>, Rc<RefCell<Canal>>),
    /// Console virtuelle `/dev/tty0` : ne sert qu'a ses ioctls de mode
    /// graphique, les entrees/sorties passent par la console.
    VirtualTerminal,
}

/// Etat partage d'un tube.
///
/// Le tampon ne suffit pas : il faut savoir **combien d'extremites existent
/// encore**. Un tube vide dont il reste un ecrivain doit faire attendre le
/// lecteur ; le meme tube vide sans plus aucun ecrivain doit rendre 0, la fin
/// de fichier. Sans ce comptage, les deux situations sont indiscernables et il
/// faut choisir la mauvaise reponse dans un cas sur deux.
pub struct PipeState {
    pub buffer: Vec<u8>,
    /// Descripteurs ouverts sur le cote lecture.
    pub readers: usize,
    /// Descripteurs ouverts sur le cote ecriture.
    pub writers: usize,
}

impl PipeState {
    fn new() -> Self {
        // Un `pipe()` cree exactement une extremite de chaque cote.
        PipeState { buffer: Vec::new(), readers: 1, writers: 1 }
    }

    /// Place restante avant saturation.
    pub fn place(&self) -> usize {
        CAPACITE_CANAL.saturating_sub(self.buffer.len())
    }
}

/// Cree l'etat partage d'un nouveau tube.
pub fn new_pipe() -> Rc<RefCell<PipeState>> {
    Rc::new(RefCell::new(PipeState::new()))
}

/// Etat d'un `eventfd`.
///
/// Ce n'est pas un tube : `read` renvoie la valeur du compteur **et le remet a
/// zero** (ou le decremente de 1 en mode semaphore), `write` ajoute au
/// compteur. Une boucle d'evenements s'en sert pour se reveiller elle-meme
/// depuis un autre thread ; la traiter comme un flux d'octets ferait accumuler
/// des reveils au lieu de les fusionner.
pub struct EventFdState {
    pub counter: u64,
    pub semaphore: bool,
}

/// Etat d'un `timerfd`.
pub struct TimerFdState {
    /// Tick de la prochaine echeance (0 = desarme).
    pub deadline: u64,
    /// Periode en ticks (0 = one-shot).
    pub interval: u64,
    /// Expirations non encore lues.
    pub expirations: u64,
}

impl FdKind {
    /// Une extremite de plus existe sur cet objet.
    fn retain(&self) {
        match self {
            FdKind::Pipe(state, readable) => {
                let mut state = state.borrow_mut();
                if *readable { state.readers += 1 } else { state.writers += 1 }
            }
            // Le nœud d'un fichier compte ses descripteurs : c'est la moitie du
            // critere qui autorise a evincer ses pages partagees.
            FdKind::File(node) => crate::kernel::partage::ouvre(*node),
            // Chaque extremite lit dans sa boite aux lettres. Compter les
            // lecteurs de ce canal-la, c'est savoir si l'ecrivain d'en face
            // parle encore a quelqu'un.
            FdKind::SocketPair(inbox, _) => inbox.borrow_mut().lecteurs += 1,
            _ => {}
        }
    }

    /// Une extremite de moins.
    fn release(&self) {
        match self {
            FdKind::Pipe(state, readable) => {
                let mut state = state.borrow_mut();
                let compteur = if *readable { &mut state.readers } else { &mut state.writers };
                *compteur = compteur.saturating_sub(1);
            }
            FdKind::File(node) => crate::kernel::partage::ferme(*node),
            FdKind::SocketPair(inbox, _) => {
                let mut canal = inbox.borrow_mut();
                canal.lecteurs = canal.lecteurs.saturating_sub(1);
            }
            _ => {}
        }
    }
}

/// Un descripteur ouvert.
///
/// ## Descripteur contre objet
///
/// `FileDesc` est **un descripteur** : le dupliquer (`dup`, `fork`) cree une
/// extremite de plus, le detruire en retire une. C'est pourquoi `Clone` et
/// `Drop` sont ecrits a la main plutot que derives — ce sont les deux seuls
/// endroits ou le comptage doit se faire, et les ecrire ici les rend
/// impossibles a oublier, quel que soit le chemin qui duplique la table.
///
/// [`FdKind`] est **l'objet** designe. Le cloner ne cree pas d'extremite : les
/// appels systeme en prennent une copie a chaque passage, pour ne pas garder un
/// emprunt de la table pendant leur travail. Ces copies-la ne comptent pas.
/// Un sens d'une paire de sockets : les octets en transit, et les descripteurs.
///
/// Un socket local ne transporte pas que des donnees : `SCM_RIGHTS` lui fait
/// aussi passer des **descripteurs de fichier** d'un processus a l'autre. C'est
/// ce dont vit l'architecture multi-processus d'un moteur web — celui qui met
/// en page ouvre un tampon partage et en confie le descripteur a celui qui
/// compose, sans jamais recopier les pixels.
///
/// Les descripteurs voyagent dans une file separee des octets. La norme les lie
/// au message qui les accompagne ; les separer rendrait possible qu'un
/// `recvmsg` recoive les descripteurs d'un envoi et les octets d'un autre. En
/// pratique les deux bouts s'echangent un message a la fois, et l'ecart ne
/// s'observe pas.
#[derive(Default)]
pub struct Canal {
    pub octets: Vec<u8>,
    pub descripteurs: Vec<FileDesc>,
    /// Descripteurs qui lisent dans ce canal.
    ///
    /// Un canal est unidirectionnel : une extremite y ecrit, l'autre y lit. Ce
    /// compteur repond a la seule question qui compte pour l'ecrivain — « ce
    /// que j'ecris sera-t-il lu par quelqu'un ? ». A zero, l'ecriture rend
    /// `EPIPE` et `poll` leve `POLLHUP` au lieu de faire attendre pour rien.
    pub lecteurs: usize,
}

/// Capacite d'un canal ou d'un tube, en octets.
///
/// Sans plafond, un ecrivain plus rapide que son lecteur fait grossir le tampon
/// jusqu'a epuiser la memoire — et `poll` ne peut jamais dire « attends ». La
/// valeur est celle de Linux, et elle n'est pas arbitraire : trop basse, elle
/// multiplie les reveils ; trop haute, elle laisse la contre-pression arriver
/// bien apres que le producteur ait pris trop d'avance.
pub const CAPACITE_CANAL: usize = 64 * 1024;

impl Canal {
    /// Un canal neuf a exactement un lecteur : l'extremite d'en face.
    pub fn neuf() -> Rc<RefCell<Canal>> {
        Rc::new(RefCell::new(Canal { lecteurs: 1, ..Canal::default() }))
    }

    /// Place restante avant saturation.
    pub fn place(&self) -> usize {
        CAPACITE_CANAL.saturating_sub(self.octets.len())
    }
}

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
        // Les objets qui comptent leurs extremites le font des leur creation :
        // `PipeState::new` part a 1/1, `Canal::neuf` a un lecteur. Un nœud de
        // fichier n'a pas de constructeur a lui — c'est donc ici qu'il prend sa
        // premiere reference, et `clone`/`drop` s'occupent des suivantes.
        if let FdKind::File(node) = kind {
            crate::kernel::partage::ouvre(node);
        }
        FileDesc { kind, offset: 0, flags: 0, cloexec: false }
    }
}

impl Clone for FileDesc {
    fn clone(&self) -> Self {
        self.kind.retain();
        FileDesc {
            kind: self.kind.clone(),
            offset: self.offset,
            flags: self.flags,
            cloexec: self.cloexec,
        }
    }
}

impl Drop for FileDesc {
    fn drop(&mut self) {
        self.kind.release();
    }
}

/// Table de descripteurs d'un processus.
///
/// `Clone` est ce dont `fork` a besoin : les descripteurs sont dupliques mais
/// les objets sous-jacents (tubes, sockets, `eventfd`) restent partages via
/// leur `Rc`. C'est le comportement POSIX, et ce sur quoi repose le chainage
/// `cmd1 | cmd2`.
#[derive(Clone)]
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

    /// Ferme les descripteurs marques `FD_CLOEXEC` (appele par `execve`).
    pub fn close_on_exec(&mut self) {
        for slot in self.entries.iter_mut() {
            if slot.as_ref().map_or(false, |desc| desc.cloexec) {
                *slot = None;
            }
        }
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
        // Les consoles virtuelles sont ouvertes par le plugin linuxfb de Qt
        // pour basculer le terminal en mode graphique.
        "/dev/tty0" | "/dev/tty1" | "/dev/tty2" | "/dev/vc/0" => Some(FdKind::VirtualTerminal),
        "/dev/fb0" | "/dev/fb" | "/dev/graphics/fb0" => Some(FdKind::Framebuffer),
        "/dev/input/event0" => Some(FdKind::InputKeyboard),
        "/dev/input/event1" => Some(FdKind::InputMouse),
        // Sortie audio. `/dev/dsp` est le nom OSS, `/dev/audio` son alias
        // historique : les deux menent au meme pilote AC'97.
        "/dev/dsp" | "/dev/dsp0" | "/dev/audio" | "/dev/sound/dsp" => Some(FdKind::Audio),
        _ => None,
    }
}
