//! Event-driven task wait queue with generation-based lost-wakeup avoidance.

use core::sync::atomic::{AtomicU64, Ordering};

/// A ticket is sampled while the caller still protects its resource condition.
#[derive(Clone, Copy)]
pub struct WaitTicket(u64);

pub struct WaitQueue {
    generation: AtomicU64,
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self { generation: AtomicU64::new(1) }
    }

    /// Arm an upcoming wait before releasing the resource lock.
    pub fn ticket(&self) -> WaitTicket {
        WaitTicket(self.generation.load(Ordering::Acquire))
    }

    /// Sleep only if no producer has signalled since `ticket()`.
    pub fn wait(&self, ticket: WaitTicket) {
        let _kernel = crate::kernel::smp_lock::enter();
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            return;
        }
        crate::kernel::task::park_current_on(self.key());
    }

    pub fn wake_one(&self) -> bool {
        self.generation.fetch_add(1, Ordering::Release);
        crate::kernel::task::wake_wait_queue(self.key(), 1) != 0
    }

    pub fn wake_all(&self) -> usize {
        self.generation.fetch_add(1, Ordering::Release);
        crate::kernel::task::wake_wait_queue(self.key(), usize::MAX)
    }

    fn key(&self) -> usize {
        self as *const Self as usize
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}
