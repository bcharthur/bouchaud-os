// Frontiere publique du sync : instantane court, travail couteux a
// profondeur zero.
//
// L'instantane ne prend plus le gros verrou. Ce qu'il lisait -- le systeme de
// fichiers -- a le sien, et `TRANSACTION` serialise deja les synchronisations
// entre elles. Le verrou global n'ajoutait rien, et il le tenait pendant tout
// le rassemblement.

/// Rendu par `synchronise` quand la machine n'a AUCUNE zone de persistance.
///
/// Ce n'est pas une panne : le Stage 2 a cle unique ne demande aucun stockage
/// writable, et le declarer en echec ferait chercher un defaut inexistant.
/// `-1` reste reserve a une zone qui EXISTE et dont l'ecriture a echoue.
pub const SANS_ZONE: i64 = -2;

/// Vrai quand une zone de persistance existe sur cette machine.
pub fn zone_disponible() -> bool {
    debut().is_some()
}

pub fn synchronise() -> i64 {
    TX_CALLS.fetch_add(1, Ordering::Relaxed);
    let total_start = crate::kernel::timer::monotonic_ns();
    let _transaction = TRANSACTION.lock();
    // P13_FSYNC_PHASE_V6: wait for transaction ownership.
    let lock_ns = crate::kernel::timer::monotonic_ns().saturating_sub(total_start);
    TX_LOCK_WAIT_NS.fetch_add(lock_ns, Ordering::Relaxed);
    TX_LOCK_WAIT_MAX_NS.fetch_max(lock_ns, Ordering::Relaxed);

    let snapshot_start = crate::kernel::timer::monotonic_ns();
    let snapshot = rassemble_snapshot();
    TX_SNAPSHOT_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(snapshot_start), Ordering::Relaxed);

    let result = synchronise_snapshot(&snapshot);
    let elapsed_ns = crate::kernel::timer::monotonic_ns().saturating_sub(total_start);
    tx_max(elapsed_ns);
    drop(_transaction);
    // P13_FSYNC_PHASE_V6: avoid serial logging while transaction held.
    if elapsed_ns >= 250_000_000 {
        let n = TX_SLOW_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 32 {
            crate::serial_println!(
                "[PERSIST-SLOW] n={} total_ns={} lock_ns={} result={} snapshot_total_ns={} hash_total_ns={} io_total_ns={} barrier_total_ns={}",
                n, elapsed_ns, lock_ns, result,
                TX_SNAPSHOT_NS.load(Ordering::Relaxed),
                TX_HASH_NS.load(Ordering::Relaxed),
                TX_IO_NS.load(Ordering::Relaxed),
                TX_BARRIER_NS.load(Ordering::Relaxed),
            );
        }
    }
    result
}
