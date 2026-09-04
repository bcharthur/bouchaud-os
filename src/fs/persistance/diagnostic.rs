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
    // BOUCHAUD_C5_COMMIT_AB_V1
    //
    // La GENERATION est ce qui distingue l'ancien etat du neuf. La publier
    // permet de verifier, dans une trace de coupure de courant, que le systeme
    // remonte bien sur la generation attendue -- et pas sur une plus ancienne,
    // ce qui serait une perte silencieuse au lieu d'une corruption bruyante.
    crate::serial_println!(
        "[PERSIST-COMMIT] commits={} generation={} montages_v1={} superblocs_rejetes={}",
        TX_COMMITS.load(Ordering::Relaxed),
        TX_GENERATION.load(Ordering::Relaxed),
        TX_MONTAGES_V1.load(Ordering::Relaxed),
        TX_SUPERBLOCS_REJETES.load(Ordering::Relaxed),
    );
}
