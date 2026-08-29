//! Bouchaud OS native kernel interfaces.
//!
//! This namespace is deliberately independent from Linux, Windows and macOS.
//! Compatibility layers live OUTSIDE this module and translate foreign ABIs
//! into these Bouchaud-native concepts.
//!
//! V11B starts with narrow stable boundaries; implementations can migrate
//! behind them without changing compatibility code or applications.

pub mod io;
pub mod memory;
pub mod network;
pub mod scheduler;
pub mod sync;
pub mod time;
