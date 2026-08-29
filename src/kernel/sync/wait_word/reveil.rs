// Producer path. Bucket lookup is local; scheduler wake is delegated to WaitSource.

pub fn wait_word_wake(uaddr: u64, count: u32) -> u32 {
    if count == 0 { return 0; }
    let Some(key) = wait_word_key(uaddr) else {
        WW_WAKE_MISSES.fetch_add(1, Ordering::Relaxed);
        return 0;
    };
    let Some(entry) = wait_word_existing(key) else {
        WW_WAKE_MISSES.fetch_add(1, Ordering::Relaxed);
        return 0;
    };

    let logical = entry.waiters.load(Ordering::Acquire).min(count as u64) as u32;
    if logical == 0 {
        WW_WAKE_MISSES.fetch_add(1, Ordering::Relaxed);
        prune_wait_word(&entry);
        return 0;
    }

    let mut scheduler_wakes = 0u32;
    for _ in 0..logical {
        if entry.wait.signal_one() { scheduler_wakes += 1; }
    }
    // A waiter between ticket and WaitQueue registration observes the advanced
    // generation and will not sleep; count it as logically woken even if no
    // scheduler task was parked yet.
    let _ = scheduler_wakes;
    WW_WAKES.fetch_add(logical as u64, Ordering::Relaxed);
    prune_wait_word(&entry);
    logical
}
