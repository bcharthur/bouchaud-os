pub struct MmState {
    pub space: AddressSpace,
    pub brk_start: u64,
    pub brk: u64,
    pub mmap_next: u64,
    pub partages: Vec<Partage>,
    pub limite_as: u64,
    pub promesses: Vec<Promesse>,
    pub clean_pages: Vec<CleanPageMapping>,
}

pub struct Mm {
    inner: SpinLock<MmState>,
    activation: SpinLock<Arc<crate::kernel::vmm::AddressSpaceIdentity>>,
}
impl Mm {
    pub fn new(state: MmState) -> Self {
        let activation = state.space.identity();
        Self {
            inner: SpinLock::new(state),
            activation: SpinLock::new(activation),
        }
    }
    pub fn lock(&self) -> SpinLockGuard<'_, MmState> { self.inner.lock() }
    pub fn activate(&self) {
        loop {
            let identity = Arc::clone(&self.activation.lock());
            if unsafe { identity.try_activate() } {
                return;
            }
            core::hint::spin_loop();
        }
    }
    pub fn mark_inactive(&self, cpu: usize) {
        let identity = Arc::clone(&self.activation.lock());
        identity.mark_inactive(cpu);
    }

    pub fn replace_activation(&self, identity: Arc<crate::kernel::vmm::AddressSpaceIdentity>) {
        *self.activation.lock() = identity;
    }
}

/// La table des descripteurs d'un processus.
///
/// Elle porte le rang `FdTable`, en amont de `Vfs` : un appel systeme resout
/// son descripteur PUIS touche le systeme de fichiers. L'ordre inverse existait
/// -- `write`, `openat` et `getdents` tenaient le RAMFS pendant qu'ils
/// reportaient un offset -- et seul le gros verrou empechait les deux sens de
/// se croiser. Le rang rend l'inversion detectable sans attendre l'entrelacement.
pub struct FileTable { inner: RankedSpinLock<FdTable> }
impl FileTable {
    pub fn new(table: FdTable) -> Self {
        Self { inner: RankedSpinLock::new(LockClass::FdTable, table) }
    }
    pub fn lock(&self) -> RankedSpinLockGuard<'_, FdTable> { self.inner.lock() }
}

pub struct ProcessMetadata {
    pub name: String,
    pub cwd: usize,
    pub uid: u32,
    pub gid: u32,
    pub ecran: Option<EcranVirtuel>,
}

/// `groupe_en_sortie` (BOUCHAUD_GROUPE_RECLAME_V1) : un fil du processus a
/// RECLAME le groupe -- `exit_group` ou `execve` -- et va tuer les autres.
/// Pose sous ce verrou, relu sous ce verrou : c'est ce qui remplace la
/// serialisation que le gros verrou donnait a ces appels.
pub struct ProcessLifecycle { pub exit_code: i32, pub zombie: bool, pub threads: usize, pub groupe_en_sortie: bool }

pub struct Process {
    pub pid: u32,
    /// PID du parent (0 pour le processus lance depuis le shell).
    pub parent: u32,
    /// Groupe applicatif léger. Un nouveau processus racine crée son groupe;
    /// fork hérite l'identité, sans influencer le placement scheduler.
    pub resource_group_id: u32,
    pub resource_group_name: String,
    pub mm: Arc<Mm>,
    pub files: Arc<FileTable>,
    pub metadata: SpinLock<ProcessMetadata>,
    pub lifecycle: SpinLock<ProcessLifecycle>,
    pub signals: SpinLock<crate::kernel::signal::SignalState>,
    /// Temps CPU des fils de ce processus dont l'emplacement a ete recycle.
    pub temps_recycle: TempsRecycle,
    /// Appels systeme de ce processus, par numero (BOUCHAUD_APPELS_PAR_PROCESSUS_V1).
    pub appels: AppelsSyscall,
}

/// Nombre de numeros d'appel systeme comptes par processus.
pub const APPELS_NR_MAX: usize = 512;

/// BOUCHAUD_APPELS_PAR_PROCESSUS_V1 -- combien d'appels systeme, et
/// lesquels, chaque processus fait.
///
/// Endurance 37654172489 (TCG, 10 min) : des services AU REPOS brulent un
/// coeur en continu -- ImageDecoder 20 %, RequestServer 21 % (ils decodent un
/// pixel toutes les ~16 s), le Compositor 72 % -- pendant qu'un cadre
/// `srcdoc` de 200x80 met 16 s en moyenne a charger. Une boucle
/// d'evenements qui tourne a vide se voit dans ses appels, pas dans son
/// temps : la table `SYSCALL_TEMPS` est GLOBALE et ne dit pas qui. Un
/// compteur par numero, ecrit par le seul fil appelant en `Relaxed` : un
/// increment atomique par appel systeme, aucun verrou.
pub struct AppelsSyscall {
    pub total: core::sync::atomic::AtomicU64,
    par_nr: [core::sync::atomic::AtomicU32; APPELS_NR_MAX],
}

impl AppelsSyscall {
    pub const fn neuf() -> Self {
        Self {
            total: core::sync::atomic::AtomicU64::new(0),
            par_nr: [const { core::sync::atomic::AtomicU32::new(0) }; APPELS_NR_MAX],
        }
    }

    #[inline]
    pub fn note(&self, nr: u64) {
        use core::sync::atomic::Ordering;
        self.total.fetch_add(1, Ordering::Relaxed);
        if let Some(case) = self.par_nr.get(nr as usize) {
            case.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Les trois numeros les plus appeles : (numero, appels), 0 si absent.
    pub fn trois_premiers(&self) -> [(u16, u32); 3] {
        use core::sync::atomic::Ordering;
        let mut top = [(0u16, 0u32); 3];
        for (nr, case) in self.par_nr.iter().enumerate() {
            let n = case.load(Ordering::Relaxed);
            if n > top[2].1 {
                top[2] = (nr as u16, n);
                if top[2].1 > top[1].1 {
                    top.swap(1, 2);
                }
                if top[1].1 > top[0].1 {
                    top.swap(0, 1);
                }
            }
        }
        top
    }
}

/// BOUCHAUD_COMPTA_SEQLOCK_V1 -- le temps des fils disparus d'un processus.
///
/// Un fil mort reste lisible tant que son emplacement de tache n'est pas
/// reutilise ; une fois ecrase, ses compteurs quittaient le total du
/// processus, et `utime` de /proc/<pid>/stat RECULAIT (sonde compta-stress,
/// QEMU : « fils pid=48 utime=1<-2 »). Le registre les verse ici AVANT
/// l'ecrasement, sous sa section d'ecriture -- qui attend tous les lecteurs :
/// un parcours de la table voit donc le fil OU ce versement, jamais les deux,
/// jamais aucun.
pub struct TempsRecycle {
    pub user_ns: core::sync::atomic::AtomicU64,
    pub noyau_ns: core::sync::atomic::AtomicU64,
}

impl TempsRecycle {
    pub const fn neuf() -> Self {
        Self {
            user_ns: core::sync::atomic::AtomicU64::new(0),
            noyau_ns: core::sync::atomic::AtomicU64::new(0),
        }
    }
}

// Compile-time contract: Task and the registry may transfer/share Process
// references across CPUs without relying on the BKL.
#[allow(dead_code)]
fn assert_process_is_send_sync() {
    fn assert_traits<T: Send + Sync>() {}
    assert_traits::<Process>();
    assert_traits::<Mm>();
    assert_traits::<FileTable>();
}

/// Redirection de `/dev/fb0` vers une surface partagee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcranVirtuel {
    /// Nœud RAMFS anonyme qui porte les pixels.
    pub node: usize,
    pub largeur: u32,
    pub hauteur: u32,
    /// Octets par ligne.
    pub pas: u32,
}

/// Une plage `MAP_SHARED` projetee dans un processus.
///
/// Elle existe pour une seule raison : savoir quelle reference rendre au cache
/// de pages quand la plage disparait — par `munmap`, par `execve` ou avec le
/// processus. Sans cette trace, `munmap` ne saurait pas quel nœud il vient de
/// lacher, et les frames resteraient allouees pour toujours.
#[derive(Clone, Copy)]
pub struct Partage {
    pub base: u64,
    pub length: u64,
    pub node: usize,
}

#[derive(Clone, Copy)]
pub struct CleanPageMapping {
    pub virt: u64,
    pub key: crate::kernel::clean_page_cache::Key,
}

impl MmState {
    pub fn retire_clean_pages(&mut self, addr: u64, len: u64) -> Vec<crate::kernel::clean_page_cache::Key> {
        let fin = addr.saturating_add(len);
        let mut keys = Vec::new();
        self.clean_pages.retain(|mapping| {
            if mapping.virt >= addr && mapping.virt < fin {
                keys.push(mapping.key);
                false
            } else { true }
        });
        keys
    }

    pub fn has_clean_pages(&self, addr: u64, len: u64) -> bool {
        let fin = addr.saturating_add(len);
        self.clean_pages.iter().any(|mapping| mapping.virt >= addr && mapping.virt < fin)
    }

    pub fn release_clean_pages(&mut self) {
        for mapping in self.clean_pages.drain(..) {
            crate::kernel::clean_page_cache::release(mapping.key);
        }
    }
    /// Le nœud partage qui couvre cette adresse, s'il y en a un.
    pub fn partage_a(&self, addr: u64) -> Option<usize> {
        self.partages
            .iter()
            .find(|p| addr >= p.base && addr < p.base + p.length)
            .map(|p| p.node)
    }

    /// Retire `[addr, addr+len)` des plages partagees. A middle punch splits
    /// one mapping reference into two; prefix/suffix punches retain exactly
    /// one reference, and complete removal returns the reference to release.
    pub fn retire_partages(&mut self, addr: u64, len: u64) -> Vec<usize> {
        let fin = addr.saturating_add(len);
        let mut rendus = Vec::new();
        let mut restantes = Vec::with_capacity(self.partages.len() + 1);
        for p in self.partages.drain(..) {
            let p_fin = p.base.saturating_add(p.length);
            if fin <= p.base || addr >= p_fin {
                restantes.push(p);
                continue;
            }

            let gauche = addr.saturating_sub(p.base);
            let droite = p_fin.saturating_sub(fin);
            if gauche != 0 {
                restantes.push(Partage {
                    base: p.base,
                    length: gauche,
                    node: p.node,
                });
            }
            if droite != 0 {
                restantes.push(Partage {
                    base: fin,
                    length: droite,
                    node: p.node,
                });
            }
            match (gauche != 0, droite != 0) {
                (false, false) => rendus.push(p.node),
                (true, true) => crate::kernel::partage::mappe(p.node),
                _ => {}
            }
        }
        self.partages = restantes;
        rendus
    }

    /// Relache toutes les plages partagees (`execve`, fin du processus).
    pub fn relache_partages(&mut self) {
        let nœuds: Vec<usize> = self.partages.iter().map(|p| p.node).collect();
        self.partages.clear();
        for node in nœuds {
            crate::kernel::partage::demappe(node);
        }
    }

    /// Taille actuelle de l'espace d'adressage, en octets.
    ///
    /// Ce noyau n'a pas d'allocation paresseuse : `mmap` et `brk` mappent
    /// immediatement ce qu'ils promettent. Taille virtuelle et taille residente
    /// coincident donc, et compter les pages possedees est une mesure exacte de
    /// ce que `RLIMIT_AS` borne.
pub fn taille_as(&self) -> u64 {
        let virtuel =
            crate::kernel::vma::octets_virtuels(&self.promesses);
        let resident = self.space.mapped_pages() as u64
            * crate::kernel::vmm::PAGE_SIZE;
        core::cmp::max(virtuel, resident)
    }

    /// Cette croissance tiendrait-elle sous `RLIMIT_AS` ?
    pub fn tient_sous_limite(&self, croissance: u64) -> bool {
        self.limite_as == 0 || self.taille_as() + croissance <= self.limite_as
    }
}

impl Drop for Process {
    /// Un processus qui disparait rend ses references sur le cache partage.
    ///
    /// C'est le filet : `munmap` couvre le cas ordinaire, celui-ci couvre la
    /// mort brutale — un `SIGKILL`, une faute de page fatale, un `exit` sans
    /// menage. Sans lui, tuer un renderer suffirait a faire fuir ses surfaces.
    fn drop(&mut self) {
        crate::kernel::security::policy::forget(self.pid);
        let (pml4, clean, shared) = {
            let mut mm = self.mm.lock();
            let pml4 = mm.space.pml4();
            let clean = core::mem::take(&mut mm.clean_pages);
            let shared = core::mem::take(&mut mm.partages);
            (pml4, clean, shared)
        };
        forget_fault_space(pml4);
        for mapping in clean {
            crate::kernel::clean_page_cache::release(mapping.key);
        }
        for mapping in shared {
            crate::kernel::partage::demappe(mapping.node);
        }
    }
}

