// State and counters.

static WAITQ_WAKE_SANS_VERROU: AtomicU64 = AtomicU64::new(0);

static WAITQ_DETACHED_WAITS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_WAIT_MAX_NS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_SCHEDULE_LOOPS: AtomicU64 = AtomicU64::new(0);

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

#[derive(Clone, Copy)]
pub struct WaitTicket(u64);

pub struct WaitQueue {
    /// Le protocole de parking vit dans `sync::rendezvous`, et c'est LE MEME
    /// code que `tools/smp/test_rendezvous.rs` met a l'epreuve. Le dupliquer
    /// ici rendrait le test decoratif : il prouverait une copie.
    point: crate::kernel::sync::rendezvous::Rendezvous,
}

struct Inscription<'a> {
    queue: &'a WaitQueue,
}

impl<'a> Inscription<'a> {
    fn nouvelle(queue: &'a WaitQueue) -> Self {
        queue.point.inscrit();
        Self { queue }
    }
}

impl Drop for Inscription<'_> {
    fn drop(&mut self) {
        self.queue.point.desinscrit();
    }
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self { point: crate::kernel::sync::rendezvous::Rendezvous::neuf() }
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
