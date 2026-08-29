// Watchdog déterministe du pipeline navigateur.
//
// Il n'essaie pas de "réparer" automatiquement un freeze : il classe le domaine
// suspect à partir d'états observables et enregistre le contexte juste avant
// qu'il ne disparaisse du ring.

static WATCHDOG_LAST_LOG_NS: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_LAST_FAULTS: AtomicU64 = AtomicU64::new(0);

pub const BOTTLENECK_HEALTHY: u64 = 0;
pub const BOTTLENECK_BKL: u64 = 1;
pub const BOTTLENECK_MEMORY: u64 = 2;
pub const BOTTLENECK_BROWSER_RENDERER: u64 = 3;

pub const fn bottleneck_name(kind: u64) -> &'static str {
    match kind {
        BOTTLENECK_BKL => "kernel-bkl",
        BOTTLENECK_MEMORY => "memory-pagefault",
        BOTTLENECK_BROWSER_RENDERER => "browser-renderer",
        _ => "healthy",
    }
}

/// Rend (classe, delta_faults) sans allocation.
pub fn classify_browser_stall(silence_ms: u64) -> (u64, u64) {
    let h = crate::kernel::smp_lock::health_snapshot();
    if !h.owner_depth_ok || h.resume_oldest_ns >= 50_000_000 {
        return (BOTTLENECK_BKL, 0);
    }

    let (resolved, _, _, _, _) = crate::kernel::task::fault_outcome_stats();
    let previous = WATCHDOG_LAST_FAULTS.swap(resolved, Ordering::AcqRel);
    let delta = resolved.saturating_sub(previous);

    // Le rapport est appelé à la cadence du journal GUI (~quelques secondes).
    // Plusieurs milliers de faults entre deux rapports constituent une pression
    // mémoire réelle sous TCG.
    if delta >= 2_000 {
        return (BOTTLENECK_MEMORY, delta);
    }

    if silence_ms >= 500 {
        return (BOTTLENECK_BROWSER_RENDERER, delta);
    }

    (BOTTLENECK_HEALTHY, delta)
}

/// Ligne d'alerte rate-limitée. Elle ne prend pas de BKL explicitement et
/// n'effectue aucune allocation.
pub fn browser_watchdog(pid: u32, silence_ms: u64) -> (u64, u64) {
    let (bottleneck, pf_delta) = classify_browser_stall(silence_ms);
    if silence_ms < 500 && bottleneck == BOTTLENECK_HEALTHY {
        return (bottleneck, pf_delta);
    }

    let now = crate::kernel::timer::monotonic_ns();
    let last = WATCHDOG_LAST_LOG_NS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 1_000_000_000 {
        return (bottleneck, pf_delta);
    }
    if WATCHDOG_LAST_LOG_NS
        .compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return (bottleneck, pf_delta);
    }

    let h = crate::kernel::smp_lock::health_snapshot();
    let snap = browser_snapshot();
    perf_record(PERF_EVT_WATCHDOG, pid, bottleneck, silence_ms);

    crate::serial_println!(
        "[PERF-WATCHDOG] pid={} silence_ms={} bottleneck={} pf_delta={} \
         bkl_owner={} bkl_resume_oldest_ns={} parked={:#x} resume={:#x} \
         input_seq={} frame_seq={} input_to_frame_max_ms={} frame_gap_max_ms={}",
        pid,
        silence_ms,
        bottleneck_name(bottleneck),
        pf_delta,
        h.owner_token,
        h.resume_oldest_ns,
        h.parked_mask,
        h.resume_mask,
        snap.input_seq,
        snap.frame_seq,
        snap.input_to_frame_max_ns / 1_000_000,
        snap.frame_gap_max_ns / 1_000_000,
    );

    (bottleneck, pf_delta)
}

pub fn note_bkl_alert(owner: u64, resume_oldest_ns: u64) {
    perf_record(PERF_EVT_BKL_ALERT, 0, owner, resume_oldest_ns);
}
