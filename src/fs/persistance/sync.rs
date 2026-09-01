// Public sync boundary: short snapshot under BKL, expensive work at depth 0.

pub fn synchronise() -> i64 {
    TX_CALLS.fetch_add(1, Ordering::Relaxed);
    let total_start = crate::kernel::timer::monotonic_ns();
    let _transaction = TRANSACTION.lock();

    let original_depth = crate::kernel::smp_lock::profondeur_locale();
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Fs);
    let owned = if original_depth == 0 { Some(crate::kernel::smp_lock::enter()) } else { None };
    let snapshot_start = crate::kernel::timer::monotonic_ns();
    let snapshot = rassemble_snapshot();
    TX_SNAPSHOT_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(snapshot_start), Ordering::Relaxed);
    drop(owned);

    let suspended = if original_depth == 0 { 0 } else { crate::kernel::smp_lock::suspend_for_schedule() };
    let result = synchronise_snapshot(&snapshot);
    let resume_start = crate::kernel::timer::monotonic_ns();
    if suspended != 0 { crate::kernel::smp_lock::resume_after_schedule(suspended); }
    TX_RESUME_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(resume_start), Ordering::Relaxed);
    tx_max(crate::kernel::timer::monotonic_ns().saturating_sub(total_start));
    result
}
