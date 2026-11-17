//! Releves du systeme pour les vues de diagnostic (BOUCHAUD_RELEVE_SECONDE_V1).
//!
//! Insights et le Moniteur affichaient des compteurs relus a CHAQUE trame de
//! leur fenetre -- une trame par mouvement de souris au-dessus d'elle. Parmi
//! eux, `pci::count()` re-enumerait tout le bus (plus de 16 000 E/S ; sous
//! WHPX ~100 ms par trame), et le tas, la table des processus et le RAMFS
//! etaient verrouilles pour des chiffres que l'oeil lit a la seconde.
//!
//! Le releve est fait au plus une fois par seconde d'horloge monotone et
//! partage entre les deux vues. Il est pris SANS tenir le verrou du releve :
//! aucun ordre nouveau entre ce verrou et ceux du tas, du RAMFS ou des
//! processus (le Moniteur a deja connu ce piege, voir LOCKDEP).

use crate::kernel::sync::SpinLock;

/// Ce que les vues affichent. Copie entiere : une trame ne tient aucun verrou.
#[derive(Clone, Copy)]
pub(crate) struct Releve {
    pub heap_utilise: usize,
    pub heap_libre: usize,
    pub heap_total: usize,
    pub processus: usize,
    pub noeuds_ramfs: usize,
    pub inodes_ramfs: usize,
    pub pci: usize,
    pub touches_client: u64,
    pub touches_bureau: u64,
    pub invalidations_plein_ecran: u64,
}

static DERNIER: SpinLock<Option<(u64, Releve)>> = SpinLock::new(None);

fn mesure() -> Releve {
    let (heap_utilise, heap_libre, heap_total) = crate::kernel::heap::stats();
    let (touches_client, touches_bureau, invalidations_plein_ecran) =
        crate::gui::window_manager::stats_entree();
    // ORDRE DES VERROUS : le RAMFS (rang 50) est rendu avant la table des
    // processus (rang 20) -- meme sequence que le Moniteur d'avant.
    let inodes_ramfs = crate::fs::ramfs::fs().used_nodes();
    let noeuds_ramfs = crate::fs::ramfs::used_nodes_relaxed();
    let processus = crate::kernel::process::count();
    Releve {
        heap_utilise, heap_libre, heap_total, processus, noeuds_ramfs, inodes_ramfs,
        pci: crate::arch::x86_64::pci::count(),
        touches_client: touches_client as u64, touches_bureau: touches_bureau as u64,
        invalidations_plein_ecran: invalidations_plein_ecran as u64,
    }
}

/// Le releve courant : refait si le dernier a plus d'une seconde.
pub(crate) fn courant() -> Releve {
    let t = crate::kernel::timer::monotonic_ns() / 1_000_000;
    let connu = *DERNIER.lock();
    if let Some((quand, releve)) = connu {
        if t.saturating_sub(quand) < 1000 {
            return releve;
        }
    }
    let releve = mesure();
    *DERNIER.lock() = Some((t, releve));
    releve
}
