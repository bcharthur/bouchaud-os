//! Session Ladybird supervisee par le bureau. Les moteurs gardent leurs IPC natifs.
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
pub const DEMARRER: u8 = 1;
pub const ARRETER: u8 = 2;
static COMMANDE: AtomicU8 = AtomicU8::new(0);
static RACINE: AtomicU32 = AtomicU32::new(0);
static ECHEC: AtomicU8 = AtomicU8::new(0);
pub fn demande(action: u8) {
    COMMANDE.store(action, Ordering::Release);
    crate::kernel::sync::reveil::INTERFACE.signale(crate::kernel::sync::reveil::Source::Explicite);
}
pub fn prend_commande() -> u8 { COMMANDE.swap(0, Ordering::AcqRel) }
pub fn enregistre(pid: u32) { RACINE.store(pid, Ordering::Release); ECHEC.store(0, Ordering::Release); }
pub fn echec() { ECHEC.store(1, Ordering::Release); }
pub fn racine() -> u32 { RACINE.load(Ordering::Acquire) }
pub fn en_echec() -> bool { ECHEC.load(Ordering::Acquire) != 0 }
/// Arret borne de l'arbre, y compris les moteurs sans fenetre.
pub fn arrete() {
    let pid = RACINE.swap(0, Ordering::AcqRel);
    if pid != 0 {
        for child in crate::kernel::task::arbre_de(pid) {
            crate::kernel::task::tue_processus(child, 0);
        }
    }
}

// Reuse the existing five-second process sample, no additional MM scans on paint.
static CPU: AtomicU64 = AtomicU64::new(0);
static RSS: AtomicU64 = AtomicU64::new(0);
static SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
pub fn observe(rows: &[crate::kernel::task::Mesure], window: u64) {
    let root = racine();
    let mut ticks = 0u64;
    let mut rss = 0u64;
    for row in rows {
        if root != 0 && row.resource_group_id == root {
            ticks = ticks.saturating_add(row.ticks);
            rss = rss.saturating_add(row.rss_octets);
        }
    }
    CPU.store(if window == 0 { 0 } else { ticks.saturating_mul(100)/window }, Ordering::Relaxed);
    RSS.store(rss, Ordering::Relaxed);
    SAMPLE_MS.store(crate::kernel::timer::monotonic_ms(), Ordering::Release);
}
pub fn usage() -> (u64, u64, u64) {
    let sample = SAMPLE_MS.load(Ordering::Acquire);
    (CPU.load(Ordering::Relaxed), RSS.load(Ordering::Relaxed), sample)
}
