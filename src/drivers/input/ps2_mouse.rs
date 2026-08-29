//! Pilote souris PS/2 : façade V7.
//!
//! Les fragments restent dans le même module `drivers::mouse` via `include!`.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, Ordering};
use crate::arch::x86_64::ports::{inb, outb};
use x86_64::instructions::interrupts;
use crate::drivers::gfx::{WIDTH, HEIGHT};

include!("mouse/etat.rs");
include!("mouse/ps2.rs");
include!("mouse/paquet.rs");
include!("mouse/diagnostic.rs");
