// Diagnostics.

pub fn bkl_stats() -> (u64, u64) {
    (
        WAITQ_BKL_ENTERS.load(Ordering::Relaxed),
        WAITQ_BKL_WAIT_NS.load(Ordering::Relaxed),
    )
}

pub fn wake_sans_verrou() -> u64 {
    WAITQ_WAKE_SANS_VERROU.load(Ordering::Relaxed)
}

/// detached, legacy, total_ns, max_ns, schedule_loops, depth_violations
pub fn detached_stats() -> (u64, u64, u64, u64, u64, u64) {
    (
        WAITQ_DETACHED_WAITS.load(Ordering::Relaxed),
        WAITQ_LEGACY_WAITS.load(Ordering::Relaxed),
        WAITQ_DETACHED_WAIT_NS.load(Ordering::Relaxed),
        WAITQ_DETACHED_WAIT_MAX_NS.load(Ordering::Relaxed),
        WAITQ_DETACHED_SCHEDULE_LOOPS.load(Ordering::Relaxed),
        WAITQ_DETACHED_BKL_RETURN_VIOLATIONS.load(Ordering::Relaxed),
    )
}
