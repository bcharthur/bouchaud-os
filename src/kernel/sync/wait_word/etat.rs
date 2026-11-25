// State and counters.

const WAIT_WORD_BUCKETS: usize = 64;

/// BOUCHAUD_COW_V1 -- l'identite d'un mot d'attente : `(espace, adresse)`.
///
/// * mot dans une frame PRIVEE (possedee par l'espace, partagee ou non en
///   copie-a-l'ecriture) ou pas encore residente : `(PML4, adresse
///   virtuelle)`. Deux processus issus d'un `fork` ont des PML4 distinctes :
///   leurs futex prives ne se melangent plus, et une copie-a-l'ecriture qui
///   change la frame du mot entre l'attente et le reveil ne change PAS la
///   cle. Un fils vfork partage la PML4 du pere : meme cle, comme sous Linux.
/// * mot dans une frame EMPRUNTEE (`MAP_SHARED`, cache de pages) :
///   `(0, adresse physique)`, pour que deux processus qui partagent la page
///   se reveillent mutuellement.
type CleMot = (u64, u64);

struct WaitWordEntry {
    key: CleMot,
    wait: WaitSource,
    waiters: AtomicU64,
}

impl WaitWordEntry {
    fn new(key: CleMot) -> Self {
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
