use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::abi::types::Signals;

pub struct Event {
    signaled: AtomicBool,
    sequence: AtomicU64,
}

impl Event {
    pub fn new(initial: bool) -> Self {
        Self { signaled: AtomicBool::new(initial), sequence: AtomicU64::new(1) }
    }

    pub fn signal(&self) -> u64 {
        self.signaled.store(true, Ordering::Release);
        self.sequence.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    pub fn reset(&self) -> u64 {
        self.signaled.store(false, Ordering::Release);
        self.sequence.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    pub fn is_signaled(&self) -> bool { self.signaled.load(Ordering::Acquire) }

    pub fn sequence(&self) -> u64 { self.sequence.load(Ordering::Acquire) }

    pub fn signals(&self) -> Signals {
        if self.is_signaled() { Signals::SIGNALED } else { Signals::NONE }
    }
}
