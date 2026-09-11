// Etat visible entre IRQ12 et le bureau.
//
// Les coordonnées/boutons/molette sont atomiques : le hard IRQ publie, la GUI
// lit hors IRQ. Le décodeur de paquet (CYCLE/PKT) reste privé à IRQ12.

static MX: AtomicI32 = AtomicI32::new(0);
static MY: AtomicI32 = AtomicI32::new(0);
static BTN: AtomicU8 = AtomicU8::new(0);
static HAS_WHEEL: AtomicBool = AtomicBool::new(false);
static WHEEL_DELTA: AtomicI32 = AtomicI32::new(0);

// BOUCHAUD_SOURIS_BOUTONS_PAR_SOURCE_V1
//
// LE DEFAUT QUE CE TABLEAU CORRIGE
// --------------------------------
// `BTN` etait ecrit DIRECTEMENT par chaque rapport recu, quel que soit le
// peripherique qui l'envoyait. Une machine reelle n'a pourtant jamais une
// seule source de boutons : la machine de reference en declare quatre
// (`claviers=2 souris=3` au recomptage HID), parce qu'un recepteur sans fil
// expose une interface par peripherique associe plus la sienne.
//
// Une interface au repos qui publie « aucun bouton » EFFACAIT donc le bouton
// que l'utilisateur maintenait enfonce sur la vraie souris. Vu du bureau,
// l'etat du bouton vacillait entre 1 et 0 : chaque retour a 1 etait un
// nouveau front montant, donc un nouveau clic. Maintenir une barre de titre
// produisait des dizaines de clics, le detecteur de double-clic s'armait, et
// la fenetre basculait en plein ecran au lieu de suivre le curseur.
//
// L'etat global est desormais l'UNION de ce que chaque source tient. Un
// rapport « rien d'enfonce » ne parle plus que pour sa propre source.
/// Nombre de sources de boutons suivies separement.
pub const SOURCES_BOUTONS: usize = 12;
/// Le chemin PS/2 occupe d'office la premiere source.
pub const SOURCE_PS2: usize = 0;
static BOUTONS_PAR_SOURCE: [AtomicU8; SOURCES_BOUTONS] =
    [const { AtomicU8::new(0) }; SOURCES_BOUTONS];
/// Sources deja reservees, un bit par source. Le bit 0 est pris d'office.
static SOURCES_OCCUPEES: AtomicU32 = AtomicU32::new(1);

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
