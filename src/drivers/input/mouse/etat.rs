// Etat visible entre IRQ12 et le bureau.
//
// Les coordonnées/boutons/molette sont atomiques : le hard IRQ publie, la GUI
// lit hors IRQ. Le décodeur de paquet (CYCLE/PKT) reste privé à IRQ12.

static MX: AtomicI32 = AtomicI32::new((WIDTH / 2) as i32);
static MY: AtomicI32 = AtomicI32::new((HEIGHT / 2) as i32);
static BTN: AtomicU8 = AtomicU8::new(0);
static HAS_WHEEL: AtomicBool = AtomicBool::new(false);
static WHEEL_DELTA: AtomicI32 = AtomicI32::new(0);

static mut CYCLE: u8 = 0;
static mut PKT: [u8; 4] = [0; 4];

// Diagnostic hard IRQ : atomiques uniquement.
const PHASE_IDLE: u8 = 0;
const PHASE_ENTER: u8 = 1;
const PHASE_READ: u8 = 2;
const PHASE_DECODE: u8 = 3;
const PHASE_PUBLISH: u8 = 4;
const PHASE_EOI: u8 = 5;
const PHASE_EXIT: u8 = 6;

static IRQ_PHASE: AtomicU8 = AtomicU8::new(PHASE_IDLE);
static IRQ_ENTRIES: AtomicU64 = AtomicU64::new(0);
static IRQ_BYTES: AtomicU64 = AtomicU64::new(0);
static IRQ_EOI: AtomicU64 = AtomicU64::new(0);
static IRQ_EXIT: AtomicU64 = AtomicU64::new(0);
static PACKETS: AtomicU64 = AtomicU64::new(0);
static PACKETS_CHANGED: AtomicU64 = AtomicU64::new(0);
static DEFERRED_SIGNALS: AtomicU64 = AtomicU64::new(0);
static LAST_IRQ_NS: AtomicU64 = AtomicU64::new(0);
static LAST_PACKET_NS: AtomicU64 = AtomicU64::new(0);
static LAST_STATUS: AtomicU8 = AtomicU8::new(0);
static LAST_BYTE: AtomicU8 = AtomicU8::new(0);
