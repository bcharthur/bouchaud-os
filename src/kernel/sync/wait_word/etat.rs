// State and counters.

const WAIT_WORD_BUCKETS: usize = 64;

struct WaitWordEntry {
    key: u64,
    wait: WaitSource,
    waiters: AtomicU64,
}

impl WaitWordEntry {
    fn new(key: u64) -> Self {
        Self { key, wait: WaitSource::new(), waiters: AtomicU64::new(0) }
    }
}

static WAIT_WORD_TABLE: [SpinLock<Vec<Arc<WaitWordEntry>>>; WAIT_WORD_BUCKETS] =
    [const { SpinLock::new(Vec::new()) }; WAIT_WORD_BUCKETS];

static WW_WAITS: AtomicU64 = AtomicU64::new(0);
static WW_VALUE_CHANGED: AtomicU64 = AtomicU64::new(0);
static WW_SIGNALED: AtomicU64 = AtomicU64::new(0);
static WW_DEADLINES: AtomicU64 = AtomicU64::new(0);
static WW_FAULTS: AtomicU64 = AtomicU64::new(0);
static WW_WAKES: AtomicU64 = AtomicU64::new(0);
static WW_WAKE_MISSES: AtomicU64 = AtomicU64::new(0);
static WW_ENTRIES_CREATED: AtomicU64 = AtomicU64::new(0);
static WW_ENTRIES_PRUNED: AtomicU64 = AtomicU64::new(0);
static WW_BUCKET_PEAK: AtomicU64 = AtomicU64::new(0);
