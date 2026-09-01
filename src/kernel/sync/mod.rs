//! Bouchaud kernel synchronization primitives.
//!
//! P0-NG1 keeps the legacy BKL only as a compatibility fallback. New shared
//! state must use a subsystem/object lock; ranked locks additionally enforce a
//! global acquisition order at runtime.

mod spinlock;
mod sleep_mutex;
pub mod bkl_compte;
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
