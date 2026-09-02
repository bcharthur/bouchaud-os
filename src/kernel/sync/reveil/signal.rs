// Producteurs : contexte normal et hard IRQ.

impl Reveil {
    #[inline]
    pub fn signale(&self, source: Source) {
        self.compteurs[source as usize].fetch_add(1, Ordering::Relaxed);
        self.source.signal_all();
    }

    /// Hard-IRQ safe producer path: atomics only, never BKL, never task scan.
    #[inline]
    pub fn signale_irq(&self, source: Source) {
        self.compteurs[source as usize].fetch_add(1, Ordering::Relaxed);
        self.source.publish_deferred();
        self.irq_signals.fetch_add(1, Ordering::Relaxed);
        self.irq_pending.store(true, Ordering::Release);
    }

    /// Bottom-half flush : atomiques + WaitQueue sans BKL.
    #[inline]
    pub fn flush_irq(&self) -> usize {
        if !self.irq_pending.swap(false, Ordering::AcqRel) {
            return 0;
        }
        self.irq_flushes.fetch_add(1, Ordering::Relaxed);
        let woken = self.source.flush_deferred();
        self.irq_woken.fetch_add(woken as u64, Ordering::Relaxed);
        woken
    }

    #[inline]
    pub fn irq_pending(&self) -> bool {
        self.irq_pending.load(Ordering::Acquire)
    }
}
