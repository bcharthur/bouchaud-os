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

/// Commits reussis, et generation du dernier.
///
/// La generation est ce qui distingue l'ancien etat du neuf au montage. La
/// publier permet de verifier, dans une trace de coupure de courant, que le
/// systeme remonte bien sur la generation attendue -- et pas sur une plus
/// ancienne, ce qui serait une perte silencieuse.
static TX_COMMITS: AtomicU64 = AtomicU64::new(0);
static TX_GENERATION: AtomicU64 = AtomicU64::new(0);
static TX_MONTAGES_V1: AtomicU64 = AtomicU64::new(0);
static TX_SUPERBLOCS_REJETES: AtomicU64 = AtomicU64::new(0);
