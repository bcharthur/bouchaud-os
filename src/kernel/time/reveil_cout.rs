//! Profilage explicite, hors qualification : compteurs cumulatifs de cout.
//! Aucun RDMSR ajoute pour mesurer un timer. Les cycles incluent toute
//! interruption de la section ; ce ne sont pas des cycles CPU imputables.
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

static ACTIF: AtomicBool = AtomicBool::new(false);
pub const N: usize = 13;
pub const NOMS: [&str; N] = ["msr_deadline_ecriture", "rearmement", "scan_du",
    "scan_recalcul", "scan_interactif", "irq_avant_election", "registre_prise", "publication",
    "msr_lecture", "msr_ecriture", "ipi_distant", "preemption_differee", "rdtsc_appels"];
static APPELS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static CYCLES: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static MAXIMUM: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

pub fn active(oui: bool) { ACTIF.store(oui, Relaxed); }
pub fn actif() -> bool { ACTIF.load(Relaxed) }

#[inline]
fn estampille() -> u64 {
    let (bas, haut): (u32, u32);
    // Horloge du profileur distincte de cpu::rdtsc, pour ne pas compter ses
    // propres lectures (deux par mesure). Les sections sont INCLUSIVES :
    // leurs sommes ne s'additionnent pas entre sites imbriques.
    unsafe { core::arch::asm!("lfence", "rdtsc", "lfence",
        out("eax") bas, out("edx") haut, options(nostack, preserves_flags)); }
    ((haut as u64) << 32) | bas as u64
}

pub struct Mesure { site: usize, debut: Option<u64> }
impl Mesure {
    #[inline]
    pub fn commence(site: usize) -> Self {
        Self { site, debut: actif().then(estampille) }
    }
}
impl Drop for Mesure {
    #[inline]
    fn drop(&mut self) {
        if let Some(debut) = self.debut {
            let cycles = estampille().wrapping_sub(debut);
            APPELS[self.site].fetch_add(1, Relaxed);
            CYCLES[self.site].fetch_add(cycles, Relaxed);
            MAXIMUM[self.site].fetch_max(cycles, Relaxed);
        }
    }
}

/// Les prises du registre sont comptees, sans les chronometrer : leur attente
/// peut etre interrompue. Les scans mesurent deja leur cout inclusif.
#[inline]
pub fn registre_pris() {
    compte(6);
}

#[inline]
pub fn compte(site: usize) {
    if actif() { APPELS[site].fetch_add(1, Relaxed); }
}

pub fn releve(site: usize) -> (u64, u64, u64) {
    (APPELS[site].load(Relaxed), CYCLES[site].load(Relaxed), MAXIMUM[site].load(Relaxed))
}
