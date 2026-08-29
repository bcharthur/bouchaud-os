// State and counters.

static WAITQ_BKL_ENTERS: AtomicU64 = AtomicU64::new(0);
static WAITQ_BKL_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static WAITQ_WAKE_SANS_VERROU: AtomicU64 = AtomicU64::new(0);

static WAITQ_DETACHED_WAITS: AtomicU64 = AtomicU64::new(0);
static WAITQ_LEGACY_WAITS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_WAIT_MAX_NS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_SCHEDULE_LOOPS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_BKL_RETURN_VIOLATIONS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn waitq_update_max(atom: &AtomicU64, value: u64) {
    let mut old = atom.load(Ordering::Relaxed);
    while value > old {
        match atom.compare_exchange_weak(old, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(now) => old = now,
        }
    }
}

fn enter_bkl() -> crate::kernel::smp_lock::KernelGuard {
    let start = crate::kernel::timer::monotonic_ns();
    let guard = crate::kernel::smp_lock::enter();
    WAITQ_BKL_ENTERS.fetch_add(1, Ordering::Relaxed);
    WAITQ_BKL_WAIT_NS.fetch_add(
        crate::kernel::timer::monotonic_ns().saturating_sub(start),
        Ordering::Relaxed,
    );
    guard
}

#[derive(Clone, Copy)]
pub struct WaitTicket(u64);

pub struct WaitQueue {
    generation: AtomicU64,
    waiters: AtomicU64,
}

struct Inscription<'a> {
    queue: &'a WaitQueue,
}

impl<'a> Inscription<'a> {
    fn nouvelle(queue: &'a WaitQueue) -> Self {
        queue.waiters.fetch_add(1, Ordering::SeqCst);
        Self { queue }
    }
}

impl Drop for Inscription<'_> {
    fn drop(&mut self) {
        self.queue.waiters.fetch_sub(1, Ordering::SeqCst);
    }
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            generation: AtomicU64::new(1),
            waiters: AtomicU64::new(0),
        }
    }

    #[inline]
    fn key(&self) -> usize {
        self as *const Self as usize
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}
