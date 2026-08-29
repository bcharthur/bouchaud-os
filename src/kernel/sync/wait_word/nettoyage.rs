// Opportunistic registry cleanup.

fn prune_wait_word(entry: &Arc<WaitWordEntry>) {
    if entry.waiters.load(Ordering::Acquire) != 0 { return; }
    let index = wait_word_bucket(entry.key);
    let mut bucket = WAIT_WORD_TABLE[index].lock();
    if let Some(pos) = bucket.iter().position(|candidate| {
        Arc::ptr_eq(candidate, entry)
            && candidate.waiters.load(Ordering::Acquire) == 0
            && Arc::strong_count(candidate) <= 2
    }) {
        bucket.remove(pos);
        WW_ENTRIES_PRUNED.fetch_add(1, Ordering::Relaxed);
    }
}
