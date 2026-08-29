// Snapshots et logs hors hot path.

#[inline]
fn interface_wait_phase_name(phase: u8) -> &'static str {
    match phase {
        INTERFACE_WAIT_PREPARE => "prepare",
        INTERFACE_WAIT_SLEEP => "sleep",
        INTERFACE_WAIT_RESUME => "resume",
        INTERFACE_WAIT_RETURN => "return",
        _ => "idle",
    }
}

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

pub fn log_interface_wait_snapshot() {
    let phase = INTERFACE_WAIT_PHASE.load(Ordering::Acquire);
    let since = INTERFACE_WAIT_PHASE_SINCE_NS.load(Ordering::Acquire);
    let now = crate::kernel::timer::monotonic_ns();

    crate::serial_println!(
        "[INTERFACE-WAIT] phase={}({}) phase_age_ns={} detached={} depth1={} nested={} max_depth={} sleep_ns={} sleep_max_ns={} resume_ns={} resume_max_ns={} depth_violations={}",
        phase,
        interface_wait_phase_name(phase),
        if since == 0 { 0 } else { now.saturating_sub(since) },
        INTERFACE_DETACHED_WAITS.load(Ordering::Relaxed),
        INTERFACE_DETACHED_DEPTH1.load(Ordering::Relaxed),
        INTERFACE_DETACHED_NESTED.load(Ordering::Relaxed),
        INTERFACE_DETACHED_MAX_DEPTH.load(Ordering::Relaxed),
        INTERFACE_DETACHED_SLEEP_NS.load(Ordering::Relaxed),
        INTERFACE_DETACHED_SLEEP_MAX_NS.load(Ordering::Relaxed),
        INTERFACE_RESUME_WAIT_NS.load(Ordering::Relaxed),
        INTERFACE_RESUME_WAIT_MAX_NS.load(Ordering::Relaxed),
        INTERFACE_DEPTH_VIOLATIONS.load(Ordering::Relaxed),
    );
}
