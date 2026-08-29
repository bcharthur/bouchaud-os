//! CPU x86_64 : façade pour accounting, idle/HLT, horloge TSC et CPUID.
//!
//! V6 fragmente physiquement le chemin idle sans changer les noms publics.
//! Les fragments utilisent `include!` afin de rester dans le même module Rust.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use crate::arch::x86_64::smp;

include!("cpu/etat.rs");
include!("cpu/idle/politique.rs");
include!("cpu/idle/etat.rs");
include!("cpu/accounting.rs");
include!("cpu/idle/trace.rs");
include!("cpu/idle/scheduler.rs");
include!("cpu/idle/lock_park.rs");
include!("cpu/time.rs");
include!("cpu/info.rs");
