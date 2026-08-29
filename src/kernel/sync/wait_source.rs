// Bouchaud OS native event source.
//
// This is intentionally NOT a Linux/POSIX abstraction. It is the kernel's own
// generation + wait queue primitive, used by native readiness/event domains.
// Linux compatibility may translate poll/futex/eventfd semantics onto it later.

use core::sync::atomic::{AtomicU64, Ordering};

use super::wait_queue::{WaitQueue, WaitTicket};

include!("wait_source/etat.rs");
include!("wait_source/ticket.rs");
include!("wait_source/attente.rs");
include!("wait_source/signal.rs");
include!("wait_source/diagnostic.rs");
