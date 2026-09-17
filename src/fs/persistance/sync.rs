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

    let original_depth = crate::kernel::smp_lock::profondeur_locale();
    let snapshot_start = crate::kernel::timer::monotonic_ns();
    let snapshot = rassemble_snapshot();
    TX_SNAPSHOT_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(snapshot_start), Ordering::Relaxed);

    let suspended = if original_depth == 0 { 0 } else { crate::kernel::smp_lock::suspend_for_schedule() };
    let result = synchronise_snapshot(&snapshot);
    let resume_start = crate::kernel::timer::monotonic_ns();
    if suspended != 0 { crate::kernel::smp_lock::resume_after_schedule(suspended); }
    TX_RESUME_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(resume_start), Ordering::Relaxed);
    tx_max(crate::kernel::timer::monotonic_ns().saturating_sub(total_start));
    result
}
