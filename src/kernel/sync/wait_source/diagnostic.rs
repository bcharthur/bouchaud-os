// Observable state, no serial I/O on the hot path.

impl WaitSource {
    pub fn stats(&self) -> WaitSourceStats {
        WaitSourceStats {
            generation: self.generation.load(Ordering::Acquire),
            tickets: self.tickets.load(Ordering::Relaxed),
            waits: self.waits.load(Ordering::Relaxed),
            avoided_waits: self.avoided_waits.load(Ordering::Relaxed),
            signaled: self.signaled.load(Ordering::Relaxed),
            deadlines: self.deadlines.load(Ordering::Relaxed),
            signals: self.signals.load(Ordering::Relaxed),
            deferred_publications: self.deferred_publications.load(Ordering::Relaxed),
            deferred_flushes: self.deferred_flushes.load(Ordering::Relaxed),
            tasks_woken: self.tasks_woken.load(Ordering::Relaxed),
        }
    }
}
