// Bucket-local registry. No global task-table scan on lookup.

fn wait_word_entry(key: u64) -> Arc<WaitWordEntry> {
    let index = wait_word_bucket(key);
    let mut bucket = WAIT_WORD_TABLE[index].lock();
    if let Some(entry) = bucket.iter().find(|entry| entry.key == key) {
        return Arc::clone(entry);
    }
    let entry = Arc::new(WaitWordEntry::new(key));
    bucket.push(Arc::clone(&entry));
    WW_ENTRIES_CREATED.fetch_add(1, Ordering::Relaxed);
    WW_BUCKET_PEAK.fetch_max(bucket.len() as u64, Ordering::Relaxed);
    entry
}

fn wait_word_existing(key: u64) -> Option<Arc<WaitWordEntry>> {
    let index = wait_word_bucket(key);
    WAIT_WORD_TABLE[index]
        .lock()
        .iter()
        .find(|entry| entry.key == key)
        .cloned()
}
