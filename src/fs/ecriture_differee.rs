//! Ecriture differee de `/persist` (BOUCHAUD_ECRITURE_DIFFEREE_V1).
//!
//! Voir `ecriture_differee_politique.rs` pour la regle. Ce module tient la
//! generation des modifications et le fil noyau qui l'applique.
//!
//! Le fil ne prend aucun verrou lui-meme pour decider : la generation est un
//! atomique, incremente par `ramfs::FileSystem::touche` quand le noeud touche
//! est sous `/persist`. La transaction est `persistance::synchronise`, la meme
//! que `fsync` : son ordre (contenu, table, barriere, superbloc, barriere) et
//! son atomicite ne changent pas.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub use super::ecriture_differee_politique::{Etat, CALME_MS, PLAFOND_MS};

static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Generation a la fin du montage : ce que le disque porte deja. Reference du
/// fil, quel que soit le moment ou il demarre -- les ecritures faites entre le
/// montage et son lancement (un autorun) restent donc a ecrire.
static AU_MONTAGE: AtomicU64 = AtomicU64::new(0);
static LANCE: AtomicBool = AtomicBool::new(false);
static TRANSACTIONS: AtomicU64 = AtomicU64::new(0);
static ECHECS: AtomicU64 = AtomicU64::new(0);

/// Periode d'observation du fil.
const PERIODE_MS: u64 = 250;

/// Une modification sous `/persist` vient d'avoir lieu.
#[inline]
pub fn note_modification() {
    GENERATION.fetch_add(1, Ordering::Release);
}

/// Le montage vient de deposer ce que le disque porte.
pub fn note_fin_montage() {
    AU_MONTAGE.store(generation(), Ordering::Release);
}

/// Generation courante des modifications sous `/persist`.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// `(transactions, echecs)` de l'ecriture differee depuis le demarrage.
pub fn compteurs() -> (u64, u64) {
    (TRANSACTIONS.load(Ordering::Relaxed), ECHECS.load(Ordering::Relaxed))
}

/// Lance le fil, une fois, s'il y a une zone de persistance.
pub fn demarre() -> bool {
    if !crate::fs::persistance::zone_disponible() {
        return false;
    }
    if LANCE.swap(true, Ordering::AcqRel) {
        return true;
    }
    let ok = crate::kernel::task::spawn_noyau_priorite(fil, "ecriture-differee",
        crate::kernel::task::Priorite::Normale);
    crate::serial_println!("BOUCHAUD_ECRITURE_DIFFEREE_LANCEE calme_ms={} plafond_ms={} ok={}",
        CALME_MS, PLAFOND_MS, ok as u8);
    ok
}

fn fil() -> ! {
    let ticks = (PERIODE_MS * crate::kernel::timer::TICKS_PER_SECOND / 1000).max(1);
    let mut etat = Etat::neuf(AU_MONTAGE.load(Ordering::Acquire));
    loop {
        crate::kernel::task::sleep_ticks(ticks);
        let maintenant = crate::kernel::timer::monotonic_ms();
        let g = generation();
        if !etat.decide(g, maintenant) {
            continue;
        }
        let debut = crate::kernel::timer::monotonic_ns();
        let r = crate::fs::persistance::synchronise();
        let us = crate::kernel::timer::monotonic_ns().saturating_sub(debut) / 1000;
        if r >= 0 {
            etat.reussie(g);
            let n = TRANSACTIONS.fetch_add(1, Ordering::Relaxed) + 1;
            // Une ligne pour les premieres, puis une sur 32 : un navigateur
            // ecrit souvent.
            if n <= 8 || n % 32 == 0 {
                crate::serial_println!("BOUCHAUD_ECRITURE_DIFFEREE n={} generation={} entrees={} us={}",
                    n, g, r, us);
            }
        } else {
            etat.echouee(maintenant);
            let n = ECHECS.fetch_add(1, Ordering::Relaxed) + 1;
            crate::serial_println!("BOUCHAUD_ECRITURE_DIFFEREE_ECHEC n={} resultat={} reprise_ms={}",
                n, r, etat.reprise_ms.saturating_sub(maintenant));
        }
    }
}
