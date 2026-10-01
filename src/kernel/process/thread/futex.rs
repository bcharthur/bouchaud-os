// --- Wait-word compatibility bridge -----------------------------------------
//
// The task subsystem no longer owns futex state. Linux/POSIX calls still use
// the historical `task::futex_*` API, but the real mechanism is the Bouchaud
// native wait-word core in `kernel::sync::wait_word`.
//
// BOUCHAUD_C1_FUTEX_MESURE_V1 : attentes et reveils sont comptes. Ces deux
// fonctions suspendaient un gros verrou herite de leur appelant ; il n'existe
// plus (BOUCHAUD_BKL_SUPPRIME_V1).

use core::sync::atomic::Ordering as OrdreFutex;

static FUTEX_ATTENTES: AtomicU64 = AtomicU64::new(0);
static FUTEX_REVEILS: AtomicU64 = AtomicU64::new(0);
/// attentes, reveils.
pub fn futex_stats() -> (u64, u64) {
    (
        FUTEX_ATTENTES.load(OrdreFutex::Relaxed),
        FUTEX_REVEILS.load(OrdreFutex::Relaxed),
    )
}

pub fn futex_wait(uaddr: u64, expected: u32, timeout_ms: u64) -> bool {
    FUTEX_ATTENTES.fetch_add(1, OrdreFutex::Relaxed);
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_begin_if_idle(
        WAIT_FUTEX,
        uaddr,
        ((expected as u64) << 32) | (timeout_ms & 0xffff_ffff),
    );
    let result = crate::kernel::sync::wait_word_wait(uaddr, expected, timeout_ms);
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_clear(WAIT_FUTEX);
    matches!(
        result,
        crate::kernel::sync::WaitWordWake::Signaled
            | crate::kernel::sync::WaitWordWake::ValueChanged
    )
}

pub fn futex_wake(uaddr: u64, count: u32) -> u32 {
    FUTEX_REVEILS.fetch_add(1, OrdreFutex::Relaxed);
    crate::kernel::sync::wait_word_wake(uaddr, count)
}
