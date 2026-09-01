// Etat atomique du domaine idle.
//
// Aucun de ces compteurs ne participe à une décision du scheduler : ce sont
// uniquement de l'accounting et de l'observabilité.

static IDLE: [AtomicBool; smp::MAX_CPUS] =
    [const { AtomicBool::new(false) }; smp::MAX_CPUS];
static IDLE_SINCE_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static IDLE_ACCUM_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];

const IDLE_PHASE_RUNNING: u8 = 0;
const IDLE_PHASE_SCHED_PREPARED: u8 = 1;
const IDLE_PHASE_SCHED_COMMIT: u8 = 2;
const IDLE_PHASE_SCHED_HLT: u8 = 3;
const IDLE_PHASE_SCHED_SAFE: u8 = 4;
const IDLE_PHASE_LOCK_PREPARED: u8 = 5;
const IDLE_PHASE_LOCK_COMMIT: u8 = 6;
const IDLE_PHASE_LOCK_HLT: u8 = 7;
const IDLE_PHASE_LOCK_SAFE: u8 = 8;
const IDLE_PHASE_WFI_HLT: u8 = 9;
const IDLE_PHASE_WFI_SAFE: u8 = 10;
const IDLE_PHASE_GENERIC_HLT: u8 = 11;

static IDLE_PHASE: [AtomicU8; smp::MAX_CPUS] =
    [const { AtomicU8::new(IDLE_PHASE_RUNNING) }; smp::MAX_CPUS];
static IDLE_PHASE_SINCE_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static IDLE_SEQ: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];

static SCHED_PREPARES: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static SCHED_COMMITS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static SCHED_WAKES: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static SCHED_SAFE_RETURNS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
/// Endormissements PREPARES puis abandonnes parce que la file s'etait remplie
/// entre-temps. Chaque unite est un reveil qui aurait ete perdu.
static SCHED_ABANDONS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];

pub fn sched_abandons(cpu: usize) -> u64 {
    if cpu >= smp::MAX_CPUS { return 0; }
    SCHED_ABANDONS[cpu].load(Ordering::Relaxed)
}

static LOCK_PREPARES: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static LOCK_COMMITS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static LOCK_WAKES: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static LOCK_SAFE_RETURNS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];

static WFI_ENTERS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static WFI_WAKES: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static WFI_SAFE_RETURNS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];

static IDLE_SLEEP_MAX_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];

static PIT_TICKS_SEEN: AtomicU64 = AtomicU64::new(0);
static LAST_PIT_NS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn idle_trace_phase(cpu: usize, phase: u8) {
    IDLE_PHASE[cpu].store(phase, Ordering::Release);
    IDLE_PHASE_SINCE_NS[cpu].store(
        crate::kernel::timer::monotonic_ns(),
        Ordering::Release,
    );
}

#[inline]
fn idle_next_seq(cpu: usize) -> u64 {
    IDLE_SEQ[cpu].fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

#[inline]
fn idle_enter(cpu: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    IDLE_SINCE_NS[cpu].store(now, Ordering::Release);
    // SeqCst, et non Release : cette ecriture est la premiere moitie d'un
    // motif CROISE. Nous publions « je dors » puis nous relisons la file ; le
    // reveilleur remplit la file puis relit « dort-il ? ». Deux ecritures
    // suivies de deux lectures croisees, sur DEUX objets differents.
    //
    // `Release` n'ordonne une ecriture qu'avec la lecture qui la suit chez
    // l'autre. Il ne dit rien de ce motif -- et sur x86 le seul reordonnancement
    // permis est justement store -> load : le tampon d'ecriture laisse notre
    // relecture de la file passer AVANT cette publication. Les deux cotes se
    // manqueraient, et la tache dormirait pour toujours.
    //
    // SeqCst place les deux ecritures dans un ordre total unique : au moins
    // l'un des deux voit l'autre. C'est le meme argument que
    // `kernel::sync::rendezvous`, et pour la meme raison.
    IDLE[cpu].store(true, Ordering::SeqCst);
}

#[inline]
fn idle_exit(cpu: usize) -> u64 {
    let now = crate::kernel::timer::monotonic_ns();
    if IDLE[cpu].swap(false, Ordering::AcqRel) {
        let since = IDLE_SINCE_NS[cpu].swap(0, Ordering::AcqRel);
        let elapsed = now.saturating_sub(since);
        IDLE_ACCUM_NS[cpu].fetch_add(elapsed, Ordering::Relaxed);
        IDLE_SLEEP_MAX_NS[cpu].fetch_max(elapsed, Ordering::Relaxed);
        elapsed
    } else {
        0
    }
}

#[inline]
fn idle_ns_at(cpu: usize, now: u64) -> u64 {
    let accumulated = IDLE_ACCUM_NS[cpu].load(Ordering::Acquire);
    if IDLE[cpu].load(Ordering::Acquire) {
        accumulated.saturating_add(
            now.saturating_sub(IDLE_SINCE_NS[cpu].load(Ordering::Acquire)),
        )
    } else {
        accumulated
    }
}

pub fn idle_ns(cpu: usize) -> u64 {
    if cpu >= smp::MAX_CPUS {
        0
    } else {
        idle_ns_at(cpu, crate::kernel::timer::monotonic_ns())
    }
}

pub fn is_idle(cpu: usize) -> bool {
    cpu < smp::MAX_CPUS && IDLE[cpu].load(Ordering::Acquire)
}

pub fn idle_mask() -> u64 {
    let mut mask = 0u64;
    for cpu in 0..smp::schedulable_cpus().min(64) {
        if is_idle(cpu) {
            mask |= 1u64 << cpu;
        }
    }
    mask
}

#[inline]
fn note_pit_tick() {
    PIT_TICKS_SEEN.fetch_add(1, Ordering::Relaxed);
    LAST_PIT_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Release);
}

