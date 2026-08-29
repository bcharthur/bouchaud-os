// Observabilité autonome du handoff V10.

#[inline]
fn log_handoff_snapshot() {
    let raw = HANDOFF_TARGET.load(Ordering::SeqCst);
    let now = crate::kernel::timer::monotonic_ns();
    let target_cpu = if raw == FREE {
        usize::MAX
    } else {
        raw.saturating_sub(1)
    };
    let age = handoff_age_ns(now, raw);

    crate::serial_println!(
        "[BKL-HANDOFF] target={} target_cpu={} age_ns={} lease_ns={} prepared={} wakes={} claims={} deferrals={} rollbacks={} expired={} resume_cancel={} replacements={} park_free_owner={} claim_wait_total_ns={} claim_wait_max_ns={}",
        raw,
        target_cpu,
        age,
        HANDOFF_LEASE_NS,
        HANDOFF_PREPARED.load(Ordering::Relaxed),
        HANDOFF_WAKEUPS.load(Ordering::Relaxed),
        HANDOFF_CLAIMS.load(Ordering::Relaxed),
        HANDOFF_DEFERRALS.load(Ordering::Relaxed),
        HANDOFF_ROLLBACKS.load(Ordering::Relaxed),
        HANDOFF_EXPIRATIONS.load(Ordering::Relaxed),
        HANDOFF_RESUME_CANCELS.load(Ordering::Relaxed),
        HANDOFF_REPLACEMENTS.load(Ordering::Relaxed),
        HANDOFF_PARK_FREE_OWNER.load(Ordering::Relaxed),
        HANDOFF_CLAIM_WAIT_TOTAL_NS.load(Ordering::Relaxed),
        HANDOFF_CLAIM_WAIT_MAX_NS.load(Ordering::Relaxed),
    );
}
