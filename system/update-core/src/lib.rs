//! bouchaud-update-core -- the A/B boot and update policy, in ONE place.
//!
//! BOUCHAUD_UPDATE_CORE_V1. `no_std`, no allocation, no dependency on the
//! kernel. Consumers:
//!   * the Bouchaud loader (UEFI preboot shim): chooses the slot to boot and
//!     consumes a candidate attempt BEFORE the handoff;
//!   * the image builder: writes the initial boot metadata;
//!   * host tests (tools/update/): state machine, torn writes, durability
//!     boundaries with a volatile write cache;
//!   * later, the kernel mechanism behind an update capability, and recovery.
//!
//! The policy (candidate, good, bad, retry, rollback) is NOT a kernel
//! responsibility: it lived in src/kernel/update/ab.rs only because nothing
//! consumed it yet. Moved here unchanged except for the explicit
//! running-slot invariant.
#![no_std]

mod ab;
pub use ab::*;
