// Observable state.

pub fn wait_word_stats() -> WaitWordStats {
    WaitWordStats {
        waits: WW_WAITS.load(Ordering::Relaxed),
        value_changed: WW_VALUE_CHANGED.load(Ordering::Relaxed),
        signaled: WW_SIGNALED.load(Ordering::Relaxed),
        deadlines: WW_DEADLINES.load(Ordering::Relaxed),
        faults: WW_FAULTS.load(Ordering::Relaxed),
        wakes: WW_WAKES.load(Ordering::Relaxed),
        wake_misses: WW_WAKE_MISSES.load(Ordering::Relaxed),
        entries_created: WW_ENTRIES_CREATED.load(Ordering::Relaxed),
        entries_pruned: WW_ENTRIES_PRUNED.load(Ordering::Relaxed),
        bucket_peak: WW_BUCKET_PEAK.load(Ordering::Relaxed),
    }
}

pub fn log_wait_word_stats() {
    let s = wait_word_stats();
    crate::serial_println!(
        "[WAIT-WORD] waits={} changed={} signaled={} deadlines={} faults={} wakes={} misses={} created={} pruned={} bucket_peak={}",
        s.waits, s.value_changed, s.signaled, s.deadlines, s.faults,
        s.wakes, s.wake_misses, s.entries_created, s.entries_pruned, s.bucket_peak,
    );
}
