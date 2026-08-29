//! Telemetrie de fluidite graphique de Bouchaud OS.
//!
//! `FPS` signifie ici : trames UTILES composees par seconde. Une trame dont le
//! seul changement est l'horloge de la barre systeme n'est pas comptee. C'est
//! donc une mesure de l'activite visuelle effectivement produite par le
//! compositeur, pas la frequence physique de l'ecran et pas, a elle seule, un
//! score de performance du navigateur.
//!
//! V16 conserve volontairement le pseudo-`Hz` de V14.1 : sans vblank/VSYNC/EDID
//! fiable, ce nombre ressemblait trop a une frequence d'ecran alors qu'il ne
//! mesurait que la cadence logique du compositeur.
//!
//! Le chemin chaud reste sans verrou et sans allocation. V15 ajoute seulement
//! une lecture du tick sur les trames utiles afin de mesurer le plus grand trou
//! entre deux trames -- metrique plus parlante qu'une moyenne FPS lorsqu'une UI
//! se fige pendant plusieurs secondes.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const SAMPLE_MIN_TICKS: u64 = 500;
const MAX_RATE_X10: u64 = 9_999;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static FRAMES_USEFUL: AtomicU64 = AtomicU64::new(0);
static LAST_USEFUL_TICK: AtomicU64 = AtomicU64::new(0);
static MAX_USEFUL_GAP_MS: AtomicU64 = AtomicU64::new(0);

static SAMPLE_TICK: AtomicU64 = AtomicU64::new(0);
static SAMPLE_USEFUL: AtomicU64 = AtomicU64::new(0);
static CACHE_FPS_X10: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub active: bool,
    pub fps_x10: u16,
    pub frames_useful: u64,
    pub useful_gap_max_ms: u64,
    pub since_useful_ms: u64,
}

impl Snapshot {
    #[inline]
    pub fn fps_arrondi(self) -> u16 {
        (self.fps_x10.saturating_add(5) / 10).min(999)
    }
}

#[inline]
fn publie_max(maximum: &AtomicU64, valeur: u64) {
    let mut courant = maximum.load(Ordering::Relaxed);
    while valeur > courant {
        match maximum.compare_exchange_weak(
            courant,
            valeur,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observe) => courant = observe,
        }
    }
}

#[inline]
pub fn note_frame(horloge_seule: bool) {
    if horloge_seule {
        return;
    }

    let maintenant = crate::kernel::timer::ticks().max(1);
    let precedent = LAST_USEFUL_TICK.swap(maintenant, Ordering::Relaxed);
    if precedent != 0 {
        publie_max(&MAX_USEFUL_GAP_MS, maintenant.wrapping_sub(precedent));
    }
    FRAMES_USEFUL.fetch_add(1, Ordering::Relaxed);
    ACTIVE.store(true, Ordering::Release);
}

#[inline]
fn rate_x10(delta: u64, elapsed_ticks: u64) -> u16 {
    if elapsed_ticks == 0 {
        return 0;
    }
    delta
        .saturating_mul(10_000)
        .saturating_add(elapsed_ticks / 2)
        .saturating_div(elapsed_ticks)
        .min(MAX_RATE_X10) as u16
}

/// Instantane sans verrou. Le taux est recalcule au plus tous les 500 ms.
pub fn snapshot() -> Snapshot {
    let active = ACTIVE.load(Ordering::Acquire);
    let useful = FRAMES_USEFUL.load(Ordering::Relaxed);
    let now = if active { crate::kernel::timer::ticks() } else { 0 };

    if active {
        let last = SAMPLE_TICK.load(Ordering::Acquire);
        if last == 0 {
            if SAMPLE_TICK
                .compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                SAMPLE_USEFUL.store(useful, Ordering::Relaxed);
            }
        } else {
            let elapsed = now.wrapping_sub(last);
            if elapsed >= SAMPLE_MIN_TICKS
                && SAMPLE_TICK
                    .compare_exchange(last, now.max(1), Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                let previous_useful = SAMPLE_USEFUL.swap(useful, Ordering::Relaxed);
                let fps_x10 = rate_x10(useful.saturating_sub(previous_useful), elapsed);
                CACHE_FPS_X10.store(fps_x10 as u32, Ordering::Release);
            }
        }
    }

    let last_useful = LAST_USEFUL_TICK.load(Ordering::Relaxed);
    Snapshot {
        active,
        fps_x10: CACHE_FPS_X10.load(Ordering::Acquire) as u16,
        frames_useful: useful,
        useful_gap_max_ms: MAX_USEFUL_GAP_MS.load(Ordering::Relaxed),
        since_useful_ms: if active && last_useful != 0 {
            now.wrapping_sub(last_useful)
        } else {
            0
        },
    }
}

/// Ligne periodique de fluidite. A rapprocher de `[PERF-BROWSER]` :
/// - FRAME-PERF = ce que le compositeur Bouchaud sort ;
/// - PERF-BROWSER = ce que Ladybird produit et la latence entree -> trame.
pub fn publie() {
    let s = snapshot();
    if !s.active {
        crate::serial_println!(
            "[FRAME-PERF] active=0 fps=-- frames_useful=0 useful_gap_max_ms=0 since_useful_ms=0 sample_ms={} source=compositor-useful",
            SAMPLE_MIN_TICKS,
        );
        return;
    }

    crate::serial_println!(
        "[FRAME-PERF] active=1 fps={}.{} frames_useful={} useful_gap_max_ms={} since_useful_ms={} sample_ms={} source=compositor-useful",
        s.fps_x10 / 10,
        s.fps_x10 % 10,
        s.frames_useful,
        s.useful_gap_max_ms,
        s.since_useful_ms,
        SAMPLE_MIN_TICKS,
    );
}
