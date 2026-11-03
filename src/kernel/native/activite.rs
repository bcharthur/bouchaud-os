//! BOUCHAUD_NATIVE_ACTIVITE_V1 -- the wakeup side of `WAITSET_WAIT`.
//!
//! ABI 1.1 first shipped `WAITSET_WAIT` as "re-read the state after each
//! bounded 1 ms sleep". Correct (state is re-read, nothing can be lost) but
//! measured expensive: the native services bench (`run_native_services.sh`,
//! TCG) gave an IPC round trip of p50 3.9 ms at SMP1 -- every hop paid a
//! sleep slice -- against ~90 us at SMP4.
//!
//! The kernel already has the right primitive: `WaitSource`, a generation
//! plus a wait queue with a written lost-wakeup contract. Every operation that
//! can RAISE a native signal publishes here (channel send, endpoint closed,
//! event signal, process termination). A waiter takes its ticket BEFORE it
//! polls its wait set; any change after that poll advances the generation, so
//! the wait returns immediately (`AlreadyChanged`) instead of parking.
//!
//! One source for the whole native layer: a send wakes every native waiter,
//! which then re-polls its own wait set. Correct and cheap while native
//! waiters are few; per-object queues can replace it without an ABI change.

use crate::kernel::sync::{WaitSource, WaitSourceTicket, WaitSourceWake};

static ACTIVITE: WaitSource = WaitSource::new();

/// A native signal may have risen. Call AFTER releasing the object's lock.
#[inline]
pub fn publie() {
    ACTIVITE.signal_all();
}

#[inline]
pub fn ticket() -> WaitSourceTicket {
    ACTIVITE.ticket()
}

#[inline]
pub fn attend(ticket: WaitSourceTicket, deadline_ns: u64) -> WaitSourceWake {
    ACTIVITE.wait_until(ticket, deadline_ns)
}
