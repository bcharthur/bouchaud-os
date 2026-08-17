//! Facade de l'ordonnanceur reel de Bouchaud OS.
//!
//! L'ancien fichier de ce nom etait reste un prototype cooperatif alors que
//! l'ordonnanceur reel vit desormais dans `kernel::task` : threads, piles
//! noyau, sauvegarde FPU/SSE, preemption IRQ0, priorites et blocage.
//!
//! Cette facade existe pour fournir un point d'entree stable aux diagnostics et
//! aux futurs backends SMP sans maintenir deux implementations concurrentes.

pub use crate::kernel::task::OrdonnanceurStats;

pub fn current() -> u32 {
    match crate::kernel::task::try_current() {
        Some(task) => task.process.borrow().pid,
        None => 0,
    }
}

/// Compatibilite avec l'ancienne API. Le processus courant est maintenant une
/// consequence du contexte ordonnance ; il ne peut plus etre force par un
/// simple entier.
pub fn set_current(_pid: u32) {}

pub fn yield_now() {
    if crate::kernel::task::in_user_task() {
        let _ = crate::kernel::task::schedule();
    }
}

pub fn stats() -> OrdonnanceurStats {
    crate::kernel::task::diagnostic_ordonnanceur()
}

pub fn state() -> &'static str {
    "preemptif UP: IRQ0 ring3 + preemption differee ring0, priorites interactive/normale, FPU/SSE par tache"
}
