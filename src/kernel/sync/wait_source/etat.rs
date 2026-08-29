// State shared by every native wait source.

#[derive(Clone, Copy)]
pub struct WaitSourceTicket {
    queue: WaitTicket,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitSourceWake {
    AlreadyChanged,
    Signaled,
    Deadline,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WaitSourceStats {
    pub generation: u64,
    pub tickets: u64,
    pub waits: u64,
    pub avoided_waits: u64,
    pub signaled: u64,
    pub deadlines: u64,
    pub signals: u64,
    pub deferred_publications: u64,
    pub deferred_flushes: u64,
    pub tasks_woken: u64,
}

pub struct WaitSource {
    queue: WaitQueue,
    generation: AtomicU64,
    tickets: AtomicU64,
    waits: AtomicU64,
    avoided_waits: AtomicU64,
    signaled: AtomicU64,
    deadlines: AtomicU64,
    signals: AtomicU64,
    deferred_publications: AtomicU64,
    deferred_flushes: AtomicU64,
    tasks_woken: AtomicU64,
}

impl WaitSource {
    pub const fn new() -> Self {
        Self {
            queue: WaitQueue::new(),
            generation: AtomicU64::new(1),
            tickets: AtomicU64::new(0),
            waits: AtomicU64::new(0),
            avoided_waits: AtomicU64::new(0),
            signaled: AtomicU64::new(0),
            deadlines: AtomicU64::new(0),
            signals: AtomicU64::new(0),
            deferred_publications: AtomicU64::new(0),
            deferred_flushes: AtomicU64::new(0),
            tasks_woken: AtomicU64::new(0),
        }
    }
}

impl Default for WaitSource {
    fn default() -> Self {
        Self::new()
    }
}
