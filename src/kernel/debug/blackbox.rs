//! Flight recorder pour le bring-up physique TRIGKEY.
//!
//! BOUCHAUD_TRIGKEY_BLACKBOX_V3
//!
//! # Ce que V3 change, et pourquoi
//!
//! V2 ecrivait PHYSIQUEMENT un enregistrement de quatre kibioctets sur la cle
//! USB a chaque appel d'`append` : prise du verrou du pilote xHCI, trois
//! transferts Bulk synchrones, deux attentes d'evenement bornees a une
//! demi-seconde chacune. Le chemin du diagnostic passait donc par le
//! peripherique meme dont il devait expliquer la defaillance.
//!
//! Les trois archives physiques disent la meme chose : l'enregistrement
//! s'arrete entre 7,3 s et 8,2 s, a l'instant ou les services du navigateur
//! commencent a lire la cle d'amorcage. Toutes les causes candidates ont ete
//! mesurees et eliminees -- tambour plein (73 enregistrements sur 8192),
//! echec d'ecriture (`bb_failures=0`), famine du verrou
//! (`equite_enregistreur_sauts=0`), support retire (jamais), ecrasement
//! (`first_seq=1`), extracteur, architecture, nombre de coeurs. Ce qui
//! restait etait la FORME du chemin.
//!
//! V3 separe les deux roles :
//!
//!   * PRODUIRE est desormais purement memoire -- deux increments atomiques,
//!     une copie, une publication. Aucune allocation, aucun verrou, aucun
//!     acces xHCI, aucune attente. Voir `kernel::bobine`.
//!   * PERSISTER n'a lieu qu'a l'extinction volontaire, par lots contigus.
//!
//! Une panne du support ne coute plus que les octets qu'on n'a pas pu poser.
//! Elle ne peut plus couter la trace elle-meme.
//!
//! # Pourquoi AUCUNE ecriture periodique, et pas « moins d'ecritures »
//!
//! C'est une experience A/B explicite, pas une optimisation. Tant qu'une
//! ecriture USB periodique subsiste sur le chemin du diagnostic, un
//! enregistrement qui s'arrete reste ambigu : est-ce l'enregistreur, ou le
//! support ? En supprimant l'ecriture pendant le runtime, un arret ne peut
//! plus venir que du producteur -- et c'est la premiere fois que ce silence
//! devient une reponse. Ne pas retablir la persistance periodique avant que
//! cette validation physique ait eu lieu.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::kernel::bobine::{self, Bobine};

pub const KIND_SERIAL: u16 = 1;
pub const KIND_SAMPLE: u16 = 2;
pub const KIND_MARKER: u16 = 3;
pub const KIND_FLIGHT: u16 = 4;
pub const KIND_MEMORY: u16 = 5;
// BOUCHAUD_NET_RELEVE_PERSISTANT_V1
//
// L'ETAT DE LA CARTE DOIT SURVIVRE A L'ARCHIVE, PAS SEULEMENT A L'ECRAN.
//
// Le releve du 17 septembre a ete extrait sans une seule occurrence de
// `rx_cur`, `chip_cmd`, `intr_status` ou `xid` : l'instantane existait dans
// `netetat`, et nulle part dans ce qui se relit apres coup. Un diagnostic qui
// ne vit que sur l'ecran de la machine ne sert a rien une fois la machine
// eteinte.
pub const KIND_NETWORK: u16 = 6;
/// Evenements de service et pics de latence.
///
/// Un genre A PART, et non des marques : ce sont les lignes qui permettent de
/// REJOUER une panne -- Ethernet pret, DHCP pret, RX degrade, ARP en echec,
/// navigation en echec -- et elles doivent se relire seules, sans etre noyees
/// dans le reste.
pub const KIND_SERVICE: u16 = 7;
/// Interactions de terminal : commande, sortie et code de retour.
/// Genre separe pour pouvoir relire `terminal.log` sans noyer la trace serie.
pub const KIND_TERMINAL: u16 = 8;
pub const KIND_FATAL: u16 = 9;

const PAYLOAD_MAX: usize = 4032;
const FLIGHT_SLOTS: usize = 8192;
const FLIGHT_EVENT_BYTES: usize = 32;
const FLIGHT_EVENTS_PER_RECORD: usize = PAYLOAD_MAX / FLIGHT_EVENT_BYTES;

// ---------------------------------------------------------------------------
// BOUCHAUD_IDENTITE_DU_BINAIRE_V1 : une archive doit dire d'ou elle sort
// ---------------------------------------------------------------------------
//
// Le releve physique du 17 septembre a ete lu comme s'il venait du commit
// qu'on croyait avoir flashe. Il venait d'un autre : ni `netetat`, ni `xid`,
// ni `chip_cmd` n'existaient dans ce binaire. Il a fallu compter les
// occurrences d'un champ dans l'archive pour s'en apercevoir, apres avoir
// cherche une panne dans du code qui n'avait jamais tourne.
//
// Deux identifiants, et les deux servent :
//
//   * `commit` vient de l'environnement de compilation quand il existe. Il est
//     exact, et absent des constructions faites a la main ;
//   * `lot` est une liste de CAPACITES compilees. Elle ne peut pas etre
//     absente, et elle repond directement a la seule question qui compte
//     devant une archive : « le correctif est-il DANS cette image ? »
pub const BUILD_COMMIT: &str = match option_env!("BOUCHAUD_BUILD_COMMIT") {
    Some(v) => v,
    None => "inconnu",
};

/// Les lots presents dans ce binaire, du plus ancien au plus recent.
///
/// Une ligne par passe. Ajouter la sienne est le prix d'entree : sans elle, le
/// prochain releve ne dira pas si le correctif y etait.
pub const BUILD_LOTS: &str = "rx-chien-de-garde-libre,dora-comptee,verdict-unique,nav-refusee,terminal-blackbox-v1,hid-running-zombie-v3,m11-tab-trace-v1";

const POLL_NS: u64 = 250_000_000;
const SAMPLE_NS: u64 = 250_000_000;
const MEMORY_NS: u64 = 1_000_000_000;
/// Periode du releve reseau. Une ligne par seconde, compacte.
const NETWORK_NS: u64 = 1_000_000_000;

const EVT_TIMER_ENTER: u16 = 1;
const EVT_TIMER_EXIT: u16 = 2;
const EVT_GFX_ENTER: u16 = 10;
const EVT_GFX_EXIT: u16 = 11;
// Un tick sur 16 est journalise dans le flight recorder. Les compteurs et
// TIMER_STAGE restent, eux, mis a jour a CHAQUE IRQ. On conserve donc le
// diagnostic d'un IRQ bloque sans transformer la cle USB en traceur 1 kHz.
const TIMER_EVENT_DIVISOR: u64 = 16;

struct FlightSlot {
    commit: AtomicU64,
    ts_ns: AtomicU64,
    meta: AtomicU64,
    arg: AtomicU64,
}

impl FlightSlot {
    const fn new() -> Self {
        Self {
            commit: AtomicU64::new(0),
            ts_ns: AtomicU64::new(0),
            meta: AtomicU64::new(0),
            arg: AtomicU64::new(0),
        }
    }
}

static FLIGHT: [FlightSlot; FLIGHT_SLOTS] = [const { FlightSlot::new() }; FLIGHT_SLOTS];
static FLIGHT_WRITE: AtomicU64 = AtomicU64::new(0);
static FLIGHT_FLUSHED: AtomicU64 = AtomicU64::new(0);

static BOOT_ID: AtomicU64 = AtomicU64::new(0);
static RECORD_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_TRACE_SEQ: AtomicUsize = AtomicUsize::new(0);

static STOPPING: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static LAST_POLL_NS: AtomicU64 = AtomicU64::new(0);
static LAST_SAMPLE_NS: AtomicU64 = AtomicU64::new(0);
static LAST_MEMORY_NS: AtomicU64 = AtomicU64::new(0);
static LAST_NETWORK_NS: AtomicU64 = AtomicU64::new(0);

const MAX_CPUS: usize = 64;
static TIMER_STAGE: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static TIMER_ENTERS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static TIMER_EXITS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

#[inline]
fn now_ns() -> u64 {
    crate::kernel::timer::monotonic_ns()
}

#[inline]
fn cpu_clamped(cpu: usize) -> usize {
    cpu.min(MAX_CPUS - 1)
}

#[inline]
fn event(kind: u16, cpu: usize, stage: u32, arg: u64) {
    let seq = FLIGHT_WRITE.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let slot = &FLIGHT[(seq as usize) % FLIGHT_SLOTS];
    let meta = (kind as u64)
        | (((cpu.min(u16::MAX as usize) as u16) as u64) << 16)
        | ((stage as u64) << 32);
    slot.ts_ns.store(now_ns(), Ordering::Relaxed);
    slot.meta.store(meta, Ordering::Relaxed);
    slot.arg.store(arg, Ordering::Relaxed);
    slot.commit.store(seq, Ordering::Release);
}

pub struct TimerIrqGuard {
    cpu: usize,
    recorded: bool,
}

impl Drop for TimerIrqGuard {
    #[inline]
    fn drop(&mut self) {
        let cpu = cpu_clamped(self.cpu);
        TIMER_STAGE[cpu].store(0, Ordering::Release);
        TIMER_EXITS[cpu].fetch_add(1, Ordering::Relaxed);
        if self.recorded {
            event(EVT_TIMER_EXIT, cpu, 0, 0);
        }
    }
}

#[inline]
pub fn timer_irq(cpu: usize, interrupted_rip: u64, interrupted_user: bool) -> TimerIrqGuard {
    let cpu = cpu_clamped(cpu);
    let count = TIMER_ENTERS[cpu].fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    TIMER_STAGE[cpu].store(1, Ordering::Release);
    let recorded = count % TIMER_EVENT_DIVISOR == 0;
    if recorded {
        event(EVT_TIMER_ENTER, cpu, if interrupted_user { 1 } else { 0 }, interrupted_rip);
    }
    TimerIrqGuard { cpu, recorded }
}

#[inline]
pub fn timer_stage(cpu: usize, stage: u32) {
    TIMER_STAGE[cpu_clamped(cpu)].store(stage as u64, Ordering::Release);
}

pub struct GfxPresentGuard {
    cpu: usize,
}

impl Drop for GfxPresentGuard {
    #[inline]
    fn drop(&mut self) {
        event(EVT_GFX_EXIT, self.cpu, 0, 0);
    }
}

#[inline]
pub fn gfx_present(x: usize, y: usize, w: usize, h: usize) -> GfxPresentGuard {
    let cpu = crate::arch::x86_64::usermode::cpu_index();
    let packed = ((x.min(0xffff) as u64) << 48)
        | ((y.min(0xffff) as u64) << 32)
        | ((w.min(0xffff) as u64) << 16)
        | (h.min(0xffff) as u64);
    event(EVT_GFX_ENTER, cpu, 0, packed);
    GfxPresentGuard { cpu }
}

fn make_boot_id() -> u64 {
    let dt = crate::arch::x86_64::rtc::now();
    let date = (dt.year as u64) * 10_000_000_000
        + (dt.month as u64) * 100_000_000
        + (dt.day as u64) * 1_000_000
        + (dt.hour as u64) * 10_000
        + (dt.minute as u64) * 100
        + dt.second as u64;
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    date.saturating_mul(1000).saturating_add(tsc % 1000).max(1)
}

fn boot_id() -> u64 {
    let current = BOOT_ID.load(Ordering::Acquire);
    if current != 0 {
        return current;
    }
    let candidate = make_boot_id();
    match BOOT_ID.compare_exchange(0, candidate, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => candidate,
        Err(value) => value,
    }
}

// ===========================================================================
// BOUCHAUD_TAMBOUR_RAM_V1 : produire et persister ne sont plus le meme geste
// ===========================================================================
//
// Le tambour vit en `.bss` : huit mebioctets et des poussieres, jamais
// alloues, jamais liberes, disponibles avant le premier `println!`. C'est
// exactement ce qu'il faut a un enregistreur qui doit survivre a la panne de
// l'allocateur autant qu'a celle du support.
//
// La discipline d'index -- reservation, publication, relecture, perte -- vit
// dans `kernel::bobine`, qui est PUR et qu'une machine hote met a l'epreuve.
// Ici il ne reste que ce qu'aucune machine hote ne peut faire : la copie.

/// Le tambour d'octets. Ecrit par plusieurs producteurs a des positions
/// DISJOINTES -- c'est `Bobine::reserve` qui le garantit --, relu par un seul
/// consommateur.
struct Tambour(UnsafeCell<[u8; bobine::OCTETS]>);

// SUR PARCE QUE LES POSITIONS SONT DISJOINTES.
//
// Deux producteurs n'obtiennent jamais la meme tranche : `reserve` decoupe
// l'anneau par `fetch_add`. Le seul chevauchement possible est celui d'un
// producteur qui rattrape un lecteur apres un tour complet, et c'est
// precisement ce que la double verification du numero, dans `bobine::lis`,
// detecte et compte.
unsafe impl Sync for Tambour {}

static TAMBOUR: Tambour = Tambour(UnsafeCell::new([0; bobine::OCTETS]));
static BOBINE: Bobine = Bobine::neuve();

/// Copie une charge utile a la place qui lui a ete reservee.
#[inline]
fn copie_dans_le_tambour(r: &bobine::Reservation, payload: &[u8]) {
    let (depart, premiere, seconde) = r.tranches();
    unsafe {
        let base = TAMBOUR.0.get() as *mut u8;
        core::ptr::copy_nonoverlapping(payload.as_ptr(), base.add(depart), premiere);
        if seconde != 0 {
            core::ptr::copy_nonoverlapping(payload.as_ptr().add(premiere), base, seconde);
        }
    }
}

/// Relit une charge utile du tambour. Rend le nombre d'octets copies.
fn lit_du_tambour(d: &bobine::Descripteur, sortie: &mut [u8]) -> usize {
    let (depart, premiere, seconde) = d.tranches();
    if sortie.len() < d.longueur {
        return 0;
    }
    unsafe {
        let base = TAMBOUR.0.get() as *const u8;
        core::ptr::copy_nonoverlapping(base.add(depart), sortie.as_mut_ptr(), premiere);
        if seconde != 0 {
            core::ptr::copy_nonoverlapping(base, sortie.as_mut_ptr().add(premiere), seconde);
        }
    }
    d.longueur
}

/// Etat du tambour RAM, pour le releve et pour l'ecran d'extinction.
pub fn tambour() -> bobine::Etat {
    BOBINE.etat()
}

/// Curseur de vidage : le prochain numero que le support n'a pas encore vu.
static PROCHAIN_A_POSER: AtomicU64 = AtomicU64::new(1);
/// Enregistrements que le vidage a trouves deja ecrases.
static VIDAGE_MANQUANTS: AtomicU64 = AtomicU64::new(0);
/// Lots physiques ecrits sur le support.
static VIDAGE_LOTS: AtomicU64 = AtomicU64::new(0);

// ===========================================================================
// BOUCHAUD_DERNIER_SOUFFLE_V1 : l'enregistreur consigne sa propre mort
// ===========================================================================
//
// L'archive du 16 septembre couvre 1,07 s a 7,30 s. La machine a tourne vingt
// minutes. Cinquante-trois enregistrements sur huit mille cent quatre-vingt-
// douze emplacements, `fatal.log` vide, aucune marque de FIN -- et, au dernier
// echantillon ecrit, `bb_failures=0 bb_busy_skips=0 bb_filets=0`.
//
// Ces trois zeros ne disent PAS que tout allait bien. Ils disent qu'a 7,047 s
// tout allait encore bien. C'est un piege circulaire : l'instrumentation ne
// peut pas rapporter sa propre mort. Cet etat-ci vit en RAM, ne depend
// d'aucun peripherique, et survit a la perte complete de la cle USB.
static SOUFFLE: crate::kernel::souffle::Souffle =
    crate::kernel::souffle::Souffle::neuf();

/// Lit l'etat de l'enregistreur sans toucher au moindre peripherique.
pub fn souffle() -> crate::kernel::souffle::Etat {
    SOUFFLE.etat(crate::kernel::timer::monotonic_ns())
}

/// Enregistre l'issue d'une tentative de pose.
fn note_souffle(ok: bool, kind: u16, seq: u64, ts_ns: u64) {
    if ok {
        SOUFFLE.succes(ts_ns);
    } else {
        SOUFFLE.echec(ts_ns, kind as u64, seq);
    }
}

/// Pose un enregistrement DANS LA MEMOIRE. Jamais sur le support.
///
/// # Ce que cette fonction ne fait plus
///
/// Elle ne prend aucun verrou, n'emet aucun transfert USB, n'attend aucun
/// evenement et ne peut pas echouer a cause d'un peripherique. Le seul echec
/// possible est une charge utile plus grande que le format ne le permet, et
/// c'est un defaut de l'appelant, pas une panne.
///
/// # Pourquoi c'est la correction, et pas une optimisation
///
/// Un diagnostic qui depend du peripherique qu'il observe ne peut pas
/// rapporter la panne de ce peripherique. Tant que `append` ecrivait sur la
/// cle, l'arret de l'archive a 7,6 s avait deux explications indiscernables
/// -- l'enregistreur, ou la cle -- et aucune mesure ne pouvait les separer.
fn append(kind: u16, payload: &[u8], ts_ns: u64, trace_end: usize) -> bool {
    let Some(reservation) = BOBINE.reserve(payload.len()) else {
        note_souffle(false, kind, 0, ts_ns);
        return false;
    };
    copie_dans_le_tambour(&reservation, payload);
    BOBINE.publie(&reservation, kind, ts_ns, trace_end as u64);
    note_souffle(true, kind, reservation.seq, ts_ns);
    true
}

struct Text {
    bytes: [u8; PAYLOAD_MAX],
    len: usize,
}

impl Text {
    const fn new() -> Self {
        Self { bytes: [0; PAYLOAD_MAX], len: 0 }
    }
    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let data = s.as_bytes();
        let room = self.bytes.len().saturating_sub(self.len);
        let n = room.min(data.len());
        self.bytes[self.len..self.len + n].copy_from_slice(&data[..n]);
        self.len += n;
        if n == data.len() { Ok(()) } else { Err(fmt::Error) }
    }
}

// ===========================================================================
// BOUCHAUD_TERMINAL_BLACKBOX_V1
// ===========================================================================
// Une commande saisie dans le terminal graphique ou texte survit au reboot
// avec sa sortie et son code de retour. Ce chemin est RAM-only.
// Les mots de passe saisis par `read_secret()` ne passent pas ici. Une cle/API
// ecrite directement dans une ligne de commande, elle, est journalisee.
pub fn terminal_commande(source: &str, cwd: &str, command: &str) {
    let ts = now_ns();
    let mut out = Text::new();
    let _ = write!(&mut out, "TERM CMD ts_ns={} source={} cwd={} command={}\n", ts, source, cwd, command);
    let _ = append(KIND_TERMINAL, out.as_bytes(), ts, crate::drivers::serial::trace_total_bytes());
}

pub fn terminal_sortie(args: fmt::Arguments<'_>) {
    let ts = now_ns();
    let mut out = Text::new();
    let _ = write!(&mut out, "TERM OUT ts_ns={} ", ts);
    let _ = out.write_fmt(args);
    let _ = out.write_str("\n");
    let _ = append(KIND_TERMINAL, out.as_bytes(), ts, crate::drivers::serial::trace_total_bytes());
}

pub fn terminal_sortie_texte(source: &str, texte: &str) {
    let ts = now_ns();
    let mut out = Text::new();
    let _ = write!(&mut out, "TERM OUT ts_ns={} source={} {}", ts, source, texte);
    let _ = out.write_str("\n");
    let _ = append(KIND_TERMINAL, out.as_bytes(), ts, crate::drivers::serial::trace_total_bytes());
}

pub fn terminal_resultat(source: &str, status: i32) {
    let ts = now_ns();
    let mut out = Text::new();
    let _ = write!(&mut out, "TERM RESULT ts_ns={} source={} status={} class={}\n", ts, source, status, if status == 0 { "ok" } else { "error" });
    let _ = append(KIND_TERMINAL, out.as_bytes(), ts, crate::drivers::serial::trace_total_bytes());
}

fn flush_flight(ts_ns: u64) {
    let end = FLIGHT_WRITE.load(Ordering::Acquire);
    let mut next = FLIGHT_FLUSHED.load(Ordering::Acquire).saturating_add(1);
    let oldest = end.saturating_sub(FLIGHT_SLOTS as u64).saturating_add(1);
    if next < oldest {
        next = oldest;
    }
    if next == 0 || next > end {
        return;
    }

    let mut payload = [0u8; PAYLOAD_MAX];
    let mut count = 0usize;
    let mut last = next.saturating_sub(1);

    while next <= end && count < FLIGHT_EVENTS_PER_RECORD {
        let slot = &FLIGHT[(next as usize) % FLIGHT_SLOTS];
        if slot.commit.load(Ordering::Acquire) != next {
            next = next.saturating_add(1);
            continue;
        }
        let ts = slot.ts_ns.load(Ordering::Relaxed);
        let meta = slot.meta.load(Ordering::Relaxed);
        let arg = slot.arg.load(Ordering::Relaxed);
        let kind = (meta & 0xffff) as u16;
        let cpu = ((meta >> 16) & 0xffff) as u16;
        let stage = (meta >> 32) as u32;

        let off = count * FLIGHT_EVENT_BYTES;
        payload[off..off + 8].copy_from_slice(&next.to_le_bytes());
        payload[off + 8..off + 16].copy_from_slice(&ts.to_le_bytes());
        payload[off + 16..off + 18].copy_from_slice(&kind.to_le_bytes());
        payload[off + 18..off + 20].copy_from_slice(&cpu.to_le_bytes());
        payload[off + 20..off + 24].copy_from_slice(&stage.to_le_bytes());
        payload[off + 24..off + 32].copy_from_slice(&arg.to_le_bytes());

        count += 1;
        last = next;
        next = next.saturating_add(1);
    }

    if count != 0
        && append(
            KIND_FLIGHT,
            &payload[..count * FLIGHT_EVENT_BYTES],
            ts_ns,
            crate::drivers::serial::trace_total_bytes(),
        )
    {
        FLIGHT_FLUSHED.store(last, Ordering::Release);
    }
}

// BOUCHAUD_DRAIN_SERIE_PAR_RETARD_V1
//
// LE DEFAUT QUE CECI CORRIGE
//
// Le drain ecrivait UN enregistrement par fenetre de scrutation : quatre mille
// trente-deux octets toutes les deux cent cinquante millisecondes, soit seize
// kilooctets par seconde, quoi qu'il y ait a poser.
//
// Ce plafond n'a jamais ete choisi en regard de ce que le noyau produit. Un
// seul releve periodique du gestionnaire de fenetres fait dix-huit mille
// quatre cent vingt-six octets : il faut plus d'une seconde pour le poser, et
// pendant cette seconde le noyau continue d'ecrire. Le retard ne se resorbe
// jamais tout seul, il attend la prochaine accalmie -- et une session qui
// n'en a pas perd son journal par le debut, silencieusement.
//
// Le nombre d'enregistrements est desormais tire du RETARD REEL, pas d'une
// constante. Le budget reste borne : au plus `RECORDS_MAX_PAR_FENETRE`, et
// l'ecriture s'arrete des que le pilote USB refuse -- c'est lui, et non un
// compteur, qui dit quand il faut rendre la main.
const RECORDS_MAX_PAR_FENETRE: usize = 32;

/// Octets de journal perdus : ecrases dans le tambour avant d'etre poses.
///
/// Zero est la seule valeur qui autorise a lire le journal comme un recit
/// complet. Tout le reste dit combien il en manque, et c'est pourquoi ce
/// chiffre appartient a l'echantillon et pas seulement a une marque isolee.
static SERIAL_PERDUS: AtomicU64 = AtomicU64::new(0);
/// Retard du drain a la derniere fenetre, en octets.
static SERIAL_RETARD: AtomicU64 = AtomicU64::new(0);

/// Perte cumulee et retard courant du journal serie.
pub fn journal_serie() -> (u64, u64, u64) {
    (
        SERIAL_PERDUS.load(Ordering::Relaxed),
        SERIAL_RETARD.load(Ordering::Relaxed),
        crate::drivers::serial::trace_total_bytes() as u64,
    )
}

fn flush_serial(ts_ns: u64, emergency: bool) {
    let (ring_start, ring_end) = crate::drivers::serial::trace_bornes();
    if ring_end == 0 {
        return;
    }

    let mut next = LAST_TRACE_SEQ.load(Ordering::Acquire);
    if next == 0 {
        next = ring_start;
    }
    if next < ring_start {
        let perdus = ring_start.saturating_sub(next);
        SERIAL_PERDUS.fetch_add(perdus as u64, Ordering::Relaxed);
        let mut gap = Text::new();
        let _ = write!(
            &mut gap,
            "BLACKBOX_SERIAL_GAP lost={} old={} new={} cumul={} capacite={}\n",
            perdus,
            next,
            ring_start,
            SERIAL_PERDUS.load(Ordering::Relaxed),
            crate::drivers::serial::trace_capacite(),
        );
        let _ = append(KIND_MARKER, gap.as_bytes(), ts_ns, ring_end);
        next = ring_start;
    }

    SERIAL_RETARD.store(ring_end.saturating_sub(next) as u64, Ordering::Relaxed);

    if emergency {
        let keep = PAYLOAD_MAX.saturating_mul(8);
        next = next.max(ring_end.saturating_sub(keep));
    }

    // Ce que le retard demande, borne par ce qu'une fenetre peut tenir.
    let attendus = ring_end
        .saturating_sub(next)
        .div_ceil(PAYLOAD_MAX)
        .max(1)
        .min(RECORDS_MAX_PAR_FENETRE);
    for _ in 0..attendus {
        if next >= ring_end {
            break;
        }
        let mut payload = [0u8; PAYLOAD_MAX];
        let n = (ring_end - next).min(PAYLOAD_MAX);
        for i in 0..n {
            payload[i] = crate::drivers::serial::trace_octet(next + i);
        }
        if !append(KIND_SERIAL, &payload[..n], ts_ns, ring_end) {
            break;
        }
        next += n;
        LAST_TRACE_SEQ.store(next, Ordering::Release);
    }
    SERIAL_RETARD.store(ring_end.saturating_sub(next.min(ring_end)) as u64, Ordering::Relaxed);
}

fn sample(ts_ns: u64) {
    let mut out = Text::new();
    let (task, syscall, phase, site, aux) = crate::kernel::task::stall_probe_local_context();
    let bkl = crate::kernel::smp_lock::health_snapshot();
    let cpu = crate::arch::x86_64::usermode::cpu_index();
    let rsp: u64;
    let here: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags));
        core::arch::asm!("lea {}, [rip + 0]", out(reg) here, options(nomem, nostack, preserves_flags));
    }
    let (t0, k0) = crate::kernel::task::rips_timer(0);
    let (t1, k1) = crate::kernel::task::rips_timer(1);
    let (t2, k2) = crate::kernel::task::rips_timer(2);
    let (t3, k3) = crate::kernel::task::rips_timer(3);
    let (polls, events, reports, kbd, mouse, hid_errors, rearms, kicks) =
        crate::drivers::xhci_active::hid_transport_stats();
    let (bb_writes, bb_failures, bb_consecutive, bb_last_error, bb_busy_skips, bb_last_ok_ns) =
        crate::drivers::xhci_active::blackbox_storage_extended_counters();
    let (hid_ecart_max, hid_dernier) = crate::drivers::xhci_active::ecart_scrutation_hid();
    let chrono = crate::drivers::xhci_active::chrono_hid();
    let tambour = BOBINE.etat();
    let verrou = crate::drivers::xhci_active::etat_du_verrou();
    let reveil = crate::kernel::scheduler::preempt::stats_reveil();
    let (perdus, retard, produit) = journal_serie();

    let _ = write!(
        &mut out,
        concat!(
            "sample ts_ns={} cpu={} rsp={:#x} here={:#x} ",
            "task={} syscall={} phase={} site={} aux={:#x} need_resched={} ",
            "bkl_owner={} bkl_cpu={} bkl_depth={} bkl_ok={} parked={:#x} resume={:#x} ",
            "timer0={:#x}/{:#x}/stage{}/{}:{} ",
            "timer1={:#x}/{:#x}/stage{}/{}:{} ",
            "timer2={:#x}/{:#x}/stage{}/{}:{} ",
            "timer3={:#x}/{:#x}/stage{}/{}:{} ",
            "hid polls={} events={} reports={} kbd={} mouse={} errors={} rearms={} kicks={} ",
            "hid_ecart_max_ms={} hid_dernier_ns={} ",
            // OU EST PASSE CE TEMPS, ET PAS SEULEMENT COMBIEN.
            //
            // Les trois intervalles se suivent : echeance -> reprise (l'
            // ordonnanceur), reprise -> verrou (le verrou), verrou -> sortie
            // (le chemin HID). Leur somme est l'ecart ci-dessus, donc aucun
            // ne peut se cacher derriere les autres.
            "hid_wake_to_run_max_us={} hid_run_to_lock_max_us={} hid_lock_starve_max_us={} ",
            "hid_poll_body_max_us={} hid_responsable={} ",
            "hid_lock_fail_total={} hid_lock_fail_streak_max={} ",
            "hid_lock_fail_owner=[{},{},{},{},{},{},{}] hid_tours={} ",
            // LE REVEIL, ET CE QU'IL A FALLU POUR L'OBTENIR.
            //
            // `hid_wake_to_run_max_us` dit COMBIEN une tache sensible a
            // attendu. Ces compteurs-ci disent POURQUOI elle ne l'a pas
            // attendu davantage : combien de fois le coeur choisi dormait
            // (`reveil_immediats`), combien de fois il a fallu couper son
            // occupant (`reveil_cibles`), et combien de ces coupes ont ete
            // REFUSEES faute de contexte sur (`reveil_refus`). Un refus qui
            // grimpe sans que l'attente bouge est normal -- le balayage de
            // quantum reessaie ; un refus qui grimpe AVEC l'attente designe
            // un verrou tenu trop longtemps, et le nomme.
            "reveil_immediats={} reveil_cibles={} reveil_differes={} reveil_en_file={} ",
            "reveil_ipi={} reveil_preempt_noyau={} reveil_refus={} reveil_deplaces={} ",
            "bb_writes={} bb_failures={} bb_consecutive={} bb_last_error={} ",
            "bb_busy_skips={} bb_filets={} bb_last_ok_ns={} ",
            // LE TAMBOUR RAM, DANS CHAQUE ECHANTILLON.
            //
            // C'est la mesure du critere d'acceptation A : `tambour_poses`
            // qui monte pendant cent vingt secondes de charge dit que
            // l'enregistreur n'a pas cesse, et `tambour_ecrases` dit
            // exactement ce qui manquera a la relecture. Les deux ensemble
            // remplacent la question « l'archive s'est-elle arretee ? » par
            // un chiffre.
            "tambour_reserves={} tambour_poses={} tambour_ecrases={} tambour_perdus={} tambour_refuses={} ",
            "tambour_octets_vifs={} ",
            // QUI TIENT LE PILOTE, ET COMBIEN DE TEMPS AU PIRE.
            //
            // « Le verrou etait pris » n'est pas un diagnostic. « Le systeme
            // de fichiers l'a tenu 480 ms » en est un.
            "verrou={} verrou_tenue_ns={} verrou_tenue_max_ns={} verrou_pire={} ",
            "verrou_prises={} verrou_contentions={} verrou_expirations={} ",
            "serial_perdus={} serial_retard={} serial_produit={} serial_capacite={} com1={} ",
            "wm_tours={} wm_entrees={} wm_trames={}\n"
        ),
        ts_ns, cpu, rsp, here,
        task, syscall, phase, site, aux,
        crate::kernel::task::besoin_de_replanifier() as u8,
        bkl.owner_token, bkl.owner_cpu, bkl.owner_depth, bkl.owner_depth_ok as u8,
        bkl.parked_mask, bkl.resume_mask,
        t0, k0, TIMER_STAGE[0].load(Ordering::Acquire),
        TIMER_ENTERS[0].load(Ordering::Relaxed), TIMER_EXITS[0].load(Ordering::Relaxed),
        t1, k1, TIMER_STAGE[1].load(Ordering::Acquire),
        TIMER_ENTERS[1].load(Ordering::Relaxed), TIMER_EXITS[1].load(Ordering::Relaxed),
        t2, k2, TIMER_STAGE[2].load(Ordering::Acquire),
        TIMER_ENTERS[2].load(Ordering::Relaxed), TIMER_EXITS[2].load(Ordering::Relaxed),
        t3, k3, TIMER_STAGE[3].load(Ordering::Acquire),
        TIMER_ENTERS[3].load(Ordering::Relaxed), TIMER_EXITS[3].load(Ordering::Relaxed),
        polls, events, reports, kbd, mouse, hid_errors, rearms, kicks,
        hid_ecart_max / 1_000_000, hid_dernier,
        chrono.wake_to_run_max_us, chrono.run_to_lock_max_us,
        chrono.lock_starve_max_us, chrono.poll_body_max_us, chrono.responsable(),
        chrono.lock_fail_total, chrono.lock_fail_streak_max,
        chrono.lock_fail_owner[0], chrono.lock_fail_owner[1],
        chrono.lock_fail_owner[2], chrono.lock_fail_owner[3],
        chrono.lock_fail_owner[4], chrono.lock_fail_owner[5],
        chrono.lock_fail_owner[6], chrono.tours,
        reveil.immediats, reveil.cibles, reveil.differes, reveil.en_file,
        reveil.ipi_envoyes, reveil.preemptions_noyau, reveil.preemptions_noyau_refusees,
        reveil.placements_deplaces,
        bb_writes, bb_failures, bb_consecutive, bb_last_error,
        bb_busy_skips, FILETS.load(Ordering::Relaxed), bb_last_ok_ns,
        tambour.reserves, tambour.poses, tambour.ecrases, tambour.perdus, tambour.refuses,
        tambour.octets_vifs,
        verrou.proprietaire.nom(), verrou.tenue_courante_ns,
        verrou.tenue_max_ns, verrou.tenue_max_proprietaire.nom(),
        verrou.prises, verrou.contentions, verrou.expirations,
        perdus, retard, produit, crate::drivers::serial::trace_capacite(),
        crate::drivers::serial::presence_com1().nom(),
        crate::gui::reveil::tours(), crate::gui::reveil::entrees(),
        crate::gui::reveil::trames_composees(),
    );
    let _ = append(KIND_SAMPLE, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
    echantillon_par_cpu(ts_ns);
    etat_systeme(ts_ns);
}

// ===========================================================================
// BOUCHAUD_ETAT_SYSTEME_V1 : ce qui marche, ce qui ne marche pas, et a quel prix
// ===========================================================================
//
// L'echantillon precedent decrit le NOYAU : piles, verrous, timers, HID. Il
// ne dit rien de ce qu'on demande d'abord a une archive quand la machine a
// mal tourne -- le lien Ethernet etait-il monte ? le bail etait-il la ? a
// combien de trames par seconde tournait le bureau ? quel a ete le pire
// a-coup ? le disque repondait-il ?
//
// Tout cela existait dans le noyau, dispersé dans des compteurs qu'aucun
// enregistrement ne portait. Le releve du 16 septembre a ete lu sans, et il a
// fallu recouper la console serie pour retrouver la montee du lien.
//
// UNE LIGNE SEPAREE, ET LE MEME GENRE D'ENREGISTREMENT.
//
// Le genre reste `KIND_SAMPLE` : l'extracteur qui produit `samples.log` le
// connait deja, et un genre neuf verrait sa charge utile perdue -- il
// n'apparaitrait que comme un numero dans `records.json`. La ligne est
// separee pour ne pas approcher `PAYLOAD_MAX` et pour que les outils qui
// lisent `sample ` gardent leur format, exactement comme `cpus `.
fn etat_systeme(ts_ns: u64) {
    let mut out = Text::new();

    let demarrage = match crate::net::etat_demarrage() {
        crate::net::Demarrage::SansCarte => "sans-carte",
        crate::net::Demarrage::CarteRefusee => "carte-refusee",
        crate::net::Demarrage::LienBas => "lien-bas",
        crate::net::Demarrage::SansBail => "sans-bail",
        crate::net::Demarrage::SansConfiguration => "sans-configuration",
        crate::net::Demarrage::Pret => "pret",
    };
    let ip = crate::net::our_ip();
    let gw = crate::net::gateway();
    let dns = crate::net::dns_server();

    let trames = crate::gui::frame_clock::snapshot();
    let (_, _, bulk_transferts, bulk_octets, bulk_stalls, _, _, bulk_echecs, _, bulk_occupes) =
        crate::drivers::xhci_active::stockage_stats();
    let (hid_sauts, hid_cessions, enr_sauts, enr_cessions) =
        crate::drivers::xhci_active::equite_stats();
    let bot = crate::drivers::xhci_active::releve_bot();
    let (stalls_bb, reculs_bb, lot_bb) = crate::drivers::xhci_active::blackbox_lot_stats();

    let _ = write!(
        &mut out,
        concat!(
            // L'HEURE MURALE SE RECONSTRUIT : `boot_id` de la marque START
            // porte la date du demarrage (AAAAMMJJhhmmssmmm), et `ts_ns` le
            // temps ecoule depuis. Les deux ensemble datent chaque ligne sans
            // lire la RTC quatre fois par seconde -- sa lecture attend la fin
            // d'une mise a jour CMOS, et ce n'est pas une attente qu'on veut
            // dans le chemin de diagnostic.
            "etat ts_ns={} ",
            "reseau={} lien={} externe={} ip={}.{}.{}.{} gw={}.{}.{}.{} dns={}.{}.{}.{} ",
            "trames_actives={} fps={} trames_utiles={} ecart_max_ms={} depuis_trame_ms={} ",
            "disque_transferts={} disque_octets={} disque_stalls={} disque_echecs={} disque_occupe={} ",
            "equite_hid_sauts={} equite_hid_cessions={} ",
            "equite_enregistreur_sauts={} equite_enregistreur_cessions={} ",
            // L'ETAT DU TRANSPORT DE MASSE, PAS SEULEMENT SES ECHECS.
            //
            // Un support hors service et un support qu'on n'a jamais
            // sollicite produisaient le meme silence. `bot=` les separe, et
            // `bot_refus=` dit combien d'appelants ont ete econduits.
            "bot={} bot_phase={} bot_echeances={} bot_reprises={} ",
            "bot_reprises_reussies={} bot_reprises_echouees={} ",
            "bot_refus={} bot_slot={} bot_dci={} ",
            // CE QUE LA CLEF A ACCEPTE, ET CE QU'ELLE A REFUSE.
            //
            // `bb_stalls` compte les phases de donnees que la clef a arretees
            // -- prevu par le protocole, et sans rapport avec un transport
            // casse. `bb_lot` dit la taille qu'elle tolere reellement, et
            // `bb_reculs` combien de fois il a fallu la reduire. Aucune
            // constante ne dirait cela.
            "bb_stalls={} bb_reculs={} bb_lot={}\n"
        ),
        ts_ns,
        demarrage,
        crate::net::connecte() as u8,
        crate::net::external_enabled() as u8,
        ip[0], ip[1], ip[2], ip[3],
        gw[0], gw[1], gw[2], gw[3],
        dns[0], dns[1], dns[2], dns[3],
        trames.active as u8,
        trames.fps_arrondi(),
        trames.frames_useful,
        // LE PIRE A-COUP, PAS LA MOYENNE. Une moyenne de soixante trames par
        // seconde avec un trou d'une seconde et demie se lit « fluide » ;
        // c'est pourtant le trou qu'on voit a l'ecran.
        trames.useful_gap_max_ms,
        trames.since_useful_ms,
        bulk_transferts, bulk_octets, bulk_stalls, bulk_echecs, bulk_occupes,
        hid_sauts, hid_cessions, enr_sauts, enr_cessions,
        bot.etat.nom(), bot.derniere_phase.nom(), bot.echeances, bot.reprises,
        bot.reprises_reussies, bot.reprises_echouees,
        bot.refus, bot.dernier_slot, bot.dernier_dci,
        stalls_bb, reculs_bb, lot_bb,
    );
    let _ = append(KIND_SAMPLE, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
}

// BOUCHAUD_ECHANTILLON_TOUS_LES_COEURS_V1
//
// L'echantillon ci-dessus decrit quatre coeurs. La TRIGKEY en a seize, et
// c'est justement sur les douze autres que se logent les figements qu'on
// cherche : un timer qui ne revient pas, une tache prete sur un coeur que
// personne ne regarde. Un releve qui s'arrete a `timer3` ne peut pas les
// voir, et son silence ressemble a une machine saine.
//
// Ce releve-ci suit `schedulable_cpus()`. Il est pose separement pour que la
// ligne d'echantillon garde sa forme -- les outils d'analyse existants la
// lisent -- et pour qu'aucun des deux ne puisse depasser la charge utile.
//
// BOUCHAUD_ENTERS_NE_MESURE_QUE_LE_BSP_V1
//
// `enters`/`exits` viennent de `TIMER_ENTERS`, alimente par le SEUL handler
// d'IRQ0 -- le PIT, que seul le processeur d'amorcage recoit. Sur les quinze
// autres coeurs ils valent donc zero en permanence, y compris quand ces
// coeurs travaillent : ils battent au timer LAPIC local, qui passe par un
// autre vecteur.
//
// Lu sans le savoir, l'echantillon du 16 septembre ressemblait trait pour
// trait a quinze coeurs morts -- `en_ligne=1 rip_u=0x0 enters=0` --, et c'est
// exactement la lecture qui en a ete faite. Le meme demarrage imprimait
// pourtant `BOUCHAUD_SMP_BATTEMENT cpu=0..15 bat=1`.
//
// `quantums` est ce compteur-la : `STALL_IPI_COUNT`, incremente par le
// handler de replanification avec le numero de coeur REEL. C'est la seule
// valeur de cette ligne qui distingue un coeur en ligne d'un coeur qui
// PARTICIPE, et elle voyage desormais avec les deux autres pour qu'aucune
// lecture ne puisse plus conclure de leur zero.
fn echantillon_par_cpu(ts_ns: u64) {
    let cpus = crate::arch::x86_64::smp::schedulable_cpus().min(MAX_CPUS);
    let mut cpu = 0usize;
    while cpu < cpus {
        let mut out = Text::new();
        let _ = write!(&mut out, "cpus ts_ns={}", ts_ns);
        let mut poses = 0usize;
        // Huit coeurs par enregistrement : une ligne reste lisible, et la
        // charge utile ne peut pas deborder quels que soient les RIP.
        while cpu < cpus && poses < 8 {
            let (utilisateur, noyau) = crate::kernel::task::rips_timer(cpu);
            let _ = write!(
                &mut out,
                " cpu{}=[en_ligne={} rip_u={:#x} rip_k={:#x} stage={} enters={} exits={} quantums={}]",
                cpu,
                crate::arch::x86_64::smp::is_online(cpu) as u8,
                utilisateur,
                noyau,
                TIMER_STAGE[cpu].load(Ordering::Acquire),
                TIMER_ENTERS[cpu].load(Ordering::Relaxed),
                TIMER_EXITS[cpu].load(Ordering::Relaxed),
                crate::kernel::task::quantums_recus(cpu),
            );
            cpu += 1;
            poses += 1;
        }
        let _ = write!(&mut out, "\n");
        let _ = append(
            KIND_SAMPLE,
            out.as_bytes(),
            ts_ns,
            crate::drivers::serial::trace_total_bytes(),
        );
    }
}

fn memory_sample(ts_ns: u64) {
    let d = crate::kernel::dalles_tas::stats();
    let (frames_used, frames_total) = crate::kernel::vmm::frame_stats_relaxed();
    // LE TAS, SUIVI DANS LE TEMPS.
    //
    // La panique du 17 septembre est une allocation refusee. Ce releve
    // decrivait les dalles et les frames physiques, jamais l'arene : on ne
    // pouvait donc pas voir venir sa saturation, ni meme dire apres coup s'il
    // en restait.
    let (heap_utilise, heap_libre, heap_total) = crate::kernel::heap::stats();
    let (_, _, _, _, pages_refus, _) = crate::kernel::pages_tas::stats();
    let plus_grand_contigu = crate::kernel::pages_tas::plus_grand_contigu();
    let mut out = Text::new();
    let _ = write!(
        &mut out,
        concat!(
            "memory ts_ns={} frames_used={} frames_total={} ",
            "heap_utilise={} heap_libre={} heap_total={} ",
            // CE QUE `heap_libre` NE DIT PAS.
            //
            // La panique du 17 septembre a refuse 4,38 Mio avec 4,58 Gio
            // libres : le tas avait la place, l'allocateur de pages ne savait
            // pas la servir d'un seul tenant. `pages_refus` compte ces refus,
            // et `plus_grand_contigu` dit la plus grande demande encore
            // servable -- c'est elle qui echoue en premier, pas le total.
            "pages_refus={} plus_grand_contigu={} ",
            "dalles={} vides={} vivants={} candidats={} manques={} saturations={} ",
            "sous_flux={} surallocations={} max_probe={}\n"
        ),
        ts_ns,
        frames_used,
        frames_total,
        heap_utilise,
        heap_libre,
        heap_total,
        pages_refus,
        plus_grand_contigu,
        d.enregistrees,
        d.dalles_vides,
        d.objets_vivants,
        d.candidats_vides,
        d.manques,
        d.saturations,
        d.sous_flux,
        d.surallocations,
        d.max_probe,
    );
    let _ = append(KIND_MEMORY, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
}

/// L'etat du controleur reseau, une ligne par seconde.
///
/// # Ce que cette ligne repond, et que rien ne repondait
///
/// `trames=104 puis plus rien` ne permet pas de choisir entre quatre pannes :
/// moteur arrete, anneau sature, descripteurs desynchronises, carte disparue
/// du bus. Chacune se lit ici, et le releve physique n'a plus a etre devine :
///
///   * `chip_cmd` porte `RxEnb` et `RxBufEmpty` -- moteur arrete ou non ;
///   * `desc_nic`/`desc_cpu` disent qui possede l'anneau -- sature ou non ;
///   * `invariant` dit si le materiel et nous parlons encore du meme objet ;
///   * `intr_status` a `0xffff` dit une carte absente du bus.
// Un releve par endpoint HID, a 1 Hz avec le releve reseau.
fn hid_points_sample(ts_ns: u64) {
    crate::drivers::xhci_active::pour_chaque_point_hid(|ep| {
        let mut out = Text::new();
        let _ = write!(
            &mut out,
            concat!(
                "hid_ep ts_ns={} slot={} dci={} kind={} interface={} proto={} ",
                "state={} dequeue={:#x} expected={:#x} events={} silence_ms={} ",
                "reprises={} broken={} sentinel={} sentinel_activity_ms={} ",
                "since_recovery_ms={} quarantine={} fallback_errors={}\n"
            ),
            ts_ns, ep.slot, ep.dci, ep.genre, ep.interface, ep.protocole,
            ep.etat_contexte, ep.defilement, ep.trb_attendu, ep.evenements,
            ep.silence_ms, ep.reprises_silence, ep.interrupt_casse as u8,
            ep.sentinelle_ep0 as u8, ep.sentinelle_activite_ms,
            ep.depuis_reprise_ms, ep.en_quarantaine as u8, ep.echecs_repli,
        );
        let _ = append(KIND_SAMPLE, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
    });
}

fn network_sample(ts_ns: u64) {
    let nic = crate::drivers::rtl8168::releve();
    let (routees, arp_vues, dhcp_vues, arp_ok, arp_ko, arp_non_emis) =
        crate::net::compteurs_routage();
    let smol = crate::net::compteurs_smoltcp();
    let dora = crate::net::application::dhcp::compteurs();
    let dns53 = crate::net::sonde_dns::compte;
    let mut out = Text::new();
    let _ = write!(
        &mut out,
        concat!(
            "nic ts_ns={} xid={:#05x} gen={} lien={} vitesse_mbps={} duplex={} ",
            "rx_cur={} tx_cur={} rx_packets={} rx_bytes={} tx_packets={} tx_bytes={} ",
            "rx_last_ns={} tx_last_ns={} desc_nic={} desc_cpu={} desc_courant={:#010x} ",
            "chip_cmd={:#04x} intr_status={:#06x} rx_missed={} ",
            "isr_lectures={} isr_rx_ok={} isr_rx_err={} isr_rx_overflow={} ",
            "isr_rx_fifo_over={} isr_tx_err={} isr_link_chg={} isr_sys_err={} isr_absente={} ",
            "rearms={} recoveries={} recovery_failures={} abandonnees={} abimees={} tx_plein={} ",
            "invariant={} ",
            // LE ROUTAGE, SUR LA MEME LIGNE QUE LA CARTE.
            //
            // Une carte qui recoit et un routage fige designent un tout autre
            // defaut qu'une carte muette. Les separer obligeait a recoller deux
            // lignes de deux sources a la main.
            "routees={} arp_vues={} dhcp_vues={} arp_ok={} arp_ko={} arp_non_emis={} ",
            // LA FILE SMOLTCP : qui consomme, et ce qui se perd.
            "smol_abonnee={} smol_posees={} smol_retirees={} smol_perdues={} smol_max={} ",
            // LA PROGRESSION MATERIELLE DE L'ANNEAU RX.
            //
            // `isr_rx_ok` qui monte pendant que `rx_packets` reste fige ne
            // permettait pas de trancher : la carte annonce-t-elle des trames
            // qu'elle n'ecrit pas, ou lisons-nous un anneau qu'elle a quitte ?
            // `rx_ok_sans_desc` repond -- c'est le `RxOK` vu alors que le
            // descripteur courant porte encore `OWN`.
            "rx_hw_head={} rx_last_desc_cpu={} own_rendus={} rx_ok_sans_desc={} ",
            "rx_stall={} repair_req={} repair_exec={} repair_degre={} ",
            // L'EMISSION PROUVEE : enfile n'est pas parti.
            "tx_enqueued={} tx_completed={} tx_ok_isr={} tx_last_complete_ns={} tx_desc_owned={} ",
            // DORA, SANS UN OCTET DE PAQUET.
            //
            // Onze emissions de 342 octets et zero reponse ne disaient pas OU
            // la negociation s'arretait : pas d'offre, offre au mauvais xid,
            // ou REQUEST sans ACK. Trois pannes, trois enquetes.
            "discover_sent={} offer_seen={} request_sent={} ack_seen={} ",
            "last_xid={:#010x} dhcp_retry={} dhcp_stage={} ",
            // LES TOURS D'ANNEAU. Un premier tour reussi ne prouve rien : le
            // releve du 18 septembre en montre exactement un, puis plus rien.
            "rx_ring_laps_cpu={} rx_desc_returned_lap1={} rx_desc_returned_lap2={} ",
            "rx_desc_rearmed_lap1={} rx_desc_reused_lap2={} rx_ok_without_progress={} ",
            // PRESENCE, ATTACHEMENT, SERVICE : trois faits, pas un booleen.
            "nic_present={} nic_bound={} nic_state={} nic_resets={} nic_resets_ok={} ",
            // L'ECHELLE DE LA REPONSE DNS. Le premier barreau nul dont le
            // predecesseur ne l'est pas nomme l'etage qui laisse tomber le
            // datagramme -- et `dns53_verdict` le dit en toutes lettres.
            "dns53_rx_ethernet={} dns53_rx_ipv4={} dns53_rx_udp={} ",
            "dns53_queued_ip={} dns53_dequeued_ip={} dns53_socket_match={} ",
            "dns53_socket_busy={} dns53_socket_delivered={} dns53_poll_ready={} ",
            "dns53_recv_success={} dns53_recv_eagain={} dns53_verdict={}\n"
        ),
        ts_ns, nic.xid, nic.generation,
        crate::drivers::e1000::link_up() as u8,
        crate::drivers::e1000::vitesse_mbps(),
        crate::drivers::e1000::duplex_complet() as u8,
        nic.rx_cur, nic.tx_cur,
        nic.rx_paquets, nic.rx_octets, nic.tx_paquets, nic.tx_octets,
        nic.rx_dernier_ns, nic.tx_dernier_ns,
        nic.rx_desc_materiel, nic.rx_desc_processeur, nic.rx_desc_courant,
        nic.chip_cmd, nic.intr_status, nic.rx_missed,
        nic.isr_lectures, nic.isr_rx_ok, nic.isr_rx_err, nic.isr_rx_overflow,
        nic.isr_rx_fifo_over, nic.isr_tx_err, nic.isr_link_chg,
        nic.isr_system_error, nic.isr_carte_absente,
        nic.rx_rearmements, nic.rx_reprises, nic.rx_reprises_echouees,
        nic.rx_abandonnees, nic.rx_abimees, nic.tx_anneau_plein,
        nic.invariant.unwrap_or("intact"),
        routees, arp_vues, dhcp_vues, arp_ok, arp_ko, arp_non_emis,
        crate::net::smoltcp_abonnee() as u8,
        smol.posees, smol.retirees, smol.perdues_pleine, smol.occupation_max,
        nic.rx_tete_cpu, nic.rx_dernier_desc_cpu, nic.rx_own_rendus,
        nic.rx_ok_sans_descripteur, nic.rx_arret_detecte,
        nic.reparations_demandees, nic.reparations_executees, nic.reparation_degre,
        nic.tx_enfiles, nic.tx_termines, nic.tx_ok_isr,
        nic.tx_dernier_termine_ns, nic.tx_desc_possedes,
        dora.discover_envoyes, dora.offres_vues, dora.requests_envoyes, dora.acks_vus,
        dora.dernier_xid, dora.tentatives, dora.derniere_etape.nom(),
        nic.rx_tours_cpu, nic.rx_rendus_tour1, nic.rx_rendus_tour2,
        nic.rx_rearmes_tour1, nic.rx_reutilises_tour2, nic.rx_ok_sans_progres,
        crate::drivers::rtl8168::presente() as u8,
        crate::drivers::rtl8168::attache() as u8,
        crate::drivers::rtl8168::etat_pilote().nom(),
        nic.reinitialisations, nic.reinitialisations_ok,
        dns53(crate::net::sonde_dns::Barreau::RxEthernet),
        dns53(crate::net::sonde_dns::Barreau::RxIpv4),
        dns53(crate::net::sonde_dns::Barreau::RxUdp),
        dns53(crate::net::sonde_dns::Barreau::MisEnFile),
        dns53(crate::net::sonde_dns::Barreau::SortiDeFile),
        dns53(crate::net::sonde_dns::Barreau::SocketTrouve),
        dns53(crate::net::sonde_dns::Barreau::SocketOccupe),
        dns53(crate::net::sonde_dns::Barreau::SocketLivre),
        dns53(crate::net::sonde_dns::Barreau::PollPret),
        dns53(crate::net::sonde_dns::Barreau::RecvSucces),
        dns53(crate::net::sonde_dns::Barreau::RecvVide),
        crate::net::sonde_dns::verdict(),
    );
    let _ = append(KIND_NETWORK, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
    hid_points_sample(ts_ns);
}

/// Un changement d'etat de service. Emis SEULEMENT quand l'etat change.
///
/// Un service qui reste actif sans erreur pendant cinq minutes n'ecrit rien :
/// c'est ce qui evite que l'observatoire refasse ce que le diagnostic
/// d'ordonnancement a fait a l'archive du 17 septembre -- effacer le
/// demarrage pour decrire un systeme qui va bien.
pub fn service_evenement(
    ts_ns: u64,
    id: &str,
    etat: &str,
    duree_ms: u64,
    erreurs: u32,
    reprises: u32,
    raison: &str,
) {
    let mut out = Text::new();
    let _ = write!(
        &mut out,
        "service ts_ns={} id={} etat={} duree_ms={} erreurs={} reprises={} raison={}\n",
        ts_ns, id, etat, duree_ms, erreurs, reprises, raison,
    );
    let _ = append(KIND_SERVICE, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
}

/// Un reveil qui a trop attendu son processeur.
///
/// # Ce que cet enregistrement ajoute a `hid_wake_to_run_max_us`
///
/// Le maximum cumule disait `12 179 860` us et ne datait rien : ni quand, ni
/// pendant quelle phase, ni sur quel coeur. Celui-ci repond aux trois, et il
/// ne sort qu'aux pics -- pas a chaque reveil.
pub fn pic_reveil(
    ts_ns: u64,
    echeance_ns: u64,
    delta_us: u64,
    cpu: u32,
    file: u32,
    noyau: bool,
    bkl: u32,
    phase: &str,
) {
    let mut out = Text::new();
    let _ = write!(
        &mut out,
        "pic_reveil ts_ns={} echeance_ns={} delta_us={} cpu={} runqueue={} \
tache_noyau={} bkl_owner={} phase={}\n",
        ts_ns, echeance_ns, delta_us, cpu, file, noyau as u8, bkl, phase,
    );
    let _ = append(KIND_SERVICE, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
    crate::serial_println!(
        "HID_LATENCY_SPIKE delta_us={} cpu={} runqueue={} tache_noyau={} bkl_owner={} phase={}",
        delta_us, cpu, file, noyau as u8, bkl, phase,
    );
}

/// Produit ce qu'il y a a produire. Rend toujours `false`.
///
/// # Ce que cette fonction ne fait plus
///
/// Elle ne consulte plus `blackbox_storage_ready()`, ne vide plus le journal
/// serie, ne vide plus l'enregistreur de vol et n'ecrit plus une seule fois
/// sur la cle. Elle pose des echantillons EN MEMOIRE, et rien d'autre.
///
/// Le journal serie et les evenements de vol vivent deja, chacun, dans leur
/// propre anneau RAM. Les recopier dans le tambour pendant la session aurait
/// coute -- pour rien -- la place des echantillons, qui sont les seuls dont
/// personne d'autre ne garde de copie. Ils sont convertis en enregistrements
/// a l'extinction, quand la place n'est plus disputee.
///
/// Le `bool` de retour disait « le pilote USB m'a fait renoncer, reessaie
/// vite ». Plus rien ne peut faire renoncer cette fonction ; il reste `false`
/// pour ne pas casser `fil_blackbox`, dont la cadence n'a plus de raison de
/// varier.
pub fn poll() -> bool {
    if STOPPING.load(Ordering::Acquire) {
        return false;
    }
    let now = now_ns();
    let previous = LAST_POLL_NS.load(Ordering::Relaxed);
    if previous != 0 && now.saturating_sub(previous) < POLL_NS {
        return false;
    }
    if LAST_POLL_NS
        .compare_exchange(previous, now, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }

    if !STARTED.swap(true, Ordering::AcqRel) {
        // CE QUE LE DEMARRAGE A DEJA COUTE AVANT LA PREMIERE MARQUE.
        //
        // `arriere=` dit combien d'octets de journal attendaient a cette
        // seconde-la, et `capacite=` ce que le tambour serie peut retenir.
        // Tant que le premier reste sous le second, aucune ligne de demarrage
        // n'a ete perdue -- et c'est une chose qui se LIT, au lieu de se
        // supposer.
        //
        // La marque part maintenant sans attendre la cle : c'est tout
        // l'interet du tambour RAM. Sur les trois archives physiques, elle
        // n'etait posee qu'une fois la cle enumeree, et tout ce qui precedait
        // n'existait que si le tambour serie le retenait encore.
        let (debut_tambour, fin_tambour) = crate::drivers::serial::trace_bornes();
        let mut msg = Text::new();
        let _ = write!(
            &mut msg,
            "BOUCHAUD_TRIGKEY_BLACKBOX_V3 START boot_id={} ts_ns={} BOUCHAUD_BUILD commit={} lot={} arriere={} produit={} capacite={} perdu_avant_demarrage={} com1={} tambour_descripteurs={} tambour_octets={}\n",
            boot_id(),
            now,
            BUILD_COMMIT,
            BUILD_LOTS,
            fin_tambour.saturating_sub(debut_tambour),
            fin_tambour,
            crate::drivers::serial::trace_capacite(),
            fin_tambour.saturating_sub(fin_tambour.min(crate::drivers::serial::trace_capacite())),
            crate::drivers::serial::presence_com1().nom(),
            bobine::DESCRIPTEURS,
            bobine::OCTETS,
        );
        if !append(KIND_MARKER, msg.as_bytes(), now, crate::drivers::serial::trace_total_bytes()) {
            STARTED.store(false, Ordering::Release);
        }
    }

    let last_sample = LAST_SAMPLE_NS.load(Ordering::Relaxed);
    if last_sample == 0 || now.saturating_sub(last_sample) >= SAMPLE_NS {
        LAST_SAMPLE_NS.store(now, Ordering::Relaxed);
        sample(now);
    }

    let last_memory = LAST_MEMORY_NS.load(Ordering::Relaxed);
    if last_memory == 0 || now.saturating_sub(last_memory) >= MEMORY_NS {
        LAST_MEMORY_NS.store(now, Ordering::Relaxed);
        memory_sample(now);
    }

    let last_network = LAST_NETWORK_NS.load(Ordering::Relaxed);
    if last_network == 0 || now.saturating_sub(last_network) >= NETWORK_NS {
        LAST_NETWORK_NS.store(now, Ordering::Relaxed);
        network_sample(now);
    }

    false
}

// ===========================================================================
// BOUCHAUD_VIDAGE_PAR_LOTS_V1 : la cle n'est sollicitee qu'a l'extinction
// ===========================================================================
//
// # Pourquoi par lots
//
// Une commande BOT, c'est trois transferts Bulk et deux attentes d'evenement,
// quelle que soit la quantite de donnees. Ecrire un enregistrement de quatre
// kibioctets par commande paie donc le protocole seize fois plus cher que
// necessaire : le tampon DMA de l'enregistreur en fait soixante-quatre.
//
// Au vidage final il peut y avoir deux mille enregistrements a poser. A une
// commande chacun, c'est deux mille allers-retours -- et l'ecran d'extinction
// reste plusieurs secondes sur « Enregistrement des journaux ». Par lots de
// seize, c'est cent vingt-cinq.
//
// # Le seul decoupage impose
//
// L'emplacement d'un enregistrement sur le support est `numero % places`. Des
// numeros consecutifs donnent donc des emplacements consecutifs, SAUF au tour
// de l'anneau du support. Le lot se coupe la, et seulement la.

/// Enregistrements par lot : le tampon DMA de l'enregistreur en tient
/// soixante-quatre kibioctets, soit seize enregistrements de quatre.
pub const ENREGISTREMENTS_PAR_LOT: usize = 16;

/// Ce qu'un vidage a donne.
#[derive(Clone, Copy, Default)]
pub struct Bilan {
    /// Enregistrements effectivement poses sur le support.
    pub poses: u64,
    /// Enregistrements que le tambour avait deja ecrases.
    pub manquants: u64,
    /// Reste-t-il quelque chose a poser ?
    pub reste: bool,
}

/// Vide le tambour vers le support, par lots contigus.
///
/// Rend le bilan. N'ecrit rien et rend un bilan vide si le support n'est pas
/// la : une machine sans cle blackbox produit une trace RAM parfaitement
/// valide, simplement non persistee -- et c'est exactement ce qu'on veut
/// pouvoir dire.
pub fn vidange(echeance_ns: u64) -> Bilan {
    let mut bilan = Bilan::default();
    if !crate::drivers::xhci_active::blackbox_storage_ready() {
        bilan.reste = PROCHAIN_A_POSER.load(Ordering::Acquire) <= BOBINE.dernier();
        return bilan;
    }

    loop {
        if now_ns() >= echeance_ns {
            break;
        }
        let debut_lot = PROCHAIN_A_POSER.load(Ordering::Acquire).max(BOBINE.plus_ancien());
        let dernier = BOBINE.dernier();
        if debut_lot > dernier {
            break;
        }

        // LE CURSEUR N'AVANCE QUE SUR CE QUI A ETE POSE.
        //
        // La premiere version avancait `PROCHAIN_A_POSER` sur tout ce que le
        // rappel avait DISTRIBUE, y compris quand le pilote n'ecrivait rien.
        // Un seul lot rate coutait donc seize enregistrements, silencieusement
        // -- et le banc l'a montre : mille quatre cent cinquante-cinq
        // enregistrements en memoire, mille cent trente-six sur la cle, et pas
        // un mot sur les trois cent dix-neuf manquants.
        //
        // Les numeros distribues sont retenus ; le curseur se pose sur le
        // DERNIER REELLEMENT ECRIT, et pas un de plus.
        let mut suivant = debut_lot;
        let mut manquants = 0u64;
        let mut distribues = [0u64; ENREGISTREMENTS_PAR_LOT];
        let mut combien = 0usize;
        let poses = crate::drivers::xhci_active::blackbox_vidange_lot(
            ENREGISTREMENTS_PAR_LOT,
            |zone| {
                while suivant <= dernier {
                    let seq = suivant;
                    suivant += 1;
                    let Some(d) = BOBINE.lis(seq) else {
                        manquants += 1;
                        continue;
                    };
                    let mut charge = [0u8; PAYLOAD_MAX];
                    let n = lit_du_tambour(&d, &mut charge);
                    if n != d.longueur {
                        manquants += 1;
                        continue;
                    }
                    if !encadre(zone, &d, &charge[..n]) {
                        manquants += 1;
                        continue;
                    }
                    if combien < distribues.len() {
                        distribues[combien] = seq;
                        combien += 1;
                    }
                    return Some(seq);
                }
                None
            },
        );

        if poses == 0 {
            if combien == 0 {
                // Le rappel n'a rien eu a distribuer : il ne reste que des
                // enregistrements ecrases. Le curseur peut avancer, sinon on
                // retenterait les memes indefiniment.
                VIDAGE_MANQUANTS.fetch_add(manquants, Ordering::Relaxed);
                bilan.manquants += manquants;
                PROCHAIN_A_POSER.store(suivant, Ordering::Release);
            }
            // Le pilote n'a rien ecrit : le curseur NE BOUGE PAS, et ces
            // enregistrements repasseront au tour suivant.
            break;
        }

        VIDAGE_MANQUANTS.fetch_add(manquants, Ordering::Relaxed);
        bilan.manquants += manquants;
        PROCHAIN_A_POSER.store(
            distribues[poses.min(combien) - 1].saturating_add(1),
            Ordering::Release,
        );
        VIDAGE_LOTS.fetch_add(1, Ordering::Relaxed);
        BOBINE.note_vidange(poses as u64, distribues[poses.min(combien) - 1]);
        bilan.poses += poses as u64;
    }

    bilan.reste = PROCHAIN_A_POSER.load(Ordering::Acquire) <= BOBINE.dernier();
    bilan
}

/// Ecrit l'en-tete du format persistant devant la charge utile.
///
/// L'en-tete est construit ICI, dans le tampon DMA du pilote, et non copie
/// depuis un tampon intermediaire : le vidage final deplace jusqu'a huit
/// mebioctets, et une copie de plus se paierait a chaque enregistrement.
fn encadre(zone: &mut [u8], d: &bobine::Descripteur, charge: &[u8]) -> bool {
    if zone.len() < ENTETE_OCTETS + charge.len() {
        return false;
    }
    for octet in zone.iter_mut() {
        *octet = 0;
    }
    zone[0..8].copy_from_slice(MAGIE);
    zone[8..10].copy_from_slice(&VERSION_FORMAT.to_le_bytes());
    zone[10..12].copy_from_slice(&d.genre.to_le_bytes());
    zone[12..16].copy_from_slice(&(ENTETE_OCTETS as u32).to_le_bytes());
    zone[16..24].copy_from_slice(&boot_id().to_le_bytes());
    zone[24..32].copy_from_slice(&d.seq.to_le_bytes());
    zone[32..40].copy_from_slice(&d.ts_ns.to_le_bytes());
    zone[40..44].copy_from_slice(&(charge.len() as u32).to_le_bytes());
    zone[44..48].copy_from_slice(&crc32(charge).to_le_bytes());
    zone[48..56].copy_from_slice(&d.fin_trace.to_le_bytes());
    zone[ENTETE_OCTETS..ENTETE_OCTETS + charge.len()].copy_from_slice(charge);
    true
}

/// `BOUBBX01`, la signature du format persistant. Voir
/// `tools/reference/extract-blackbox.py`, qui la relit.
const MAGIE: &[u8; 8] = b"BOUBBX01";
const VERSION_FORMAT: u16 = 1;
const ENTETE_OCTETS: usize = 64;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// Depuis quand l'enregistreur n'a-t-il rien POSE, en nanosecondes ?
///
/// Rend zero tant qu'il n'a jamais rien pose -- il n'a alors rien a
/// expliquer.
///
/// # Ce que ce chiffre mesure maintenant
///
/// Il mesurait la derniere ECRITURE REUSSIE SUR LA CLE. Il melait donc deux
/// pannes : un enregistreur qui ne tourne plus, et un support qui ne repond
/// plus. Les trois archives physiques ont ete lues avec cette ambiguite, et
/// c'est elle qui a coute trois hypotheses fausses.
///
/// Il mesure desormais la derniere POSE EN MEMOIRE. Il ne dit plus rien du
/// support -- et c'est le point : le support a ses propres compteurs, et la
/// question « l'enregistreur tourne-t-il encore ? » a enfin une reponse qui
/// ne depend que de l'enregistreur.
pub fn silence_ns() -> u64 {
    let etat = souffle();
    if etat.dernier_ok_ns == 0 {
        return 0;
    }
    etat.silence_ms.saturating_mul(1_000_000)
}

/// Filet de securite : produire depuis un autre fil quand le notre ne tourne
/// plus.
///
/// # Pourquoi un filet, et pas seulement une priorite
///
/// L'archive du 12 septembre 17:55 s'arrete au milieu d'un tour de
/// scrutation, `bb_failures=0`, alors que le bureau tournait a soixante-deux
/// trames par seconde. L'enregistreur n'avait pas echoue : il n'etait plus
/// elu. Promouvoir son fil en Interactive corrige la cause la plus probable ;
/// ce filet couvre le cas ou elle ne serait pas la seule.
///
/// L'appelant est le compositeur, qui tourne toujours. Depuis V3, `poll()` ne
/// touche que la memoire : cet appel coute une copie et deux atomiques, et ne
/// peut donc plus figer une trame meme si le support est perdu.
pub fn filet_de_securite(seuil_ns: u64) -> bool {
    let silence = silence_ns();
    if silence < seuil_ns {
        return false;
    }
    FILETS.fetch_add(1, Ordering::Relaxed);
    poll();
    true
}

/// Nombre de fois ou un autre fil a du produire a la place de l'enregistreur.
pub fn filets() -> u64 {
    FILETS.load(Ordering::Relaxed)
}

static FILETS: AtomicU64 = AtomicU64::new(0);

/// Lots physiques poses, et enregistrements que le tambour avait deja
/// ecrases quand le vidage est passe.
///
/// Zero ecrase veut dire que la trace persistee est un recit COMPLET. Tout
/// autre chiffre dit exactement combien il en manque -- ce qu'aucune archive
/// des trois sessions physiques n'a jamais su dire.
pub fn vidage_compteurs() -> (u64, u64, u64) {
    (
        VIDAGE_LOTS.load(Ordering::Relaxed),
        VIDAGE_MANQUANTS.load(Ordering::Relaxed),
        PROCHAIN_A_POSER.load(Ordering::Acquire),
    )
}

/// Vide l'enregistreur de vol AVANT une extinction volontaire.
///
/// # Pourquoi cette fonction existe
///
/// Les trois premieres sessions physiques ont toutes fini au bouton
/// d'alimentation, et l'archive extraite ne contenait a chaque fois que les
/// premieres dizaines de secondes. Non parce que l'enregistreur tombait, mais
/// parce qu'une coupure brutale n'a personne pour ecrire ce qui reste.
///
/// Ce sont precisement les dernieres secondes qu'on cherche : celles ou la
/// machine s'est degradee. L'enregistreur voyait tout sauf ce qu'on lui
/// demandait.
///
/// Une extinction par le menu passe donc par ici, et ecrit :
///
///   * les evenements de vol encore en memoire ;
///   * le journal serie qui n'a pas encore ete pose ;
///   * un dernier echantillon, et un dernier releve memoire ;
///   * une marque de fin, qui distingue une session CLOSE d'une session
///     coupee -- sans elle, on ne sait pas si le silence est la fin ou une
///     panne.
/// Issue detaillee d'un vidage final.
///
/// Un `bool` perdait l'information a l'instant precis ou elle devenait utile :
/// « incomplet » ne dit pas si le journal n'a pas pu etre draine, si la marque
/// de fin n'est pas passee, ou si seule la synchronisation a manque -- trois
/// pannes de trois couches differentes, avec trois remedes differents.
pub struct Vidage {
    /// Le support repondait-il seulement ?
    pub support: bool,
    /// Le journal serie et l'enregistreur de vol ont-ils ete vides ?
    pub draine: bool,
    /// La marque de FIN est-elle posee ? Sans elle, un silence est
    /// indistinguable d'une coupure.
    pub marque: bool,
    /// La cle a-t-elle confirme l'ecriture ?
    pub synchronise: bool,
}

impl Vidage {
    pub fn complet(&self) -> bool {
        self.support && self.draine && self.marque && self.synchronise
    }
}

pub fn vide_avant_extinction(raison: &str) -> Vidage {
    // ARRETER LA PRODUCTION AVANT DE VIDER, ET DANS CET ORDRE.
    //
    // `poll()` continue d'ajouter des echantillons tant qu'il tourne. Vider
    // en meme temps qu'on produit, c'est courir apres une queue qui avance :
    // le vidage ne se terminerait qu'a la faveur d'un hasard d'ordonnancement.
    STOPPING.store(true, Ordering::Release);
    let maintenant = now_ns();
    let echeance = maintenant.saturating_add(BUDGET_EXTINCTION_NS);

    // Un dernier releve AVANT tout le reste : c'est celui qui porte l'etat au
    // moment ou l'utilisateur a demande l'extinction, et c'est souvent le seul
    // qu'on vienne chercher.
    sample(maintenant);
    memory_sample(maintenant);
    network_sample(maintenant);

    let support = crate::drivers::xhci_active::blackbox_storage_ready();
    if !support {
        let etat = BOBINE.etat();
        crate::serial_println!(
            "BOUCHAUD_BLACKBOX_FIN_SANS_SUPPORT raison={} tambour_poses={} tambour_ecrases={}",
            raison, etat.poses, etat.ecrases,
        );
        return Vidage {
            support: false,
            draine: false,
            marque: false,
            synchronise: false,
        };
    }

    // ORDRE : LE TAMBOUR D'ABORD, LES ANNEAUX ENSUITE.
    //
    // Le journal serie et les evenements de vol representent plus d'un
    // mebioctet a convertir en enregistrements. Les convertir AVANT de vider
    // le tambour ferait ecraser les plus anciens echantillons -- c'est-a-dire
    // le debut de la session, precisement ce qu'aucune des trois archives
    // physiques n'a jamais contenu. Convertis apres, ils passent par un
    // tambour vide, et rien n'est chasse.
    let mut bilan = vidange(echeance);
    crate::gui::power_screen::progress("Enregistrement des journaux", 0);

    // Les evenements de vol, par paquets, en vidant entre chaque paquet.
    let cible_vol = FLIGHT_WRITE.load(Ordering::Acquire);
    let mut etape = 1usize;
    while now_ns() < echeance && FLIGHT_FLUSHED.load(Ordering::Acquire) < cible_vol {
        let avant = FLIGHT_FLUSHED.load(Ordering::Acquire);
        for _ in 0..ENREGISTREMENTS_PAR_LOT {
            flush_flight(maintenant);
            if FLIGHT_FLUSHED.load(Ordering::Acquire) >= cible_vol {
                break;
            }
        }
        let tour = vidange(echeance);
        bilan.poses += tour.poses;
        bilan.manquants += tour.manquants;
        crate::gui::power_screen::progress("Enregistrement des journaux", etape);
        etape += 1;
        if FLIGHT_FLUSHED.load(Ordering::Acquire) == avant {
            // Aucun progres : les evenements restants n'ont jamais ete
            // publies. Insister ne les fera pas apparaitre.
            break;
        }
    }

    // Le journal serie, de la meme facon.
    let cible_serie = crate::drivers::serial::trace_total_bytes();
    while now_ns() < echeance && LAST_TRACE_SEQ.load(Ordering::Acquire) < cible_serie {
        let avant = LAST_TRACE_SEQ.load(Ordering::Acquire);
        flush_serial(maintenant, false);
        let tour = vidange(echeance);
        bilan.poses += tour.poses;
        bilan.manquants += tour.manquants;
        crate::gui::power_screen::progress("Enregistrement des journaux", etape);
        etape += 1;
        if LAST_TRACE_SEQ.load(Ordering::Acquire) == avant {
            break;
        }
    }

    // LAQUELLE DES TROIS CONDITIONS MANQUE ?
    //
    // `draine` en agrege trois, et un `drained=0` seul ne dit pas laquelle.
    // Le banc rend `drained=0 marker=1 sync=1 ok=0` : l'archive est pourtant
    // declaree COMPLETE par l'extracteur, donc le verdict ment encore, par un
    // autre chemin que la synchronisation.
    //
    // La troisieme condition est de la meme famille que le defaut que le
    // checkpoint vient de corriger chez lui : elle exige un tambour
    // ENTIEREMENT vide, ce qui n'est pas la meme question que « tout ce qui
    // devait sortir est sorti ».
    let vol_ok = FLIGHT_FLUSHED.load(Ordering::Acquire) >= cible_vol;
    let serie_ok = LAST_TRACE_SEQ.load(Ordering::Acquire) >= cible_serie;
    let reste = vidange(echeance).reste;
    let draine = vol_ok && serie_ok && !reste;
    if !draine {
        crate::serial_println!(
            "BLACKBOX_FIN_DRAINE_KO vol={} serie={} reste={} vol_flushed={} vol_cible={} serie_seq={} serie_cible={}",
            vol_ok as u8, serie_ok as u8, reste as u8,
            FLIGHT_FLUSHED.load(Ordering::Acquire), cible_vol,
            LAST_TRACE_SEQ.load(Ordering::Acquire), cible_serie,
        );
    }

    // LA MARQUE DE FIN PART EN DERNIER, ET ELLE PORTE LE BILAN.
    //
    // Sans elle, un silence est indistinguable d'une coupure. Avec le bilan
    // du tambour dedans, elle dit aussi COMBIEN il manque -- ce qui distingue
    // une archive complete d'une archive amputee, sans avoir a recouper les
    // numeros a la relecture.
    let etat = BOBINE.etat();
    let mut marque = Text::new();
    let _ = write!(
        &mut marque,
        "BOUCHAUD_TRIGKEY_BLACKBOX_V3 FIN raison={} boot_id={} ts_ns={} draine={} tambour_reserves={} tambour_poses={} tambour_ecrases={} tambour_perdus={} tambour_refuses={} vidage_poses={} vidage_manquants={} vidage_lots={}\n",
        raison, boot_id(), maintenant, draine as u8,
        etat.reserves, etat.poses, etat.ecrases, etat.perdus, etat.refuses,
        bilan.poses, bilan.manquants, VIDAGE_LOTS.load(Ordering::Relaxed),
    );
    let mut marked = false;
    if append(KIND_MARKER, marque.as_bytes(), maintenant, cible_serie) {
        // L'echeance de la marque est SEPAREE, et plus genereuse : une
        // archive sans marque de fin ne se distingue pas d'une coupure
        // brutale, et c'est la distinction qu'on paie le plus cher a perdre.
        let echeance_marque = now_ns().saturating_add(BUDGET_MARQUE_NS);
        marked = vidange(echeance_marque).poses != 0 || !vidange(echeance_marque).reste;
    }

    let mut synced = false;
    for pas in 0..16 {
        if now_ns() >= echeance.saturating_add(BUDGET_MARQUE_NS) {
            break;
        }
        crate::gui::power_screen::progress("Synchronisation de la cle USB", pas);
        if crate::drivers::xhci_active::blackbox_force_sync() {
            synced = true;
            break;
        }
        if crate::kernel::task::try_current().is_some() {
            crate::kernel::task::sleep_ticks(1);
        }
    }

    let ok = draine && marked && synced;
    let souffle = souffle();
    let verrou = crate::drivers::xhci_active::etat_du_verrou();
    let bot = crate::drivers::xhci_active::releve_bot();
    let (stalls_bb, reculs_bb, lot_bb) = crate::drivers::xhci_active::blackbox_lot_stats();
    crate::serial_println!(
        "BOUCHAUD_BLACKBOX_FIN raison={} drained={} marker={} sync={} ok={} poses={} echecs={} serie={} pire_serie={} dernier_ok_ns={} silence_ms={} tambour_reserves={} tambour_ecrases={} tambour_perdus={} vidage_poses={} vidage_manquants={} verrou={} verrou_tenue_max_ns={} bot={} bot_reprises={}",
        raison, draine as u8, marked as u8, synced as u8, ok as u8,
        souffle.poses, souffle.echecs, souffle.serie, souffle.pire_serie,
        souffle.dernier_ok_ns, souffle.silence_ms,
        etat.reserves, etat.ecrases, etat.perdus, bilan.poses, bilan.manquants,
        verrou.proprietaire.nom(), verrou.tenue_max_ns,
        bot.etat.nom(), bot.reprises,
    );
    if !synced {
        // CE QUI REND `ok=0` ALORS QUE TOUT EST ECRIT.
        //
        // Au banc, `drained=1 marker=1 sync=0` : les donnees ET la marque de
        // fin sont posees, l'extracteur declare l'archive COMPLETE, et
        // l'utilisateur voit pourtant « erreur lors de l'enregistrement ».
        // C'est tres probablement ce qu'a vu le Trigkey.
        crate::serial_println!(
            "BLACKBOX_FIN_SYNC_KO raison={} transport={} bot={} reprises={}",
            raison, crate::drivers::xhci_active::derniere_raison_sync(),
            bot.etat.nom(), bot.reprises,
        );
    }
    Vidage {
        support: true,
        draine,
        marque: marked,
        synchronise: synced,
    }
}

/// Budget total du vidage final, hors marque de fin.
///
/// Douze secondes. C'est long, et c'est assume : un vidage final porte
/// jusqu'a deux mille enregistrements, et cinq secondes n'y suffisaient pas
/// -- le banc s'arretait a mille cent trente-six sur mille quatre cent
/// cinquante-cinq, sans marque de fin, c'est-a-dire avec une archive qu'on ne
/// peut pas distinguer d'une coupure.
///
/// L'ecran d'extinction affiche sa progression a chaque lot : ce n'est pas un
/// figement, et cela se voit.
// ---------------------------------------------------------------------------
// BOUCHAUD_C72_CHECKPOINT_FAIL_SAFE
// ---------------------------------------------------------------------------
//
// POURQUOI. Au test physique du Trigkey (image d131f2a), l'utilisateur demande
// l'extinction, la sauvegarde finale echoue, la machine s'eteint -- et TOUTE
// la session est perdue, y compris les journaux qui expliquaient le reseau.
// Un observatoire qui perd ses preuves au moment de la panne n'est pas fini.
//
// CE QUE CE N'EST PAS. Ce n'est pas un retour des E/S USB dans le chemin
// chaud : le design RAM-only garde sa raison d'etre -- un diagnostic ne doit
// pas dependre, a chaque evenement, du peripherique qu'il observe. Le
// checkpoint est une operation SEPAREE, bornee, et portee par un fil de
// mesure deja existant.
//
// CE QU'IL REUTILISE. `vidange` est deja bornee par echeance et INCREMENTALE :
// son curseur n'avance que sur ce que le support a confirme. Un checkpoint
// n'est donc qu'un vidage borne suivi d'une marque -- rien de neuf dans le
// chemin de donnees.
//
// LA MARQUE EST D'UN AUTRE TYPE QUE `FIN`. `FIN` reste reservee a une vraie
// fin de session ; une archive qui n'en a pas n'est pas complete, et le dire
// est precisement ce qu'on paie le plus cher a perdre. Un checkpoint porte
// donc `CHECKPOINT`, avec son numero de sequence, et l'extracteur distingue
// trois cas : session complete, session partielle jusqu'au checkpoint N,
// coupure sans checkpoint.

/// Budget total d'un checkpoint. Tres inferieur a celui de l'extinction : il
/// ne doit jamais se voir.
const BUDGET_CHECKPOINT_NS: u64 = 1_500_000_000;

/// Budget propre de la synchronisation de cache.
///
/// Separe, parce que partager celui du vidage revenait a ne jamais
/// synchroniser : le vidage consomme d'abord, et il ne reste rien.
const BUDGET_SYNC_CHECKPOINT_NS: u64 = 800_000_000;

/// Budget de la marque de checkpoint, separe comme celui de `FIN`.
///
/// Releve de 400 ms a 1,2 s sur mesure : a 400 ms, le banc rendait
/// `marker=0` alors que `sync=1` et que 176 enregistrements etaient poses --
/// la marque restait dans le tambour, l'archive n'avait aucun CHECKPOINT, et
/// le verdict tombait a COUPURE. La marque est posee EN DERNIER : tout ce qui
/// la precede doit sortir avant elle.
const BUDGET_MARQUE_CHECKPOINT_NS: u64 = 1_200_000_000;

/// Cadence du checkpoint periodique.
///
/// Valeur de depart, a confirmer par la mesure du cout reel : ce fichier ne
/// choisit pas un chiffre au hasard, il en pose un et le banc le corrige.
const PERIODE_CHECKPOINT_MS: u64 = 45_000;

static CHECKPOINT_SEQ: AtomicU64 = AtomicU64::new(0);
static CHECKPOINT_PROCHAIN_MS: AtomicU64 = AtomicU64::new(0);

/// Ce qu'un checkpoint a donne.
#[derive(Clone, Copy, Default)]
pub struct Checkpoint {
    /// Le support repondait-il seulement ?
    pub support: bool,
    /// Enregistrements effectivement poses par CE checkpoint.
    pub poses: u64,
    /// La marque de checkpoint est-elle posee ?
    pub marque: bool,
    /// La cle a-t-elle confirme l'ecriture ?
    pub synchronise: bool,
    /// Numero de ce checkpoint dans la session.
    pub seq: u64,
    /// Dernier enregistrement confirme persistant.
    pub dernier_confirme: u64,
    /// Duree reelle, pour que la cadence soit choisie sur une mesure.
    pub duree_us: u64,
    /// Pourquoi la synchronisation n'a pas eu lieu, le cas echeant.
    pub cause_sync: u8,
}

impl Checkpoint {
    pub fn ok(&self) -> bool {
        self.support && self.marque && self.synchronise
    }
}

/// Pose un checkpoint : vidage borne, marque `CHECKPOINT`, synchronisation.
///
/// Ne suspend PAS la production : contrairement a `vide_avant_extinction`, la
/// session continue. Ne retire de la RAM que ce que le support a confirme --
/// c'est `vidange` qui en tient le curseur, et elle ne l'avance jamais sur du
/// non-confirme.
///
/// Sans support, rend immediatement un bilan a `support=false` : une machine
/// sans cle reste parfaitement utilisable.
pub fn checkpoint(raison: &str) -> Checkpoint {
    let debut = now_ns();
    let mut bilan = Checkpoint::default();
    bilan.seq = CHECKPOINT_SEQ.fetch_add(1, Ordering::AcqRel) + 1;

    if !crate::drivers::xhci_active::blackbox_storage_ready() {
        bilan.duree_us = now_ns().saturating_sub(debut) / 1_000;
        crate::serial_println!(
            "BLACKBOX_CHECKPOINT_END ok=0 raison={} seq={} support=0 records=0 \
duration_ms={} marker=0 sync=0",
            raison, bilan.seq, bilan.duree_us / 1_000,
        );
        return bilan;
    }
    bilan.support = true;

    crate::serial_println!(
        "BLACKBOX_CHECKPOINT_BEGIN raison={} seq={} boot_id={} ts_ns={}",
        raison, bilan.seq, boot_id(), debut,
    );

    let echeance = debut.saturating_add(BUDGET_CHECKPOINT_NS);
    let tour = vidange(echeance);
    bilan.poses = tour.poses;

    let maintenant = now_ns();
    let etat = BOBINE.etat();

    // CE QUE LA MARQUE ANNONCE EST CE QUI SERA VRAI QUAND ELLE ATTERRIRA.
    //
    // La marque part EN DERNIER : tout ce qui la precede est persiste au
    // moment ou elle-meme l'est. Le dernier numero du tambour AVANT elle est
    // donc exactement la borne qu'elle peut promettre.
    //
    // Le banc a attrape la version precedente : le champ portait `0` parce
    // que `bilan.dernier_confirme` n'etait calcule qu'APRES le vidage, alors
    // que le texte, lui, est fige avant. Le journal disait 288, la marque
    // disait 0, et l'extracteur croyait la marque -- a juste titre.
    let confirme_annonce = BOBINE.dernier() as u64;
    let mut marque = Text::new();
    let _ = write!(
        &mut marque,
        "BOUCHAUD_TRIGKEY_BLACKBOX_V3 CHECKPOINT raison={} boot_id={} seq={} ts_ns={} \
dernier_confirme={} tambour_reserves={} tambour_poses={} tambour_ecrases={} tambour_perdus={} \
vidage_poses={}\n",
        raison, boot_id(), bilan.seq, maintenant, confirme_annonce,
        etat.reserves, etat.poses, etat.ecrases, etat.perdus, tour.poses,
    );
    // LA MARQUE EST POSEE QUAND LE CURSEUR L'A DEPASSEE, ET PAS AVANT.
    //
    // Premiere version : `bilan.marque = !pose.reste`, c'est-a-dire « plus
    // rien du tout ne reste a poser ». Ce n'est pas la question. Le tambour
    // continue de recevoir pendant le vidage : `reste` peut etre vrai a cause
    // d'enregistrements PLUS RECENTS que la marque, qui est pourtant sortie.
    //
    // Le banc l'a montre -- `records=128 marker=0 sync=1` -- et l'archive
    // resultante n'avait aucun CHECKPOINT, donc `COUPURE` au lieu de
    // `PARTIEL_CHECKPOINT` : le checkpoint avait ecrit ses donnees et se
    // declarait rate.
    //
    // Le critere juste est le numero de la marque compare au curseur de
    // persistance.
    if append(KIND_MARKER, marque.as_bytes(), maintenant,
              crate::drivers::serial::trace_total_bytes()) {
        let seq_marque = BOBINE.dernier();
        let echeance_marque = now_ns().saturating_add(BUDGET_MARQUE_CHECKPOINT_NS);
        loop {
            let pose = vidange(echeance_marque);
            bilan.poses = bilan.poses.saturating_add(pose.poses);
            let (_lots, _manquants, prochain) = vidage_compteurs();
            if prochain > seq_marque {
                bilan.marque = true;
                break;
            }
            if now_ns() >= echeance_marque {
                break;
            }
            if pose.poses == 0 {
                // Rien n'est sorti ce tour-ci : le support est momentanement
                // occupe, pas absent. On lui laisse un tick plutot que
                // d'abandonner une marque qui n'attend qu'elle -- la boucle
                // reste bornee par l'echeance juste au-dessus.
                if crate::kernel::task::try_current().is_some() {
                    crate::kernel::task::sleep_ticks(1);
                } else {
                    break;
                }
            }
        }
    }

    // APRES le vidage de la marque, pas avant : sinon le checkpoint
    // sous-declare jusqu'ou il a confirme.
    let (_lots, _manquants, prochain) = vidage_compteurs();
    bilan.dernier_confirme = prochain.saturating_sub(1) as u64;

    // LA SYNCHRONISATION A SON PROPRE BUDGET, ET NON LE RESTE DU PRECEDENT.
    //
    // Premiere version : quatre tentatives bornees par l'echeance du vidage.
    // Le banc a rendu `duration_ms=1093 marker=1 sync=0` -- le vidage avait
    // deja mange l'essentiel du budget, et la synchronisation n'avait plus de
    // quoi aboutir. Les donnees etaient bien sur la cle (l'extracteur relit
    // ses 293 enregistrements), mais `ok=0` faisait passer un checkpoint
    // reussi pour un echec, et `diag-save` aurait rendu un code trompeur.
    //
    // Son echec reste non fatal : seul le cache n'est pas rendu.
    let echeance_sync = now_ns().saturating_add(BUDGET_SYNC_CHECKPOINT_NS);
    let mut cause = crate::drivers::xhci_active::SYNC_SANS_SUPPORT;
    for _ in 0..16 {
        if now_ns() >= echeance_sync {
            break;
        }
        let (fait, pourquoi) = crate::drivers::xhci_active::blackbox_force_sync_detaille();
        cause = pourquoi;
        if fait {
            bilan.synchronise = true;
            break;
        }
        if crate::kernel::task::try_current().is_some() {
            crate::kernel::task::sleep_ticks(1);
        }
    }
    bilan.cause_sync = cause;

    bilan.duree_us = now_ns().saturating_sub(debut) / 1_000;
    crate::serial_println!(
        "BLACKBOX_CHECKPOINT_END ok={} raison={} seq={} support=1 records={} \
duration_ms={} marker={} sync={} sync_cause={} dernier_confirme={}",
        bilan.ok() as u8, raison, bilan.seq, bilan.poses,
        bilan.duree_us / 1_000, bilan.marque as u8, bilan.synchronise as u8,
        nom_cause_sync(bilan.cause_sync), bilan.dernier_confirme,
    );
    if !bilan.synchronise {
        crate::serial_println!(
            "BLACKBOX_CHECKPOINT_SYNC_KO seq={} cause={} transport={}",
            bilan.seq, nom_cause_sync(bilan.cause_sync),
            crate::drivers::xhci_active::derniere_raison_sync(),
        );
    }
    bilan
}

fn nom_cause_sync(cause: u8) -> &'static str {
    use crate::drivers::xhci_active as usb;
    match cause {
        usb::SYNC_OK => "ok",
        usb::SYNC_SANS_SUPPORT => "sans-support",
        usb::SYNC_VERROU_REFUSE => "verrou-refuse",
        _ => "pilote-ko",
    }
}

/// Checkpoint periodique, appele par le fil de mesures.
///
/// Rend `true` si un checkpoint a eu lieu. Borne par `PERIODE_CHECKPOINT_MS`
/// et silencieux sans support : ce chemin ne doit jamais couter a une machine
/// qui n'a pas de cle.
pub fn checkpoint_si_du() -> bool {
    let maintenant = crate::kernel::timer::monotonic_ms();
    let prochain = CHECKPOINT_PROCHAIN_MS.load(Ordering::Acquire);
    if maintenant < prochain {
        return false;
    }
    CHECKPOINT_PROCHAIN_MS.store(
        maintenant.saturating_add(PERIODE_CHECKPOINT_MS), Ordering::Release);
    if !crate::drivers::xhci_active::blackbox_storage_ready() {
        return false;
    }
    checkpoint("periodique");
    true
}

const BUDGET_EXTINCTION_NS: u64 = 12_000_000_000;
/// Budget SUPPLEMENTAIRE reserve a la marque de fin et a la synchronisation.
const BUDGET_MARQUE_NS: u64 = 1_500_000_000;

/// Ce qu'on peut encore sauver quand le noyau vient de tomber.
///
/// # Le budget est BORNE, et c'est la regle
///
/// Un chemin fatal qui attend sans limite ne rend pas la trace : il remplace
/// une panne diagnosticable par une machine figee sur un ecran noir. La pose
/// en memoire, elle, ne peut pas echouer -- elle a lieu d'abord, et c'est
/// elle qui compte. Le vidage vers la cle est un BONUS, tente au mieux, et
/// abandonne des que le budget est epuise.
pub fn fatal_best_effort(cpu: usize, vector: u8, rip: u64, rsp: u64, code: u64) {
    let now = now_ns();
    let mut out = Text::new();
    let etat = BOBINE.etat();
    let _ = write!(
        &mut out,
        "FATAL cpu={} vector={} rip={:#x} rsp={:#x} code={:#x} ts_ns={} tambour_reserves={} tambour_ecrases={} tambour_perdus={}\n",
        cpu, vector, rip, rsp, code, now, etat.reserves, etat.ecrases, etat.perdus,
    );
    // EN MEMOIRE D'ABORD, TOUJOURS.
    let _ = append(KIND_FATAL, out.as_bytes(), now, crate::drivers::serial::trace_total_bytes());
    flush_serial(now, true);

    if !crate::drivers::xhci_active::blackbox_storage_ready() {
        return;
    }
    let echeance = now.saturating_add(BUDGET_FATAL_NS);
    while now_ns() < echeance {
        if !vidange(echeance).reste {
            break;
        }
    }
    crate::drivers::xhci_active::blackbox_force_sync();
}

/// Budget du vidage d'urgence. Deux secondes : assez pour poser quelques
/// centaines d'enregistrements, trop peu pour ressembler a un figement.
const BUDGET_FATAL_NS: u64 = 2_000_000_000;

/// Ce qu'une panique noyau doit laisser derriere elle.
///
/// # Pourquoi cette fonction existe
///
/// La panique du 17 septembre -- une allocation refusee au lancement du
/// navigateur -- n'a laisse que huit lignes a l'ecran. Tout le reste partait
/// sur COM1, que la machine de reference n'a pas, et l'archive ne contenait
/// pas un mot de la session : le handler de panique n'ecrivait rien ici.
///
/// Quatre sessions passees a rendre l'enregistreur increvable, et le seul
/// evenement qu'il devait absolument retenir ne lui etait jamais confie.
///
/// # L'ordre, et pourquoi il est celui-la
///
/// La pose EN MEMOIRE d'abord : elle ne peut pas echouer, elle n'alloue rien,
/// elle ne touche aucun peripherique. Le journal serie ensuite, converti
/// depuis son propre anneau. Le vidage vers la cle en dernier, borne -- une
/// panique qui attendrait sans fin remplacerait un diagnostic par un ecran
/// noir.
pub fn panique(cpu: usize, fichier: &str, ligne: u32, message: &str) {
    let now = now_ns();
    let etat = BOBINE.etat();
    let souffle = souffle();
    let mut out = Text::new();
    let _ = write!(
        &mut out,
        "PANIC cpu={} fichier={} ligne={} ts_ns={} message={}\n",
        cpu, fichier, ligne, now, message,
    );
    let _ = write!(
        &mut out,
        "PANIC_ETAT tambour_reserves={} tambour_poses={} tambour_ecrases={} tambour_perdus={} silence_ms={} derniere_seq={}\n",
        etat.reserves, etat.poses, etat.ecrases, etat.perdus,
        souffle.silence_ms, souffle.derniere_seq,
    );
    // CE QUE LE TAS AVAIT ENCORE, PUISQUE C'EST LUI QUI A REFUSE.
    //
    // Une panique d'allocation sans l'etat du tas laisse la meme question
    // ouverte qu'un ecran vide : manquait-il un octet ou un mebioctet ?
    let d = crate::kernel::dalles_tas::stats();
    let (frames_used, frames_total) = crate::kernel::vmm::frame_stats_relaxed();
    // CE QUI RESTAIT, ET PAS SEULEMENT CE QUI ETAIT PRIS.
    //
    // « Il manquait combien ? » est LA question d'une allocation refusee, et
    // aucun des compteurs ci-dessus n'y repond : ils decrivent les dalles et
    // les frames physiques, pas l'arene du tas. `heap_libre` la ferme.
    let (heap_utilise, heap_libre, heap_total) = crate::kernel::heap::stats();
    let (_, _, _, _, pages_refus, _) = crate::kernel::pages_tas::stats();
    let plus_grand_contigu = crate::kernel::pages_tas::plus_grand_contigu();
    let _ = write!(
        &mut out,
        "PANIC_TAS heap_utilise={} heap_libre={} heap_total={} \
pages_refus={} plus_grand_contigu={} frames_used={} frames_total={} dalles={} vivants={} manques={} saturations={} sous_flux={} surallocations={}\n",
        heap_utilise, heap_libre, heap_total,
        pages_refus, plus_grand_contigu,
        frames_used, frames_total, d.enregistrees, d.objets_vivants,
        d.manques, d.saturations, d.sous_flux, d.surallocations,
    );
    let _ = append(KIND_FATAL, out.as_bytes(), now, crate::drivers::serial::trace_total_bytes());

    // Le journal serie porte tout le releve de panique qui vient d'etre
    // imprime -- sur une machine sans COM1, c'est la SEULE copie.
    STOPPING.store(false, Ordering::Release);
    flush_serial(now, true);

    if !crate::drivers::xhci_active::blackbox_storage_ready() {
        return;
    }
    let echeance = now.saturating_add(BUDGET_FATAL_NS);
    while now_ns() < echeance {
        if !vidange(echeance).reste {
            break;
        }
    }
    crate::drivers::xhci_active::blackbox_force_sync();
}
