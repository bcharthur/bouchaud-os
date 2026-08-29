// Producer side.

impl WaitSource {
    /// Réveille au plus un waiter de cette source.
    ///
    /// La génération avance avant le réveil, comme pour `signal_all`, afin de
    /// fermer la fenêtre ticket -> enregistrement. Les attentes de type futex
    /// tolèrent les réveils parasites et revalident toujours leur mot.
    #[inline]
    pub fn signal_one(&self) -> bool {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.signals.fetch_add(1, Ordering::Relaxed);
        let woke = self.queue.wake_one();
        if woke {
            self.tasks_woken.fetch_add(1, Ordering::Relaxed);
        }
        woke
    }

    /// Normal process/kernel-context signal.
    #[inline]
    pub fn signal_all(&self) -> usize {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.signals.fetch_add(1, Ordering::Relaxed);
        let woke = self.queue.wake_all();
        self.tasks_woken.fetch_add(woke as u64, Ordering::Relaxed);
        woke
    }

    /// Advance the native generation without touching the scheduler.
    ///
    /// Designed for hard IRQ publication. A bottom half must later call
    /// `flush_deferred_bkl_held`.
    #[inline]
    pub fn publish_deferred(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.signals.fetch_add(1, Ordering::Relaxed);
        self.deferred_publications.fetch_add(1, Ordering::Relaxed);
    }

    /// Flush a previously published deferred signal.
    ///
    /// Caller MUST already hold the BKL. This mirrors the V7 mouse bottom-half
    /// contract and avoids recursive global-lock acquisition.
    #[inline]
    pub fn flush_deferred_bkl_held(&self) -> usize {
        self.deferred_flushes.fetch_add(1, Ordering::Relaxed);
        let woke = self.queue.wake_all_bkl_held();
        self.tasks_woken.fetch_add(woke as u64, Ordering::Relaxed);
        woke
    }
}
