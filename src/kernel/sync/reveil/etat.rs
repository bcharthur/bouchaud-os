// État du domaine événementiel interface.

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
