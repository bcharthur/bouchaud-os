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

use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::kernel::sync::SpinLock;
use crate::kernel::sync::{WaitQueue, WaitTicket};
use core::ops::{Deref, DerefMut};

/// Mode d'acces d'un descripteur ouvert en lecture et en ecriture.
///
/// Ce nombre appartient au contrat de l'ABI utilisateur de Bouchaud. Le
/// conserver ici evite que les producteurs de memoire partagee fabriquent un
/// fichier inscriptible tout en publiant, par oubli, un descripteur en lecture
/// seule.
const ACCES_LECTURE_ECRITURE: u32 = 2;

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
    Pipe(Arc<SpinLock<PipeState>>, bool),
    /// Instance `epoll` : liste de (fd surveille, evenements demandes, donnee).
    Epoll(Arc<SpinLock<Vec<(i32, u32, u64)>>>),
    /// `eventfd` : compteur 64 bits partage entre threads.
    EventFd(Arc<SpinLock<EventFdState>>),
    /// `timerfd` : echeance et periode, en ticks du timer noyau.
    TimerFd(Arc<SpinLock<TimerFdState>>),
    /// Socket reseau (TCP ou UDP).
    Socket(Arc<SpinLock<crate::kernel::abi::net::SocketState>>),
    /// Extremite de `socketpair` : (tampon de lecture, tampon d'ecriture).
    ///
    /// Deux tampons et non un : un socket est bidirectionnel, chaque extremite
    /// lit dans celui que l'autre alimente.
    SocketPair(Arc<SpinLock<Canal>>, Arc<SpinLock<Canal>>),
    /// Console virtuelle `/dev/tty0` : ne sert qu'a ses ioctls de mode
    /// graphique, les entrees/sorties passent par la console.
    VirtualTerminal,
    /// Instantane en memoire, produit a l'ouverture et lu comme un fichier.
    ///
    /// C'est ce qui permet a `/proc/self/…` de decrire le processus **courant**
    /// sans que le RAMFS n'ait a porter une copie par processus. Un fichier
    /// ecrit dans `/proc/self/` serait partage par tout le systeme : le
    /// gestionnaire de fenetres et le navigateur y liraient la meme chose,
    /// c'est-a-dire la mauvaise pour l'un des deux.
    ///
    /// Le contenu est fige a l'ouverture. C'est la meme convention que les
    /// autres fichiers de `/proc` sur Bouchaud (voir `sysroot.rs` : « ce sont
    /// des instantanes, pas des vues vivantes »), et elle suffit aux
    /// consommateurs reels, qui ouvrent, lisent d'un trait et referment.
    Instantane(Arc<Vec<u8>>),
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
pub fn new_pipe() -> Arc<SpinLock<PipeState>> {
    Arc::new(SpinLock::new(PipeState::new()))
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
                let mut state = state.lock();
                if *readable { state.readers += 1 } else { state.writers += 1 }
            }
            // Le nœud d'un fichier compte ses descripteurs : c'est la moitie du
            // critere qui autorise a evincer ses pages partagees.
            FdKind::File(node) => crate::kernel::partage::ouvre(*node),
            // Une extremite de socketpair lit son inbox et ecrit son outbox.
            // `dup`, `fork` et `SCM_RIGHTS` dupliquent donc ces deux capacites :
            // les compter ici garantit que l'EOF n'arrive qu'apres la fermeture
            // du dernier descripteur capable d'ecrire dans le canal.
            FdKind::SocketPair(inbox, outbox) => {
                inbox.lock().lecteurs += 1;
                let mut canal = outbox.lock();
                canal.ecrivains += 1;
                canal.octets.rouvre();
            }
            _ => {}
        }
    }

    /// Une extremite de moins.
    fn release(&self) {
        match self {
            FdKind::Pipe(state, readable) => {
                let mut state = state.lock();
                let compteur = if *readable { &mut state.readers } else { &mut state.writers };
                *compteur = compteur.saturating_sub(1);
            }
            FdKind::File(node) => crate::kernel::partage::ferme(*node),
            FdKind::SocketPair(inbox, outbox) => {
                {
                    let mut canal = inbox.lock();
                    canal.lecteurs = canal.lecteurs.saturating_sub(1);
                }
                let mut canal = outbox.lock();
                canal.ecrivains = canal.ecrivains.saturating_sub(1);
                if canal.ecrivains == 0 {
                    canal.octets.marque_eof();
                }
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
///
/// `Vec::is_empty()` n'est pas suffisant pour un flux : un canal sans octet
/// mais dont le dernier ecrivain vient de fermer est **lisible**, parce qu'un
/// `read` doit rendre 0 immediatement. `OctetsCanal` conserve l'API de `Vec`
/// pour les producteurs/consommateurs existants, tout en faisant de
/// `is_empty()` la question utile a leurs boucles d'attente : « cette lecture
/// devrait-elle encore bloquer ? ».
#[derive(Default)]
pub struct OctetsCanal {
    buffer: Vec<u8>,
    eof: bool,
}

impl OctetsCanal {
    /// `true` uniquement lorsqu'il n'y a ni donnees ni EOF a consommer.
    ///
    /// A EOF, `len()` reste 0 ; les chemins de lecture existants sortent donc
    /// de leur attente puis rendent naturellement 0 sans octet sentinelle.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty() && !self.eof
    }

    fn marque_eof(&mut self) {
        self.eof = true;
    }

    fn rouvre(&mut self) {
        self.eof = false;
    }
}

impl Deref for OctetsCanal {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl DerefMut for OctetsCanal {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffer
    }
}

#[derive(Default)]
pub struct Canal {
    pub octets: OctetsCanal,
    pub descripteurs: Vec<FileDesc>,
    /// Descripteurs qui lisent dans ce canal.
    ///
    /// Un canal est unidirectionnel : une extremite y ecrit, l'autre y lit. Ce
    /// compteur repond a la question de l'ecrivain — « ce que j'ecris sera-t-il
    /// lu par quelqu'un ? ». A zero, l'ecriture rend `EPIPE`.
    pub lecteurs: usize,
    /// Descripteurs qui peuvent encore ecrire dans ce canal.
    ///
    /// Le lecteur en a besoin pour distinguer un canal momentanement vide d'un
    /// flux termine. Ce compteur suit exactement les `FileDesc`, y compris les
    /// duplications par `dup`/`fork` et les transferts `SCM_RIGHTS`.
    pub ecrivains: usize,
    // BOUCHAUD_GUI_EVENT_DRIVEN_V1
    /// Une ecriture dans ce canal doit reveiller le compositeur.
    ///
    /// Pose uniquement sur le canal client -> gestionnaire de fenetres. Sans ce
    /// drapeau il faudrait signaler a CHAQUE ecriture de descripteur -- tubes,
    /// sockets reseau, journaux -- et le compositeur se reveillerait pour du
    /// trafic qui ne le concerne pas. Un reveil inutile coute une composition
    /// complete ; le drapeau coute un booleen.
    pub reveille_compositeur: bool,
}

/// Capacite d'un canal ou d'un tube, en octets.
///
/// Sans plafond, un ecrivain plus rapide que son lecteur fait grossir le tampon
/// jusqu'a epuiser la memoire — et `poll` ne peut jamais dire « attends ». La
/// valeur est celle de Linux, et elle n'est pas arbitraire : trop basse, elle
/// multiplie les reveils ; trop haute, elle laisse la contre-pression arriver
/// bien apres que le producteur ait pris trop d'avance.
pub const CAPACITE_CANAL: usize = 64 * 1024;

/// Taille jusqu'a laquelle une ecriture dans un tube ou un canal est ATOMIQUE
/// (POSIX `PIPE_BUF`, valeur de Linux) : ses octets ne sont jamais coupes ni
/// melanges a ceux d'un autre ecrivain. BOUCHAUD_TUBE_ATOMIQUE_V1.
pub const PIPE_BUF: usize = 4096;

/// Ecrit `data` dans un tampon de canal de `place` octets libres, avec la
/// regle d'atomicite : jusqu'a [`PIPE_BUF`] octets, tout ou rien (`0` = rien
/// d'ecrit, attendre) ; au-dela, autant que la place le permet. Pure : rend
/// le nombre d'octets a ecrire.
pub fn octets_admis(place: usize, longueur: usize) -> usize {
    if longueur <= PIPE_BUF {
        if place >= longueur { longueur } else { 0 }
    } else {
        place.min(longueur)
    }
}

/// Queue de changement de readiness des objets FD.
///
/// La generation ferme le lost-wakeup entre le scan final de `poll` et le
/// parking. Une queue globale peut produire des reveils parasites entre objets,
/// mais elle supprime le polling periodique et constitue un contrat extensible:
/// tout producteur appelle `notify_readiness` apres avoir publie son etat.
static READINESS: WaitQueue = WaitQueue::new();

/// Cle de la file de disponibilite : un fil dont `wait_queue_key` vaut
/// cette cle dort dans `poll`/`select`/`epoll_wait` (meme calcul que
/// `WaitQueue::key`). BOUCHAUD_SIGNAL_FIL_PRINCIPAL_V1.
pub fn readiness_cle() -> usize {
    &READINESS as *const WaitQueue as usize
}

pub fn readiness_ticket() -> WaitTicket {
    READINESS.ticket()
}

pub fn wait_readiness(ticket: WaitTicket, deadline_ns: Option<u64>) {
    match deadline_ns {
        Some(deadline) => { READINESS.wait_until(ticket, deadline); }
        None => READINESS.wait(ticket),
    }
}

pub fn notify_readiness() {
    READINESS.wake_all();
}

/// Ferme des descripteurs retires d'une table, puis reveille les attentes de
/// disponibilite (BOUCHAUD_EOF_HORS_CLOSE_V1).
///
/// `FdKind::release` decremente les extremites d'un tube ou d'un canal sans
/// reveiller personne : un lecteur bloque sur un tube vide n'apprend la fin de
/// fichier (ni un ecrivain bloque EPIPE) que par `notify_readiness`. `close()`
/// l'appelait ; `execve` (FD_CLOEXEC, le motif de `posix_spawn`), `dup2` /
/// `dup3` sur un numero ouvert et la mort d'un orphelin (aucun SIGCHLD) ne le
/// faisaient pas. Le lecteur attendait la mort de l'ecrivain ou un evenement
/// sans rapport -- eof-fermeture-probe, TCG SMP4 : 12 cas sur 18, fin de
/// fichier a 3,2 s (mort du fils) ou 5 s (alarme) au lieu de 0,15 s.
///
/// Jamais sous le verrou de la table (rang `FdTable`) : le reveil prend la
/// table des taches, de rang inferieur.
pub fn ferme_hors_verrou(fermes: impl IntoIterator<Item = FileDesc>) {
    let mut aucun = true;
    for desc in fermes {
        aucun = false;
        drop(desc);
    }
    if !aucun {
        notify_readiness();
    }
}

impl Canal {
    /// Un canal neuf a exactement un lecteur et un ecrivain : les deux
    /// extremites creees ensemble par `socketpair()`.
    pub fn neuf() -> Arc<SpinLock<Canal>> {
        Arc::new(SpinLock::new(Canal {
            lecteurs: 1,
            ecrivains: 1,
            ..Canal::default()
        }))
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
        // `PipeState::new` part a 1/1, `Canal::neuf` a un lecteur et un
        // ecrivain. Un nœud de fichier n'a pas de constructeur a lui — c'est
        // donc ici qu'il prend sa premiere reference, et `clone`/`drop`
        // s'occupent des suivantes.
        if let FdKind::File(node) = kind {
            crate::kernel::partage::ouvre(node);
        }
        FileDesc { kind, offset: 0, flags: 0, cloexec: false }
    }

    /// Descripteur d'un fichier destine a une projection partagee inscriptible.
    ///
    /// Le droit du nœud et le mode porte par le descripteur sont deux choses
    /// distinctes. Une projection partagee en ecriture exige les deux ; ce
    /// constructeur rend impossible l'oubli du second sur les surfaces et les
    /// tampons anonymes crees par Bouchaud.
    pub fn fichier_partage_inscriptible(node: usize) -> Self {
        let mut desc = Self::new(FdKind::File(node));
        desc.flags = ACCES_LECTURE_ECRITURE;
        desc
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
/// leur `Arc`. C'est le comportement POSIX, et ce sur quoi repose le chainage
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

    /// Force un descripteur a un numero precis (`dup2`) et rend celui qu'il
    /// remplace, a fermer par [`ferme_hors_verrou`].
    #[must_use = "l'ancien descripteur se ferme hors du verrou de la table (ferme_hors_verrou)"]
    pub fn set(&mut self, fd: usize, desc: FileDesc) -> Option<FileDesc> {
        while self.entries.len() <= fd {
            self.entries.push(None);
        }
        self.entries[fd].replace(desc)
    }

    /// Descripteur en lecture seule.
    pub fn get(&self, fd: i32) -> Option<&FileDesc> {
        if fd < 0 {
            return None;
        }
        self.entries.get(fd as usize).and_then(|slot| slot.as_ref())
    }

    /// Descripteurs ouverts, dans l'ordre des numeros.
    ///
    /// Necessaire aux sous-systemes qui doivent repartir une ressource
    /// **partagee** entre plusieurs descripteurs : une seule carte reseau
    /// alimente tous les sockets UDP du processus, et le datagramme sorti de
    /// l'anneau pour l'un peut appartenir a l'autre.
    pub fn iter(&self) -> impl Iterator<Item = &FileDesc> {
        self.entries.iter().filter_map(|slot| slot.as_ref())
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

    /// Retire TOUS les descripteurs, pour qu'ils soient fermes par
    /// l'appelant -- hors du verrou de la table ([`ferme_hors_verrou`]) : la
    /// fermeture d'un tube reveille ses lecteurs, et ne doit rien faire sous
    /// ce verrou.
    ///
    /// BOUCHAUD_DESCRIPTEURS_A_LA_MORT_V1 : appele a la mort du processus.
    pub fn prend_tout(&mut self) -> Vec<Option<FileDesc>> {
        core::mem::take(&mut self.entries)
    }

    /// Retire les descripteurs marques `FD_CLOEXEC` (appele par `execve`),
    /// a fermer par [`ferme_hors_verrou`].
    #[must_use = "les descripteurs retires se ferment hors du verrou de la table (ferme_hors_verrou)"]
    pub fn close_on_exec(&mut self) -> Vec<FileDesc> {
        let mut retires = Vec::new();
        for slot in self.entries.iter_mut() {
            if slot.as_ref().map_or(false, |desc| desc.cloexec) {
                retires.extend(slot.take());
            }
        }
        retires
    }
}

/// Resout un chemin de peripherique en descripteur, ou `None` si ce n'en est
/// pas un (le chemin sera alors cherche dans le RAMFS).
pub fn device_for_path(path: &str) -> Option<FdKind> {
    // BOUCHAUD_V13_PROC_STATS_DYNAMIQUES_OPEN
    // Ladybird epingle lit ces deux interfaces pour son ProcessManager.
    // Un fichier RAMFS ecrit au boot mentirait des la milliseconde suivante :
    // on fige donc un instantane AU MOMENT DE open(2), comme /proc/self/maps.
    if path == "/proc/stat" {
        return Some(FdKind::Instantane(Arc::new(proc_stat_global())));
    }
    if path == "/proc/self/stat" {
        let pid = crate::kernel::task::current_process().pid;
        return proc_stat_processus(pid).map(|v| FdKind::Instantane(Arc::new(v)));
    }
    // BOUCHAUD_ATTRIBUTION_REVEIL_V1 : le pire reveil de sommeil du FIL
    // appelant depuis la lecture precedente, decompose, et le pire ecart
    // entre deux tics du PIT. L'ouverture RELEVE et REMET A ZERO.
    if path == "/proc/self/reveil" {
        return Some(FdKind::Instantane(Arc::new(proc_reveil())));
    }
    // BOUCHAUD_PREUVE_HOTE_V1 : les comptes CUMULES du fil, relus apres
    // chaque reveil par la sonde (jamais remis a zero).
    if path == "/proc/self/fil" {
        return Some(FdKind::Instantane(Arc::new(proc_fil())));
    }
    if let Some(pid) = proc_pid_stat_depuis_chemin(path) {
        return proc_stat_processus(pid).map(|v| FdKind::Instantane(Arc::new(v)));
    }
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
        "/proc/self/maps" | "/proc/self/smaps" => {
            Some(FdKind::Instantane(Arc::new(carte_memoire())))
        }
        _ => None,
    }
}

// BOUCHAUD_V13_PROC_STATS_DYNAMIQUES_FORMAT
const PROC_USER_HZ: u64 = 100;
const PROC_TICK_NS: u64 = 1_000_000_000 / PROC_USER_HZ;

fn proc_ticks(ns: u64) -> u64 { ns / PROC_TICK_NS }

fn proc_pid_stat_depuis_chemin(path: &str) -> Option<u32> {
    let reste = path.strip_prefix("/proc/")?;
    let (pid, feuille) = reste.split_once('/')?;
    if feuille != "stat" || pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    pid.parse::<u32>().ok()
}

fn proc_reveil() -> Vec<u8> {
    use alloc::format;
    let r = crate::kernel::task::current().attribution_reveil.releve();
    let t = crate::kernel::attribution_reveil::releve_tics();
    let m = crate::kernel::attribution_reveil::releve_masquage();
    // Le nom est resolu ICI, hors interruption : le balayage n'a garde que
    // pid et tid.
    let par = match crate::kernel::attribution_reveil::depaquete_par(r.par) {
        None => alloc::string::String::from("-"),
        Some((pid, tid, noyau, en_irq)) => {
            let nom = crate::kernel::task::proc_processus_cumul(pid as u32)
                .map(|c| c.nom.rsplit('/').next().unwrap_or("").replace(' ', "_"))
                .unwrap_or_else(|| alloc::string::String::from("?"));
            format!("{}:pid={}:tid={}:noyau={}:irq={}", nom, pid, tid, noyau as u8, en_irq as u8)
        }
    };
    let us = |ns: u64| ns / 1000;
    let cyc_us = |c: u64| match crate::kernel::timer::cycles_par_ms() {
        0 => 0,
        par_ms => c.saturating_mul(1000) / par_ms,
    };
    let site = |s: u64| -> alloc::string::String {
        use crate::kernel::attribution_reveil::SITE_VECTEUR;
        if s == 0 {
            alloc::string::String::from("-")
        } else if s & SITE_VECTEUR != 0 {
            format!("irq:{:#x}", s & 0xff)
        } else {
            // SAFETY : `s` vient d'un `&'static Location` (SpinLockIrq).
            let l = unsafe { &*(s as usize as *const core::panic::Location<'static>) };
            format!("{}:{}", l.file(), l.line())
        }
    };
    let ligne1 = format!(
        "reveils={} pire_total_us={} tardif_us={} attente_us={} reprise_us={} dormi_ms={} tics_vus={} \
         sans_balayage={} cpu={} a_ms={} tardifs_gt_5ms={} attentes_gt_1ms={} sommeils_tics_manquants={} \
         tic_ecart_pire_us={} tic_rip={:#x} tic_ring3={} tic_cpu={} tic_a_ms={} tic_ecarts_gt_2ms={} \
         masque_avant_ecart_us={} masque_avant_ecart_site={} masque_pire_us={} masque_pire_site={} masques_gt_1ms={} \
         reveille_par={} reprise_commutation_us={} reprise_passation_us={} reprise_retour_us={}\n",
        r.reveils, us(r.pire.total_ns), us(r.pire.tardif_ns), us(r.pire.attente_ns), us(r.pire.reprise_ns),
        r.pire.dormi_ms, r.pire.tics_vus, r.pire.sans_balayage as u8, r.cpu, r.a_ns / 1_000_000,
        r.tardifs_signales, r.attentes_signalees, r.sommeils_tics_manquants,
        us(t.pire_ns), t.rip, t.ring3 as u8, t.cpu, t.a_ns / 1_000_000, t.signales,
        cyc_us(m.avant_ecart_cyc), site(m.avant_ecart_site), cyc_us(m.pire_cyc), site(m.pire_site), m.signalees,
        par, us(r.pire.commutation_ns), us(r.pire.passation_ns), us(r.pire.retour_ns),
    );
    // BOUCHAUD_ATTRIBUTION_REVEIL_V2 : seconde ligne, la ventilation des
    // reveils en retard de plus d'une milliseconde depuis le releve precedent
    // (`n:somme_us:pire_us` par part, n = reveils dont la part depassait le
    // seuil). BOUCHAUD_PREUVE_HOTE_V1 : « externe » = prouve par l'hote ;
    // les parts ni masquees ni prouvees sont « inexpliquees ».
    let v = crate::kernel::task::current().ventilation_reveil.releve();
    let (externes, masquees, inexpliquees) = crate::kernel::attribution_reveil::absences_comptees();
    let part = |c: crate::kernel::attribution_reveil::ReleveCompteur| format!("{}:{}:{}", c.n, us(c.somme_ns), us(c.pire_ns));
    let ligne2 = format!(
        "ventilation seuil_us={} preuve_hote={} ventiles={} contamines={} inexpliques={} incompletes={} \
         livraison_externe={} livraison_masquee={} livraison_inexpliquee={} detection={} election={} \
         election_externe={} election_inexpliquee={} reprise={} reprise_externe={} reprise_inexpliquee={} \
         absences_externes_boot={} absences_masquees_boot={} absences_inexpliquees_boot={}\n",
        crate::kernel::attribution_reveil::SEUIL_CONTAMINATION_NS / 1000,
        preuve_hote(),
        v.ventiles, v.contamines, v.inexpliques, v.incompletes,
        part(v.livraison_externe), part(v.livraison_masquee), part(v.livraison_inexpliquee),
        part(v.detection), part(v.election), part(v.election_externe), part(v.election_inexpliquee),
        part(v.reprise), part(v.reprise_externe), part(v.reprise_inexpliquee),
        externes, masquees, inexpliquees,
    );
    let mut sortie = ligne1.into_bytes();
    sortie.extend_from_slice(ligne2.as_bytes());
    sortie
}

/// `kvm-steal` si l'hyperviseur publie le vol d'au moins un coeur, sinon
/// `absente` (TCG, materiel nu) : aucune absence n'y est prouvee externe.
fn preuve_hote() -> &'static str {
    if crate::kernel::vol_hote::disponible() { "kvm-steal" } else { "absente" }
}

/// BOUCHAUD_PREUVE_HOTE_V1 : les cumuls du fil appelant
/// (`attribution_reveil::ComptesFil`) et le vol cumule de tous les coeurs ;
/// puis, depuis l'ouverture precedente, les sommeils que l'enregistreur a
/// ranges pour ce fil (`sommeils n=K perdus=L`, puis K lignes `echeance_us
/// entree_us externe_ns inexplique_ns masque_ns pret_ns`) et les absences de
/// son coeur closes pendant qu'il le tenait (`absences n=A incomplet=0|1`,
/// puis A lignes `fin_us E|M|I part_ns`). L'ouverture REARME l'enregistreur :
/// la sonde ouvre avant sa serie, puis apres.
fn proc_fil() -> Vec<u8> {
    use crate::kernel::attribution_reveil::{Cause, ENREGISTREUR};
    use alloc::format;
    let maintenant = crate::kernel::timer::monotonic_ns();
    let tache = crate::kernel::task::indice_courant();
    let f = crate::kernel::task::current().comptes_fil.cumuls();
    let (vol_total, coeurs) = crate::kernel::vol_hote::vole_total_ns();
    let mut sommeils = alloc::string::String::new();
    let mut n = 0usize;
    let perdus = ENREGISTREUR.lis(tache, |s| {
        n += 1;
        sommeils.push_str(&format!(
            "{} {} {} {} {} {}\n",
            s.echeance_ns / 1000, s.entree_ns / 1000, s.comptes.externe_ns, s.comptes.inexplique_ns,
            s.comptes.masque_ns, s.comptes.pret_ns
        ));
    });
    // Les absences closes pendant que le fil tenait son coeur, rangees par
    // le gestionnaire de battement depuis l'armement.
    let mut absences = alloc::string::String::new();
    let mut na = 0usize;
    let incomplet = ENREGISTREUR.lis_absences(tache, |a| {
        let cause = match a.cause {
            Cause::Externe => 'E',
            Cause::Masquee => 'M',
            Cause::Inexpliquee => 'I',
        };
        absences.push_str(&format!("{} {} {}\n", a.fin_ns / 1000, cause, a.part_ns));
        na += 1;
    });
    ENREGISTREUR.arme(tache, maintenant);
    let mut sortie = format!(
        "fil preuve_hote={} externe_ns={} inexplique_ns={} masque_ns={} pret_ns={} incompletes={} \
         vol_total_ns={} vol_coeurs={} a_ns={}\nsommeils n={} perdus={}\n",
        preuve_hote(), f.externe_ns, f.inexplique_ns, f.masque_ns, f.pret_ns, f.incompletes,
        vol_total, coeurs, maintenant, n, perdus,
    );
    sortie.push_str(&sommeils);
    sortie.push_str(&format!("absences n={} incomplet={}\n", na, incomplet as u8));
    sortie.push_str(&absences);
    sortie.into_bytes()
}

fn proc_stat_global() -> Vec<u8> {
    use alloc::format;
    // V16: l'agregation est faite a partir de points coherents PAR CPU.
    // La fonction raw proc_cpu_cumul() conserve tous ses diagnostics physiques.
    let c = crate::kernel::task::proc_cpu_publie();
    format!(
        "cpu  {} 0 {} {} 0 0 0 0 0 0\n",
        proc_ticks(c.user_ns),
        proc_ticks(c.system_ns),
        proc_ticks(c.idle_ns),
    ).into_bytes()
}

fn proc_stat_processus(pid: u32) -> Option<Vec<u8>> {
    use alloc::format;
    let c = crate::kernel::task::proc_processus_cumul(pid)?;
    // Le parser Ladybird epingle lit exactement 14=utime, 15=stime, 24=rss.
    // On fournit aussi ppid, nombre de threads et vsize pour rester utile aux
    // autres consommateurs. `comm` est nettoye car le parser amont emploie %s.
    let mut nom = c.nom.rsplit('/').next().unwrap_or(&c.nom).replace(' ', "_");
    nom = nom.replace(')', "_");
    let rss_pages = c.rss_octets / crate::kernel::vmm::PAGE_SIZE;
    Some(format!(
        "{} ({}) {} {} 0 0 0 0 0 0 0 0 0 {} {} 0 0 0 0 {} 0 0 {} {}\n",
        c.pid,
        nom,
        c.etat as char,
        c.ppid,
        proc_ticks(c.user_ns),
        proc_ticks(c.system_ns),
        c.threads,
        c.vss_octets,
        rss_pages,
    ).into_bytes())
}

/// Contenu de `/proc/self/maps` pour le processus courant.
///
/// ## Pourquoi ce fichier existe
///
/// Parce que la glibc en a besoin pour repondre a une question qu'elle ne sait
/// pas resoudre autrement : **ou commence et ou finit la pile du thread
/// principal ?**
///
/// Pour un thread cree par `pthread_create`, la reponse est connue — c'est la
/// libc qui a alloue la pile. Pour le thread principal, c'est le noyau qui l'a
/// posee, et rien dans l'ABI ne la lui communique. `pthread_getattr_np()` ouvre
/// donc `/proc/self/maps`, y cherche la ligne dont la plage contient
/// `__libc_stack_end`, et en deduit les bornes. Sans le fichier, elle rend
/// `ENOENT`.
///
/// Ce n'est pas un detail de confort. `AK::StackInfo` appelle
/// `pthread_getattr_np` et fait un `VERIFY_NOT_REACHED()` sur echec : **toute**
/// brique de Ladybird s'arrete la — AK, LibGC, LibJS. Et LibGC en a besoin pour
/// une raison de fond, pas seulement de demarrage : c'est un ramasse-miettes
/// **conservateur**, qui balaie la pile a la recherche de racines. Des bornes
/// fausses ne le feraient pas planter, elles lui feraient recolter des objets
/// vivants — un defaut qui se manifeste ailleurs, plus tard, sans lien apparent
/// avec sa cause.
///
/// ## Ce qu'on decrit
///
/// Les quatre regions que le noyau connait exactement : l'image, le tas `brk`,
/// la zone `mmap` et la pile. Le format est celui de Linux, que la glibc analyse
/// par `sscanf("%lx-%lx")` :
///
/// ```text
///   400000000000-400000001000 r-xp 00000000 00:00 0    /chemin/du/binaire
///   407efe000000-407effffffff rw-p 00000000 00:00 0    [stack]
/// ```
///
/// Seule la ligne `[stack]` compte pour `pthread_getattr_np`, mais les autres
/// coutent quelques lignes et rendent le fichier utile a `pmap`, aux
/// bibliotheques qui comptent leurs pages, et au diagnostic.
fn carte_memoire() -> Vec<u8> {
    use alloc::format;
    use alloc::string::String;

    let processus = crate::kernel::task::current_process();
    let p = processus.mm.lock();
    let name = processus.metadata.lock().name.clone();

    let sommet = crate::kernel::vmm::user_stack_top();
    let base_pile = sommet - crate::kernel::vmm::USER_STACK_SIZE;

    let mut texte = String::new();

    // L'image chargee. On n'en connait pas le decoupage en segments ici : une
    // seule ligne couvrant du debut de l'image au debut du tas suffit aux
    // consommateurs, et vaut mieux qu'une ligne inventee par segment.
    let base_image = crate::kernel::vmm::user_load_base();
    if p.brk_start > base_image {
        texte.push_str(&format!(
            "{:012x}-{:012x} r-xp 00000000 00:00 0                          /{}\n",
            base_image, p.brk_start, name
        ));
    }

    // Le tas `brk`. Vide tant que rien n'a ete demande — on ne l'ecrit alors
    // pas, plutot que d'annoncer une region de taille nulle.
    if p.brk > p.brk_start {
        texte.push_str(&format!(
            "{:012x}-{:012x} rw-p 00000000 00:00 0                          [heap]\n",
            p.brk_start, p.brk
        ));
    }

    // La zone `mmap`, de sa base au premier emplacement libre.
    let base_mmap = crate::kernel::vmm::user_mmap_base();
    if p.mmap_next > base_mmap {
        texte.push_str(&format!(
            "{:012x}-{:012x} rw-p 00000000 00:00 0\n",
            base_mmap, p.mmap_next
        ));
    }

    // La pile — la seule ligne dont `pthread_getattr_np` a reellement besoin.
    texte.push_str(&format!(
        "{:012x}-{:012x} rw-p 00000000 00:00 0                          [stack]\n",
        base_pile, sommet
    ));

    texte.into_bytes()
}
