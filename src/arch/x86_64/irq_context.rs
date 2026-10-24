//! Active interrupt frames belong to the running stack, not to its last CPU.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub struct IrqContext {
    depth: AtomicU32,
    entries: AtomicU64,
    maximum: AtomicU32,
}

impl IrqContext {
    pub const fn new() -> Self {
        Self { depth: AtomicU32::new(0), entries: AtomicU64::new(0), maximum: AtomicU32::new(0) }
    }

    pub fn enter(&self) {
        let depth = self.depth.fetch_add(1, Ordering::Relaxed) + 1;
        self.entries.fetch_add(1, Ordering::Relaxed);
        self.maximum.fetch_max(depth, Ordering::Relaxed);
    }

    pub fn exit(&self) {
        self.depth.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .expect("irq_depth underflow");
    }

    pub fn depth(&self) -> u32 { self.depth.load(Ordering::Relaxed) }
    pub fn entries(&self) -> u64 { self.entries.load(Ordering::Relaxed) }
    pub fn maximum(&self) -> u32 { self.maximum.load(Ordering::Relaxed) }

    /// IF must be clear throughout the stack switch and until restore().
    pub fn suspend(&self) -> u32 { self.depth.swap(0, Ordering::Relaxed) }

    pub fn restore(&self, depth: u32) {
        assert_eq!(self.depth.swap(depth, Ordering::Relaxed), 0,
            "switch_context: incoming stack inherited another IRQ context");
    }
}
