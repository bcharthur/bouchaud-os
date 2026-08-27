//! Bouchaud kernel synchronization primitives.
//!
//! The legacy Big Kernel Lock remains enabled during the SMP-NG migration.
//! New subsystems should use the narrowest primitive that protects their own
//! state instead of adding new BKL dependencies.

mod spinlock;
mod sleep_mutex;
pub mod discipline;
pub mod ordre_verrous;
pub mod reveil;
mod wait_queue;

pub use spinlock::{
    SpinLock,
    SpinLockGuard,
    SpinLockIrq,
    SpinLockIrqGuard,
};
pub use spinlock::{
    attente_verrou,
    AttenteVerrou,
    ATTENTE_LONGUE,
    ATTENTE_REENTRANTE,
};
pub use sleep_mutex::{SleepMutex, SleepMutexGuard};
pub use wait_queue::{WaitQueue, WaitTicket};
pub use wait_queue::bkl_stats as waitq_bkl_stats;
pub use wait_queue::wake_sans_verrou as waitq_wake_sans_verrou;
pub use reveil::{signale_interface, Source as SourceReveil};

// CpuMask lives with the architecture-neutral logical CPU identity for NG1.
// It is re-exported here so scheduler and kernel code have one stable import.
pub use crate::arch::x86_64::cpu_local::CpuMask;
