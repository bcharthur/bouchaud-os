// Native readiness primitive for Bouchaud OS.
//
// Purpose: replace a future global "something became ready" queue with one
// source per object/event domain. This is a Bouchaud kernel concept; Linux
// poll/ppoll/epoll adapters should translate onto it at the compatibility edge.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::kernel::sync::{
    WaitSource,
    WaitSourceTicket,
    WaitSourceWake,
};

include!("readiness/etat.rs");
include!("readiness/ticket.rs");
include!("readiness/wait.rs");
include!("readiness/signal.rs");
include!("readiness/diagnostic.rs");
