// Per-CPU sequential history and V14 counters.

const RA_CPUS: usize = 64;
static LAST_NODE: [AtomicUsize; RA_CPUS] = [const { AtomicUsize::new(usize::MAX) }; RA_CPUS];
static LAST_OFFSET: [AtomicU64; RA_CPUS] = [const { AtomicU64::new(u64::MAX) }; RA_CPUS];
static RUN: [AtomicU64; RA_CPUS] = [const { AtomicU64::new(0) }; RA_CPUS];
static RA_OBSERVE: AtomicU64 = AtomicU64::new(0);
static RA_SEQUENTIAL: AtomicU64 = AtomicU64::new(0);
static RA_REQUESTED: AtomicU64 = AtomicU64::new(0);
static RA_OK: AtomicU64 = AtomicU64::new(0);
static RA_FAIL: AtomicU64 = AtomicU64::new(0);
static RA_WINDOW_2: AtomicU64 = AtomicU64::new(0);
static RA_WINDOW_4: AtomicU64 = AtomicU64::new(0);
static RA_WINDOW_8: AtomicU64 = AtomicU64::new(0);
static RA_WINDOW_16: AtomicU64 = AtomicU64::new(0);
static RA_MAX_WINDOW_SEEN: AtomicU64 = AtomicU64::new(0);
