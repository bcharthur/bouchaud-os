// Snapshots et logs hors hot path.

impl Reveil {
    pub fn generation(&self) -> u64 {
        self.source.generation()
    }

    pub fn invalidations(&self, source: Source) -> u64 {
        self.compteurs[source as usize].load(Ordering::Relaxed)
    }

    pub fn invalidations_totales(&self) -> u64 {
        self.compteurs.iter().map(|c| c.load(Ordering::Relaxed)).sum()
    }

    pub fn statistiques(&self) -> (u64, u64, u64, u64) {
        (
            self.sommeils.load(Ordering::Relaxed),
            self.sommeils_evites.load(Ordering::Relaxed),
            self.reveils_signal.load(Ordering::Relaxed),
            self.reveils_echeance.load(Ordering::Relaxed),
        )
    }

    pub fn irq_statistiques(&self) -> (u64, u64, u64, bool) {
        (
            self.irq_signals.load(Ordering::Relaxed),
            self.irq_flushes.load(Ordering::Relaxed),
            self.irq_woken.load(Ordering::Relaxed),
            self.irq_pending.load(Ordering::Acquire),
        )
    }

    pub fn wait_source_stats(&self) -> super::WaitSourceStats {
        self.source.stats()
    }
}
