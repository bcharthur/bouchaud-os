// Periodic persistence snapshot.

pub fn log_transaction_stats() {
    crate::serial_println!(
        "[PERSIST-TXN] calls={} snapshot_ns={} hash_ns={} io_ns={} resume_ns={} bytes={} written={} skipped={} max_ns={}",
        TX_CALLS.load(Ordering::Relaxed), TX_SNAPSHOT_NS.load(Ordering::Relaxed),
        TX_HASH_NS.load(Ordering::Relaxed), TX_IO_NS.load(Ordering::Relaxed),
        TX_RESUME_NS.load(Ordering::Relaxed), TX_BYTES.load(Ordering::Relaxed),
        TX_WRITTEN.load(Ordering::Relaxed), TX_SKIPPED.load(Ordering::Relaxed),
        TX_MAX_NS.load(Ordering::Relaxed),
    );
}
