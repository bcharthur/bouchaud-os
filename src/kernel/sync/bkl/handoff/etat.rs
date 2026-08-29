// État atomique du waiter handoff V10.
//
// `HANDOFF_TARGET` reprend le même codage que OWNER : 0 = aucun, CPU+1 = cible.
// Une reprise scheduler (`RESUME_WAITERS`) reste toujours plus prioritaire.

const HANDOFF_LEASE_NS: u64 = 50_000_000; // 50 ms, borne de vivacité sous TCG.

static HANDOFF_TARGET: AtomicUsize = AtomicUsize::new(FREE);
static HANDOFF_SINCE_NS: AtomicU64 = AtomicU64::new(0);

static HANDOFF_PREPARED: AtomicU64 = AtomicU64::new(0);
static HANDOFF_WAKEUPS: AtomicU64 = AtomicU64::new(0);
static HANDOFF_CLAIMS: AtomicU64 = AtomicU64::new(0);
static HANDOFF_DEFERRALS: AtomicU64 = AtomicU64::new(0);
static HANDOFF_ROLLBACKS: AtomicU64 = AtomicU64::new(0);
static HANDOFF_EXPIRATIONS: AtomicU64 = AtomicU64::new(0);
static HANDOFF_RESUME_CANCELS: AtomicU64 = AtomicU64::new(0);
static HANDOFF_REPLACEMENTS: AtomicU64 = AtomicU64::new(0);
static HANDOFF_PARK_FREE_OWNER: AtomicU64 = AtomicU64::new(0);

static HANDOFF_CLAIM_WAIT_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static HANDOFF_CLAIM_WAIT_MAX_NS: AtomicU64 = AtomicU64::new(0);
