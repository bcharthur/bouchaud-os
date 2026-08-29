//! Réveil événementiel natif de Bouchaud OS.
//!
//! La façade reste `kernel::sync::reveil`. Les responsabilités sont éclatées
//! physiquement, mais les fragments vivent dans le même module Rust.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use super::{WaitSource, WaitSourceTicket, WaitSourceWake};

include!("reveil/types.rs");
include!("reveil/etat.rs");
include!("reveil/signal.rs");
include!("reveil/attente.rs");
include!("reveil/diagnostic.rs");
include!("reveil/global.rs");
