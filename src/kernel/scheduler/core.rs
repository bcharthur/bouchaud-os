//! Facade de l'ordonnanceur reel de Bouchaud OS.
//!
//! P0-NG1 ajoute deux briques au scheduler SMP existant : la preemption noyau
//! differee aux safe-points et l'observatoire ready-to-run. L'election et les
//! runqueues restent celles de `kernel::task`, ce qui preserve les invariants
//! Gate0 de passation de pile.

pub mod latency;
pub mod preempt;

pub use crate::kernel::task::OrdonnanceurStats;

pub fn current() -> u32 {
    match crate::kernel::task::try_current() {
        Some(task) => task.process.pid,
        None => 0,
    }
}

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
    "preemptif SMP-NG: affinite, runqueues per-CPU, work-steal, safe-points noyau, latence ready-to-run"
}
