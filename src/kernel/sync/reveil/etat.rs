// État du domaine événementiel interface.

const INTERFACE_WAIT_IDLE: u8 = 0;
const INTERFACE_WAIT_PREPARE: u8 = 1;
const INTERFACE_WAIT_SLEEP: u8 = 2;
const INTERFACE_WAIT_RESUME: u8 = 3;
const INTERFACE_WAIT_RETURN: u8 = 4;

static INTERFACE_WAIT_PHASE: AtomicU8 = AtomicU8::new(INTERFACE_WAIT_IDLE);
static INTERFACE_WAIT_PHASE_SINCE_NS: AtomicU64 = AtomicU64::new(0);
static INTERFACE_DETACHED_WAITS: AtomicU64 = AtomicU64::new(0);
static INTERFACE_DETACHED_SLEEP_NS: AtomicU64 = AtomicU64::new(0);
static INTERFACE_DETACHED_SLEEP_MAX_NS: AtomicU64 = AtomicU64::new(0);
static INTERFACE_RESUME_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static INTERFACE_RESUME_WAIT_MAX_NS: AtomicU64 = AtomicU64::new(0);
static INTERFACE_DEPTH_VIOLATIONS: AtomicU64 = AtomicU64::new(0);
// V16: depth=2 etait justement le cas des stalls 4-5 s du desktop.
// On distingue profondeur racine et profondeur imbriquee pour verifier
// qu'on detache effectivement les deux sans casser le contrat de retour.
static INTERFACE_DETACHED_DEPTH1: AtomicU64 = AtomicU64::new(0);
static INTERFACE_DETACHED_NESTED: AtomicU64 = AtomicU64::new(0);
static INTERFACE_DETACHED_MAX_DEPTH: AtomicU64 = AtomicU64::new(0);

#[inline]
fn interface_update_max(atom: &AtomicU64, value: u64) {
    let mut old = atom.load(Ordering::Relaxed);
    while value > old {
        match atom.compare_exchange_weak(
            old,
            value,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(now) => old = now,
        }
    }
}

#[inline]
fn interface_phase(phase: u8) {
    INTERFACE_WAIT_PHASE.store(phase, Ordering::Release);
    INTERFACE_WAIT_PHASE_SINCE_NS.store(
        crate::kernel::timer::monotonic_ns(),
        Ordering::Release,
    );
}

pub struct Reveil {
    source: WaitSource,
    compteurs: [AtomicU64; NOMBRE_SOURCES],
    sommeils: AtomicU64,
    sommeils_evites: AtomicU64,
    reveils_signal: AtomicU64,
    reveils_echeance: AtomicU64,

    // Hard-IRQ coalescing state. Correctness is carried by WaitSource generation.
    irq_pending: AtomicBool,
    irq_signals: AtomicU64,
    irq_flushes: AtomicU64,
    irq_woken: AtomicU64,
}

impl Reveil {
    pub const fn new() -> Self {
        Self {
            source: WaitSource::new(),
            compteurs: [const { AtomicU64::new(0) }; NOMBRE_SOURCES],
            sommeils: AtomicU64::new(0),
            sommeils_evites: AtomicU64::new(0),
            reveils_signal: AtomicU64::new(0),
            reveils_echeance: AtomicU64::new(0),
            irq_pending: AtomicBool::new(false),
            irq_signals: AtomicU64::new(0),
            irq_flushes: AtomicU64::new(0),
            irq_woken: AtomicU64::new(0),
        }
    }
}
