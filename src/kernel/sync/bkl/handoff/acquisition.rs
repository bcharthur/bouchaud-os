// Claim et priorité d'acquisition du handoff V10.

#[inline]
fn handoff_claim_after_acquire(cpu: usize) {
    let mine = token(cpu);
    let since = HANDOFF_SINCE_NS.load(Ordering::Acquire);

    if HANDOFF_TARGET
        .compare_exchange(mine, FREE, Ordering::SeqCst, Ordering::Acquire)
        .is_ok()
    {
        let wait = crate::kernel::timer::monotonic_ns().saturating_sub(since);
        HANDOFF_CLAIMS.fetch_add(1, Ordering::Relaxed);
        HANDOFF_CLAIM_WAIT_TOTAL_NS.fetch_add(wait, Ordering::Relaxed);
        handoff_update_max(&HANDOFF_CLAIM_WAIT_MAX_NS, wait);
    }
}

/// Une reprise scheduler passe devant tout handoff ordinaire.
#[inline]
fn handoff_cancel_for_resume() {
    if HANDOFF_TARGET.swap(FREE, Ordering::SeqCst) != FREE {
        HANDOFF_RESUME_CANCELS.fetch_add(1, Ordering::Relaxed);
    }
}
