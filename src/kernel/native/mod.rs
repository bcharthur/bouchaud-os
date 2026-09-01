//! Bouchaud OS native kernel interfaces.
//!
//! This namespace is deliberately independent from Linux, Windows and macOS.
//! Compatibility layers live OUTSIDE this module and translate foreign ABIs
//! into Bouchaud-native concepts.
//!
//! CHANTIER 7 introduces the first real native object ABI: typed generational
//! handles, per-process handle tables, channels with handle passing, events,
//! wait sets and shared regions.  The x86-64 syscall entry routes the native
//! namespace here before the Linux compatibility dispatcher.

pub mod abi;
pub mod event;
pub mod handle;
pub mod io;
pub mod ipc;
pub mod memory;
pub mod network;
pub mod object;
pub mod scheduler;
pub mod shm;
pub mod sync;
pub mod time;
pub mod waitset;
