//! Bouchaud kernel synchronization primitives.
//!
//! The legacy Big Kernel Lock remains enabled during the SMP-NG migration.
//! New subsystems should use the narrowest primitive that protects their own
//! state instead of adding new BKL dependencies.

mod spinlock;

pub use spinlock::{
    SpinLock,
    SpinLockGuard,
    SpinLockIrq,
    SpinLockIrqGuard,
};

// CpuMask lives with the architecture-neutral logical CPU identity for NG1.
// It is re-exported here so scheduler and kernel code have one stable import.
pub use crate::arch::x86_64::cpu_local::CpuMask;
