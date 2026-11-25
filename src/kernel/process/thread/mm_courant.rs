// BOUCHAUD_MM_REMPLACABLE_V1 -- LE `Mm` D'UN PROCESSUS EST UNE REFERENCE, PAS UN CHAMP.
//
// Pourquoi
// ========
// `Process.mm` etait un `Arc<Mm>` FIGE a la creation du processus, et
// `execve` remplacait l'espace d'adressage EN PLACE dans ce `Mm`
// (`core::mem::replace(&mut mm.space, space)`). Tant qu'un `Mm` n'appartient
// qu'a un processus, c'est equivalent. Des qu'il est partage -- ce que
// `CLONE_VM|CLONE_VFORK` exige : le fils s'execute dans la memoire du pere
// jusqu'a son `execve` -- l'`execve` du fils detruirait l'espace du pere.
//
// C'est ce verrou qui obligeait `posix_spawn` (glibc et musl emettent
// `clone(CLONE_VM|CLONE_VFORK)`) a passer par un `fork` a COPIE COMPLETE :
// 11 a 18 ms de recopie par processus du navigateur, plus 7 a 10 ms pour
// rendre ces memes frames a l'`execve` qui suit (docs/current/PERF_LADYBIRD.md).
//
// Le contrat
// ==========
// * `Process.mm` est un `MmCourant` : une case qui designe le `Mm` COURANT.
//   `lock()` rend un garde qui TIENT le `Mm` qu'il a verrouille ; un `execve`
//   concurrent peut changer la case, il ne peut pas detruire ce qu'un garde
//   tient encore.
// * Un `Mm` rend lui-meme ses references (cache de pages propre, plages
//   `MAP_SHARED`) quand il meurt (`impl Drop for Mm`). Elles etaient rendues
//   par `Drop for Process` : avec un `Mm` partage, la mort du fils `vfork`
//   aurait rendu les references du PERE.
// * Le `execve` d'un processus dont le `Mm` est partage (fils vfork, ou pere
//   dont un fils vfork s'execute encore) INSTALLE un `Mm` neuf au lieu de
//   remplacer l'espace en place, et ne retire pas l'ancienne identite : elle
//   reste celle de l'autre processus.
//
// La case est un `SpinLockIrq` : elle est lue sur le chemin de commutation
// (`install`), interruptions masquees, et ne doit jamais pouvoir etre tenue
// par le contexte qu'une IRQ interrompt. Elle ne protege qu'un echange de
// pointeur : aucune autre prise sous elle.

/// La case qui designe le `Mm` courant d'un processus.
pub struct MmCourant {
    case: SpinLockIrq<Arc<Mm>>,
}

/// Un verrou sur l'etat d'un `Mm`, qui garde ce `Mm` vivant.
///
/// L'ordre des champs EST le contrat : un champ est detruit dans l'ordre de
/// declaration, donc le garde (qui emprunte l'interieur du `Mm`) tombe AVANT
/// la reference qui le fait vivre.
pub struct MmGarde {
    garde: SpinLockGuard<'static, MmState>,
    _mm: Arc<Mm>,
}

impl core::ops::Deref for MmGarde {
    type Target = MmState;
    #[inline]
    fn deref(&self) -> &MmState { &self.garde }
}

impl core::ops::DerefMut for MmGarde {
    #[inline]
    fn deref_mut(&mut self) -> &mut MmState { &mut self.garde }
}

impl MmCourant {
    pub fn new(mm: Arc<Mm>) -> Self {
        Self { case: SpinLockIrq::new(mm) }
    }

    /// Le `Mm` courant. Une reference : il survit a un `execve` concurrent.
    #[inline]
    pub fn get(&self) -> Arc<Mm> {
        Arc::clone(&self.case.lock())
    }

    /// Verrouille l'etat du `Mm` courant.
    ///
    /// `track_caller` : une attente longue nomme l'APPELANT
    /// (BOUCHAUD_SOUFFRANCE_SITE_DETENTEUR_V1), comme `Mm::lock`.
    #[track_caller]
    pub fn lock(&self) -> MmGarde {
        let mm = self.get();
        let garde = mm.lock();
        // SAFETY : `garde` emprunte l'interieur de l'allocation de `mm`, qui
        // ne bouge pas et vit tant que `_mm` existe ; `MmGarde` detruit
        // `garde` avant `_mm` (ordre des champs).
        let garde: SpinLockGuard<'static, MmState> = unsafe { core::mem::transmute(garde) };
        MmGarde { garde, _mm: mm }
    }

    /// Installe `neuf` et rend l'ancien `Mm`. Reserve a `execve`.
    pub fn remplace(&self, neuf: Arc<Mm>) -> Arc<Mm> {
        core::mem::replace(&mut *self.case.lock(), neuf)
    }

    /// Les deux processus designent-ils le meme `Mm` ?
    pub fn meme_que(&self, autre: &MmCourant) -> bool {
        Arc::ptr_eq(&self.case.lock(), &autre.case.lock())
    }

    // Les operations d'activation suivent le `Mm` COURANT : apres un
    // `execve`, c'est le nouvel espace que la commutation charge.
    #[inline]
    pub fn activate(&self) { self.get().activate() }
    #[inline]
    pub fn mark_inactive(&self, cpu: usize) { self.get().mark_inactive(cpu) }
    #[inline]
    pub fn reste_charge_ici(&self, cpu: usize) -> bool { self.get().reste_charge_ici(cpu) }
    #[inline]
    pub fn replace_activation(&self, identity: Arc<crate::kernel::vmm::AddressSpaceIdentity>) {
        self.get().replace_activation(identity)
    }
}

// BOUCHAUD_VFORK_REEL_V1 -- LE PERE ATTEND, LE FILS EMPRUNTE.
//
// `vfork` (et `clone(CLONE_VM|CLONE_VFORK)`, donc `posix_spawn`) : le fils
// s'execute sur le `Mm` du pere, sur une autre pile, et le FIL appelant du
// pere est suspendu jusqu'a ce que le fils ait fait `execve` ou soit mort.
// La suspension n'est pas une politesse : la glibc libere la pile du fils au
// retour de `clone` dans le pere ; un pere qui repartirait trop tot la
// rendrait sous les pieds du fils.
//
// Un lien, une liberation : `libere` est idempotent, et TOUT chemin qui met
// fin a l'emprunt (execve reussi, mort du dernier fil, destruction du
// processus en filet) l'appelle. Aucun chemin ne peut laisser le pere dormir.
pub struct LienVfork {
    libere: AtomicBool,
    file: crate::kernel::sync::WaitQueue,
    /// Le `Mm` emprunte : son compteur d'emprunts redescend a la liberation.
    mm: Arc<Mm>,
}

static VFORK_CREES: AtomicU64 = AtomicU64::new(0);
static VFORK_PAR_EXECVE: AtomicU64 = AtomicU64::new(0);
static VFORK_PAR_SORTIE: AtomicU64 = AtomicU64::new(0);
static VFORK_ATTENTE_NS: AtomicU64 = AtomicU64::new(0);
static VFORK_ATTENTE_PIRE_NS: AtomicU64 = AtomicU64::new(0);

/// Pourquoi l'emprunt finit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FinEmprunt {
    Execve,
    Sortie,
}

impl LienVfork {
    pub fn nouveau(mm: Arc<Mm>) -> Arc<Self> {
        mm.emprunts.fetch_add(1, Ordering::SeqCst);
        VFORK_CREES.fetch_add(1, Ordering::Relaxed);
        Arc::new(Self {
            libere: AtomicBool::new(false),
            file: crate::kernel::sync::WaitQueue::new(),
            mm,
        })
    }

    #[inline]
    pub fn est_libere(&self) -> bool {
        self.libere.load(Ordering::SeqCst)
    }

    /// Met fin a l'emprunt et reveille le pere. Idempotent.
    pub fn libere(&self, fin: FinEmprunt) {
        if self.libere.swap(true, Ordering::SeqCst) {
            return;
        }
        self.mm.emprunts.fetch_sub(1, Ordering::SeqCst);
        match fin {
            FinEmprunt::Execve => VFORK_PAR_EXECVE.fetch_add(1, Ordering::Relaxed),
            FinEmprunt::Sortie => VFORK_PAR_SORTIE.fetch_add(1, Ordering::Relaxed),
        };
        self.file.wake_all();
    }

    /// Le fil appelant du pere attend la fin de l'emprunt.
    ///
    /// Attente NON interruptible par un signal ordinaire, comme sous Linux
    /// (`TASK_KILLABLE`) : un signal n'y met pas fin, puisque reprendre le
    /// pere rendrait la pile du fils. Une tache CONDAMNEE (SIGKILL,
    /// exit_group d'un frere) n'attend plus : le `Mm` survit a sa mort, le
    /// fils le tient.
    ///
    /// L'echeance periodique n'est pas une scrutation du lien : un reveil le
    /// livre. Elle ne sert qu'a relire la condamnation, qui ne passe pas par
    /// cette file.
    pub fn attend(&self) {
        let debut = crate::kernel::timer::monotonic_ns();
        loop {
            let ticket = self.file.ticket();
            if self.est_libere() || condamnee_courante() {
                break;
            }
            let echeance = crate::kernel::timer::monotonic_ns().saturating_add(100_000_000);
            self.file.wait_until_noyau(ticket, echeance);
        }
        let attente = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
        VFORK_ATTENTE_NS.fetch_add(attente, Ordering::Relaxed);
        VFORK_ATTENTE_PIRE_NS.fetch_max(attente, Ordering::Relaxed);
    }
}

impl Process {
    /// Termine l'emprunt `vfork` de ce processus, s'il en a un.
    pub fn libere_vfork(&self, fin: FinEmprunt) {
        let lien = self.vfork.lock().take();
        if let Some(lien) = lien {
            lien.libere(fin);
        }
    }

    /// Ce processus s'execute-t-il encore sur le `Mm` de son pere ?
    pub fn emprunte_mm(&self) -> bool {
        self.vfork.lock().is_some()
    }
}

/// (crees, finis par execve, finis par sortie, attente cumulee ns, pire ns)
pub fn vfork_stats() -> (u64, u64, u64, u64, u64) {
    (
        VFORK_CREES.load(Ordering::Relaxed),
        VFORK_PAR_EXECVE.load(Ordering::Relaxed),
        VFORK_PAR_SORTIE.load(Ordering::Relaxed),
        VFORK_ATTENTE_NS.load(Ordering::Relaxed),
        VFORK_ATTENTE_PIRE_NS.load(Ordering::Relaxed),
    )
}

/// BOUCHAUD_GETPPID_V1 -- le pere que `getppid` doit rendre.
///
/// Un pere vivant (enregistre, pas encore zombie) ; sinon 1 : l'orphelin est
/// rattache a init, comme sous Linux. Le pid 0 (processus lance par le noyau)
/// rend aussi 1.
pub fn ppid_visible(parent: u32) -> u32 {
    if parent == 0 {
        return 1;
    }
    match process_by_pid(parent) {
        Some(pere) if !pere.lifecycle.lock().zombie => parent,
        _ => 1,
    }
}
