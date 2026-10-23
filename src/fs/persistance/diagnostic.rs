// Periodic persistence snapshot.

pub fn log_transaction_stats() {
    crate::serial_println!(
        "[PERSIST-TXN] calls={} snapshot_ns={} hash_ns={} io_ns={} bytes={} written={} skipped={} max_ns={}",
        TX_CALLS.load(Ordering::Relaxed), TX_SNAPSHOT_NS.load(Ordering::Relaxed),
        TX_HASH_NS.load(Ordering::Relaxed), TX_IO_NS.load(Ordering::Relaxed),
        TX_BYTES.load(Ordering::Relaxed),
        TX_WRITTEN.load(Ordering::Relaxed), TX_SKIPPED.load(Ordering::Relaxed),
        TX_MAX_NS.load(Ordering::Relaxed),
    );
    // P13_FSYNC_PHASE_V6: lock and barrier components.
    crate::serial_println!(
        "[PERSIST-PHASE] lock_ns={} lock_max_ns={} barrier_ns={} barrier_max_ns={} slow_calls={}",
        TX_LOCK_WAIT_NS.load(Ordering::Relaxed),
        TX_LOCK_WAIT_MAX_NS.load(Ordering::Relaxed),
        TX_BARRIER_NS.load(Ordering::Relaxed),
        TX_BARRIER_MAX_NS.load(Ordering::Relaxed),
        TX_SLOW_CALLS.load(Ordering::Relaxed),
    );
    // BOUCHAUD_C5_COMMIT_AB_V1
    //
    // La GENERATION est ce qui distingue l'ancien etat du neuf. La publier
    // permet de verifier, dans une trace de coupure de courant, que le systeme
    // remonte bien sur la generation attendue -- et pas sur une plus ancienne,
    // ce qui serait une perte silencieuse au lieu d'une corruption bruyante.
    // BOUCHAUD_ATA_VIDANGE_A_LA_BARRIERE_V1 : barrieres REELLES (le disque a
    // vide son cache) et barrieres demandees sans etre obtenues. Deux par
    // commit ; une barriere absente est un aveu, pas une erreur.
    let (barrieres, barrieres_absentes) = volume_statistiques();
    crate::serial_println!(
        "[PERSIST-COMMIT] commits={} generation={} montages_v1={} superblocs_rejetes={} barrieres={} barrieres_absentes={}",
        TX_COMMITS.load(Ordering::Relaxed),
        TX_GENERATION.load(Ordering::Relaxed),
        TX_MONTAGES_V1.load(Ordering::Relaxed),
        TX_SUPERBLOCS_REJETES.load(Ordering::Relaxed),
        barrieres,
        barrieres_absentes,
    );
}
