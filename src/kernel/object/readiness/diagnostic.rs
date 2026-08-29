// Diagnostic snapshot.

impl ReadinessSource {
    pub fn stats(&self) -> ReadinessStats {
        ReadinessStats {
            bits: self.bits.load(Ordering::Acquire),
            changes: self.changes.load(Ordering::Relaxed),
            waits: self.waits.load(Ordering::Relaxed),
            immediate_hits: self.immediate_hits.load(Ordering::Relaxed),
            wakeups: self.wakeups.load(Ordering::Relaxed),
        }
    }

    pub fn wait_stats(&self) -> crate::kernel::sync::WaitSourceStats {
        self.wait.stats()
    }
}
