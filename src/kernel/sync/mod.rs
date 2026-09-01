//! Bouchaud kernel synchronization primitives.
//!
//! P0-NG1 keeps the legacy BKL only as a compatibility fallback. New shared
//! state must use a subsystem/object lock; ranked locks additionally enforce a
//! global acquisition order at runtime.

mod spinlock;
mod sleep_mutex;
pub mod bkl_compte;
pub mod domaine;
pub mod discipline;
pub mod ordre_verrous;
pub mod lockdep;
mod ranked;
pub mod reveil;
mod wait_queue;
mod wait_source;
mod wait_word;

pub use spinlock::{SpinLock, SpinLockGuard, SpinLockIrq, SpinLockIrqGuard};
pub use spinlock::{attente_verrou, AttenteVerrou, ATTENTE_LONGUE, ATTENTE_REENTRANTE};
pub use ranked::{RankedSpinLock, RankedSpinLockGuard};
pub use sleep_mutex::{SleepMutex, SleepMutexGuard};
pub use wait_queue::{WaitQueue, WaitTicket};
pub use wait_queue::bkl_stats as waitq_bkl_stats;
pub use wait_queue::detached_stats as waitq_detached_stats;
pub use wait_queue::wake_sans_verrou as waitq_wake_sans_verrou;
pub use wait_word::{wait_word_wait, wait_word_wake, wait_word_stats, log_wait_word_stats, WaitWordStats, WaitWordWake};
pub use wait_source::{WaitSource, WaitSourceStats, WaitSourceTicket, WaitSourceWake};
pub use reveil::{signale_interface, Source as SourceReveil};

pub use crate::arch::x86_64::cpu_local::CpuMask;

// =============================================================================
// BOUCHAUD_C1_ATTRIBUTION_DOMAINE_V1 -- la colle noyau de `domaine.rs`
// =============================================================================
//
// `domaine.rs` reste pur : il ne connait ni le CPU courant, ni le port serie,
// et se teste donc sur l'hote. Ce bloc lui donne les deux.

use core::sync::atomic::{AtomicU64, Ordering as OrdreDomaine};
pub use domaine::{Contrat, Domaine};

static DOMAINES: domaine::Registre = domaine::Registre::neuf();

/// Regressions deja signalees au journal, pour ne pas noyer le port serie.
///
/// Une regression se produit sur un chemin CHAUD : la signaler a chaque
/// acquisition transformerait un diagnostic en panne. Un bit par domaine suffit
/// -- la premiere occurrence porte toute l'information, le compteur porte le
/// volume.
static REGRESSIONS_SIGNALEES: AtomicU64 = AtomicU64::new(0);

/// L'attribution des prises du gros verrou.
pub fn registre_domaines() -> &'static domaine::Registre {
    &DOMAINES
}

/// Journalise la PREMIERE reprise du gros verrou par un chemin declare sorti.
///
/// Ne panique pas, volontairement : une regression de verrouillage se decouvre
/// en general sous charge, et transformer un diagnostic en arret de la machine
/// ferait perdre l'execution qui l'a produite. Le compteur, lui, ne s'efface
/// pas, et `tools/verifie-domaines-bkl.py` refuse la construction qui laisserait
/// le contrat mentir.
pub fn signale_regression_domaine(domaine: Domaine) {
    let bit = 1u64 << (domaine.code() as u64 & 63);
    let deja = REGRESSIONS_SIGNALEES.fetch_or(bit, OrdreDomaine::Relaxed);
    if deja & bit != 0 {
        return;
    }
    crate::serial_println!(
        "[BKL-REGRESSION] domaine={} contrat={} : ce chemin est declare sorti du \
gros verrou et vient de le reprendre",
        domaine.nom(),
        domaine.contrat().nom(),
    );
}

/// Ouvre une portee d'attribution sur le CPU courant, et la referme au Drop.
///
/// A placer a l'ENTREE d'un chemin de production, pas autour d'une acquisition :
/// ce qu'on veut savoir, c'est quel sous-systeme avait besoin du verrou, pas
/// quelle ligne l'a pris.
pub struct PorteeDomaine {
    cpu: usize,
}

impl PorteeDomaine {
    #[inline]
    pub fn nouvelle(domaine: Domaine) -> Self {
        // L'index est lu une fois et CONSERVE : la portee doit se refermer sur
        // la meme pile que celle qu'elle a ouverte, meme si la tache migre
        // entre-temps. Se fier au CPU courant au Drop depilerait chez le voisin.
        let cpu = crate::arch::x86_64::smp::cpu_index().min(domaine::MAX_CPUS - 1);
        DOMAINES.entre(cpu, domaine);
        Self { cpu }
    }
}

impl Drop for PorteeDomaine {
    #[inline]
    fn drop(&mut self) {
        DOMAINES.sort(self.cpu);
    }
}

/// Raccourci lisible sur les chemins de production.
#[inline]
pub fn portee(domaine: Domaine) -> PorteeDomaine {
    PorteeDomaine::nouvelle(domaine)
}
