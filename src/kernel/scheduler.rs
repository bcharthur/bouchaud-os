//! Ordonnanceur — socle.
//!
//! Etape actuelle : ordonnancement **cooperatif** trivial (le noyau execute une
//! tache a la fois : le shell ou le bureau). Pas encore de preemption ni de
//! changement de contexte. A terme : scheduler round-robin sur timer (IRQ0),
//! sauvegarde/restauration des registres, piles par tache.

static mut CURRENT_PID: u32 = 1;

/// PID de la tache courante.
pub fn current() -> u32 {
    unsafe { CURRENT_PID }
}

/// Designe la tache courante (appele lors d'un changement logique d'activite).
pub fn set_current(pid: u32) {
    unsafe { CURRENT_PID = pid; }
}

/// Cede la main. Cooperatif : no-op pour l'instant (placeholder d'API).
pub fn yield_now() {}

/// Etat de l'ordonnanceur, pour les commandes systeme.
pub fn state() -> &'static str {
    "cooperatif (pas de preemption ; round-robin sur timer planifie)"
}
