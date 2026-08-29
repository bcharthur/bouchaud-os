// State and neutral Bouchaud readiness bits.

pub const READABLE: u32 = 1 << 0;
pub const WRITABLE: u32 = 1 << 1;
pub const HANGUP: u32 = 1 << 2;
pub const ERROR: u32 = 1 << 3;
pub const PRIORITY: u32 = 1 << 4;

#[derive(Clone, Copy)]
pub struct ReadinessTicket {
    wait: WaitSourceTicket,
    interest: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ReadinessStats {
    pub bits: u32,
    pub changes: u64,
    pub waits: u64,
    pub immediate_hits: u64,
    pub wakeups: u64,
}

pub struct ReadinessSource {
    bits: AtomicU32,
    wait: WaitSource,
    changes: AtomicU64,
    waits: AtomicU64,
    immediate_hits: AtomicU64,
    wakeups: AtomicU64,
}

impl ReadinessSource {
    pub const fn new(initial: u32) -> Self {
        Self {
            bits: AtomicU32::new(initial),
            wait: WaitSource::new(),
            changes: AtomicU64::new(0),
            waits: AtomicU64::new(0),
            immediate_hits: AtomicU64::new(0),
            wakeups: AtomicU64::new(0),
        }
    }
}
