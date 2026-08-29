//! Native synchronization boundary.

pub use crate::kernel::readiness::{
    ReadinessSource,
    ReadinessStats,
    ReadinessTicket,
    ERROR,
    HANGUP,
    PRIORITY,
    READABLE,
    WRITABLE,
};
pub use crate::kernel::sync::{
    SleepMutex,
    SleepMutexGuard,
    SpinLock,
    SpinLockGuard,
    SpinLockIrq,
    SpinLockIrqGuard,
    WaitQueue,
    WaitSource,
    WaitSourceStats,
    WaitSourceTicket,
    WaitSourceWake,
};
