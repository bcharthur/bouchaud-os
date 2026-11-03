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
    note_signal();
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

// ---------------------------------------------------------------------------
// BOUCHAUD_IPC_ATTRIBUTION_V1 -- where a native wakeup spends its time.
// ---------------------------------------------------------------------------
//
// Instrumentation only: nothing here changes who runs or when. Each native
// wait that PARKED and was woken by a signal records four instants:
//
//   signal   the last `publie()` (channel send, event, death...)
//   file     the waiter put back in a run queue  (`publie_pret`)
//   elu      the scheduler elected it            (`commutation`)
//   retour   the wait returned into the syscall  (here)
//
// and so three stages: signal->file (wake path), file->elu (run-queue wait,
// scheduler), elu->retour (switch + resumption). A sample whose last
// `publie()` is LATER than its enqueue is ambiguous (another signal raced in)
// and only counted, never measured. Samples are kept in a fixed ring; the
// shell command `ipc-attribution` prints exact percentiles.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static DERNIER_SIGNAL_NS: AtomicU64 = AtomicU64::new(0);
const ECHANTILLONS: usize = 1024;
static ANNEAU: [[AtomicU64; 3]; ECHANTILLONS] =
    [const { [const { AtomicU64::new(0) }; 3] }; ECHANTILLONS];
static ECRITS: AtomicUsize = AtomicUsize::new(0);
static AMBIGUS: AtomicU64 = AtomicU64::new(0);
static NON_GARES: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn note_signal() { DERNIER_SIGNAL_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Relaxed); }

/// Called by WAITSET_WAIT after a wait. `gare`: the waiter really parked.
pub fn note_reprise(gare: bool) {
    if !gare { NON_GARES.fetch_add(1, Ordering::Relaxed); return; }
    let retour = crate::kernel::timer::monotonic_ns();
    let Some(t) = crate::kernel::task::try_current() else { return };
    let (file, elu) = (t.ipc_file_ns.charge(), t.ipc_elu_ns.charge());
    let signal = DERNIER_SIGNAL_NS.load(Ordering::Relaxed);
    if file == 0 || elu < file || retour < elu || signal == 0 || signal > file {
        AMBIGUS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let i = ECRITS.fetch_add(1, Ordering::Relaxed) % ECHANTILLONS;
    ANNEAU[i][0].store(file - signal, Ordering::Relaxed);
    ANNEAU[i][1].store(elu - file, Ordering::Relaxed);
    ANNEAU[i][2].store(retour - elu, Ordering::Relaxed);
}

/// `ipc-attribution` : one line per stage, exact percentiles over the ring.
pub fn publie_attribution() {
    let n = ECRITS.load(Ordering::Relaxed).min(ECHANTILLONS);
    let cpus = crate::arch::x86_64::smp::schedulable_cpus();
    for (k, nom) in ["signal_file", "file_elu", "elu_retour", "total"].iter().enumerate() {
        let mut v: alloc::vec::Vec<u64> = (0..n).map(|i| {
            if k == 3 { (0..3).map(|j| ANNEAU[i][j].load(Ordering::Relaxed)).sum() }
            else { ANNEAU[i][k].load(Ordering::Relaxed) }
        }).collect();
        v.sort_unstable();
        let p = |q: usize| if v.is_empty() { 0 } else { v[(v.len() - 1) * q / 100] };
        crate::serial_println!(
            "NATIVE_IPC_ATTRIBUTION etape={} n={} p50_ns={} p95_ns={} p99_ns={} max_ns={} cpus={}",
            nom, v.len(), p(50), p(95), p(99), v.last().copied().unwrap_or(0), cpus
        );
    }
    crate::serial_println!(
        "NATIVE_IPC_ATTRIBUTION_COMPTES echantillons={} ambigus={} sans_parking={}",
        ECRITS.load(Ordering::Relaxed), AMBIGUS.load(Ordering::Relaxed), NON_GARES.load(Ordering::Relaxed)
    );
}
