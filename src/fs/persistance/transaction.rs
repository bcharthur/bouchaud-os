// Transaction serialization and timing.

static TRANSACTION: SleepMutex<()> = SleepMutex::new(());
static TX_CALLS: AtomicU64 = AtomicU64::new(0);
static TX_SNAPSHOT_NS: AtomicU64 = AtomicU64::new(0);
static TX_HASH_NS: AtomicU64 = AtomicU64::new(0);
static TX_IO_NS: AtomicU64 = AtomicU64::new(0);
static TX_RESUME_NS: AtomicU64 = AtomicU64::new(0);
static TX_BYTES: AtomicU64 = AtomicU64::new(0);
static TX_WRITTEN: AtomicU64 = AtomicU64::new(0);
static TX_SKIPPED: AtomicU64 = AtomicU64::new(0);
static TX_MAX_NS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn tx_max(value: u64) { TX_MAX_NS.fetch_max(value, Ordering::Relaxed); }
