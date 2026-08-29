// Bouchaud OS native WaitQueue — Final V12.
//
// Depth-0 callers (notably poll/ppoll) never retain a WaitQueue-owned BKL guard
// across schedule(). Depth>0 callers keep the historical depth-preserving path.

use core::sync::atomic::{AtomicU64, Ordering};

include!("wait_queue/etat.rs");
include!("wait_queue/ticket.rs");
include!("wait_queue/attente.rs");
include!("wait_queue/reveil.rs");
include!("wait_queue/diagnostic.rs");
