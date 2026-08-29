// Diagnostic V9 : une ligne globale + detail par safe point.

pub fn log_diagnostic() {
    let depth = crate::kernel::smp_lock::profondeur_locale();
    let health = crate::kernel::smp_lock::health_snapshot();
    let now = crate::kernel::timer::monotonic_ns();
    let last = LAST_HANDOFF_NS.load(Ordering::Acquire);

    crate::serial_println!(
        "[KTHREAD-BKL] mode={} task={} depth={} owner={} owner_cpu={} parked={:#x} resume={:#x} checks={} checkpoints={} scopes={} releases={} contended={} gap_current_ns={} gap_max_ns={} unlocked_ns={} unlocked_max_ns={} reacquire_ns={} reacquire_max_ns={} release_window_ns={} release_window_max_ns={} nested_scopes={} max_scope_depth={} handoff_spins={}",
        match MODE { Mode::Legacy => "legacy", Mode::Scoped => "scoped" },
        crate::kernel::task::nom_pour_faute(),
        depth,
        health.owner_token,
        health.owner_cpu,
        health.parked_mask,
        health.resume_mask,
        CHECKS.load(Ordering::Relaxed),
        CHECKPOINTS.load(Ordering::Relaxed),
        SCOPES.load(Ordering::Relaxed),
        RELEASES.load(Ordering::Relaxed),
        CONTENDED_RELEASES.load(Ordering::Relaxed),
        if last == 0 { 0 } else { now.saturating_sub(last) },
        GAP_MAX_NS.load(Ordering::Relaxed),
        UNLOCKED_WORK_NS.load(Ordering::Relaxed),
        UNLOCKED_WORK_MAX_NS.load(Ordering::Relaxed),
        REACQUIRE_WAIT_NS.load(Ordering::Relaxed),
        REACQUIRE_WAIT_MAX_NS.load(Ordering::Relaxed),
        RELEASE_WINDOW_NS.load(Ordering::Relaxed),
        RELEASE_WINDOW_MAX_NS.load(Ordering::Relaxed),
        NESTED_SCOPES.load(Ordering::Relaxed),
        MAX_SCOPE_DEPTH.load(Ordering::Relaxed),
        HANDOFF_SPINS_TOTAL.load(Ordering::Relaxed),
    );

    crate::serial_println!(
        "[KTHREAD-BKL-SKIP] mode={} not_desktop={} interrupts={} no_bkl={} nested={} rate={}",
        SKIP_MODE.load(Ordering::Relaxed),
        SKIP_NOT_DESKTOP.load(Ordering::Relaxed),
        SKIP_INTERRUPTS.load(Ordering::Relaxed),
        SKIP_NO_BKL.load(Ordering::Relaxed),
        SKIP_NESTED.load(Ordering::Relaxed),
        SKIP_RATE.load(Ordering::Relaxed),
    );

    let mut i = 0usize;
    while i < NOMBRE_SITES {
        crate::serial_println!(
            "[KTHREAD-BKL-SITE] site={} releases={} unlocked_ns={}",
            NOMS_SITES[i],
            SITE_RELEASES[i].load(Ordering::Relaxed),
            SITE_UNLOCKED_NS[i].load(Ordering::Relaxed),
        );
        i += 1;
    }
}
