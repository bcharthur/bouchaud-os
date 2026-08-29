//! Bouchaud-native network vocabulary and event boundary.

use core::sync::atomic::{AtomicU64, Ordering};
use crate::kernel::readiness::{ReadinessSource, READABLE, WRITABLE, HANGUP, ERROR};

include!("network/types.rs");
include!("network/readiness.rs");
include!("network/diagnostic.rs");
