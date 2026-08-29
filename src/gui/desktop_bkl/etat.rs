// Etat et politique du BKL cooperatif desktop.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Legacy,
    Scoped,
}

/// V9 active les safe points et les scopes BKL du desktop.
pub const MODE: Mode = Mode::Scoped;

static CHECKS: AtomicU64 = AtomicU64::new(0);
static CHECKPOINTS: AtomicU64 = AtomicU64::new(0);
static SCOPES: AtomicU64 = AtomicU64::new(0);
static RELEASES: AtomicU64 = AtomicU64::new(0);
// V16: scopes explicitement locaux (present/report) peuvent suspendre un
// guard imbrique et restaurer exactement la profondeur au retour.
static NESTED_SCOPES: AtomicU64 = AtomicU64::new(0);
static MAX_SCOPE_DEPTH: AtomicU64 = AtomicU64::new(0);

static SKIP_MODE: AtomicU64 = AtomicU64::new(0);
static SKIP_NOT_DESKTOP: AtomicU64 = AtomicU64::new(0);
static SKIP_INTERRUPTS: AtomicU64 = AtomicU64::new(0);
static SKIP_NO_BKL: AtomicU64 = AtomicU64::new(0);
static SKIP_NESTED: AtomicU64 = AtomicU64::new(0);
static SKIP_RATE: AtomicU64 = AtomicU64::new(0);

static CONTENDED_RELEASES: AtomicU64 = AtomicU64::new(0);
static HANDOFF_SPINS_TOTAL: AtomicU64 = AtomicU64::new(0);

static LAST_HANDOFF_NS: AtomicU64 = AtomicU64::new(0);
static GAP_MAX_NS: AtomicU64 = AtomicU64::new(0);

static UNLOCKED_WORK_NS: AtomicU64 = AtomicU64::new(0);
static UNLOCKED_WORK_MAX_NS: AtomicU64 = AtomicU64::new(0);

static REACQUIRE_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static REACQUIRE_WAIT_MAX_NS: AtomicU64 = AtomicU64::new(0);

static RELEASE_WINDOW_NS: AtomicU64 = AtomicU64::new(0);
static RELEASE_WINDOW_MAX_NS: AtomicU64 = AtomicU64::new(0);

static SITE_RELEASES: [AtomicU64; NOMBRE_SITES] =
    [const { AtomicU64::new(0) }; NOMBRE_SITES];
static SITE_UNLOCKED_NS: [AtomicU64; NOMBRE_SITES] =
    [const { AtomicU64::new(0) }; NOMBRE_SITES];
