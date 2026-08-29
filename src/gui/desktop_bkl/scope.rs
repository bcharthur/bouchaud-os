// V16 - Safe points et scopes hors BKL du desktop.
//
// `checkpoint()` reste volontairement limite a depth=1 : il n'a aucune raison
// fonctionnelle de couper une section critique imbriquee.
//
// `sans_bkl()` est different : ses appels sont poses UNIQUEMENT autour de
// travail explicitement local (copie LFB et rapport atomique). Le scheduler
// sait deja suspendre/restaurer une profondeur BKL complete lors d'un switch.
// V16 reutilise ce meme contrat afin que depth=2 ne transforme plus un present
// ou une attente desktop en tenue BKL de plusieurs secondes.

#[inline]
fn mode_actif() -> bool { MODE == Mode::Scoped }

#[inline]
fn contexte_desktop() -> Option<usize> {
    CHECKS.fetch_add(1, Ordering::Relaxed);
    if !mode_actif() { SKIP_MODE.fetch_add(1, Ordering::Relaxed); return None; }
    if !crate::kernel::task::current_is_kernel_task() {
        SKIP_NOT_DESKTOP.fetch_add(1, Ordering::Relaxed); return None;
    }
    if !crate::arch::x86_64::cpu::interrupts_enabled() {
        SKIP_INTERRUPTS.fetch_add(1, Ordering::Relaxed); return None;
    }
    if !crate::kernel::smp_lock::held_by_current_cpu() {
        SKIP_NO_BKL.fetch_add(1, Ordering::Relaxed); return None;
    }
    let depth = crate::kernel::smp_lock::profondeur_locale();
    if depth == 0 { SKIP_NO_BKL.fetch_add(1, Ordering::Relaxed); return None; }
    if crate::kernel::task::nom_pour_faute() != "desktop" {
        SKIP_NOT_DESKTOP.fetch_add(1, Ordering::Relaxed); return None;
    }
    Some(depth)
}

#[inline]
fn note_gap(now: u64) {
    let last = LAST_HANDOFF_NS.load(Ordering::Acquire);
    if last != 0 { GAP_MAX_NS.fetch_max(now.saturating_sub(last), Ordering::Relaxed); }
}

#[inline]
fn spins_handoff() -> usize {
    let health = crate::kernel::smp_lock::health_snapshot();
    if health.parked_mask != 0 || health.resume_mask != 0 {
        CONTENDED_RELEASES.fetch_add(1, Ordering::Relaxed);
        HANDOFF_SPINS_CONTENTION
    } else { HANDOFF_SPINS_CALME }
}

#[inline]
fn ouvre_fenetre(site: Site, accepte_imbrique: bool) -> Option<(usize, u64)> {
    let depth = contexte_desktop()?;
    if depth != 1 && !accepte_imbrique {
        SKIP_NESTED.fetch_add(1, Ordering::Relaxed);
        return None;
    }

    let now = crate::kernel::timer::monotonic_ns();
    note_gap(now);
    let contention_spins = spins_handoff();
    let started = crate::kernel::timer::monotonic_ns();
    let suspended = crate::kernel::smp_lock::suspend_for_schedule();
    if suspended != depth || suspended == 0 {
        if suspended != 0 { crate::kernel::smp_lock::resume_after_schedule(suspended); }
        SKIP_NESTED.fetch_add(1, Ordering::Relaxed);
        return None;
    }

    if depth > 1 { NESTED_SCOPES.fetch_add(1, Ordering::Relaxed); }
    MAX_SCOPE_DEPTH.fetch_max(depth as u64, Ordering::Relaxed);
    RELEASES.fetch_add(1, Ordering::Relaxed);
    SITE_RELEASES[site as usize].fetch_add(1, Ordering::Relaxed);
    for _ in 0..contention_spins { core::hint::spin_loop(); }
    HANDOFF_SPINS_TOTAL.fetch_add(contention_spins as u64, Ordering::Relaxed);
    Some((depth, started))
}

#[inline]
fn ferme_fenetre(site: Site, depth: usize, started: u64, work_started: u64) {
    let reacquire_started = crate::kernel::timer::monotonic_ns();
    crate::kernel::smp_lock::resume_after_schedule(depth);
    let done = crate::kernel::timer::monotonic_ns();
    let work_ns = reacquire_started.saturating_sub(work_started);
    let reacquire_ns = done.saturating_sub(reacquire_started);
    let window_ns = done.saturating_sub(started);
    UNLOCKED_WORK_NS.fetch_add(work_ns, Ordering::Relaxed);
    UNLOCKED_WORK_MAX_NS.fetch_max(work_ns, Ordering::Relaxed);
    SITE_UNLOCKED_NS[site as usize].fetch_add(work_ns, Ordering::Relaxed);
    REACQUIRE_WAIT_NS.fetch_add(reacquire_ns, Ordering::Relaxed);
    REACQUIRE_WAIT_MAX_NS.fetch_max(reacquire_ns, Ordering::Relaxed);
    RELEASE_WINDOW_NS.fetch_add(window_ns, Ordering::Relaxed);
    RELEASE_WINDOW_MAX_NS.fetch_max(window_ns, Ordering::Relaxed);
    LAST_HANDOFF_NS.store(done, Ordering::Release);
}

pub fn checkpoint(site: Site) {
    let Some(depth) = contexte_desktop() else { return; };
    if depth != 1 { SKIP_NESTED.fetch_add(1, Ordering::Relaxed); return; }
    let now = crate::kernel::timer::monotonic_ns();
    note_gap(now);
    let last = LAST_HANDOFF_NS.load(Ordering::Acquire);
    if last != 0 && now.saturating_sub(last) < CHECKPOINT_MIN_NS {
        SKIP_RATE.fetch_add(1, Ordering::Relaxed); return;
    }
    CHECKPOINTS.fetch_add(1, Ordering::Relaxed);
    let Some((depth, started)) = ouvre_fenetre(site, false) else { return; };
    let work_started = crate::kernel::timer::monotonic_ns();
    ferme_fenetre(site, depth, started, work_started);
}

/// Travail GUI explicitement local hors BKL. V16 autorise depth>1 et restaure
/// la profondeur exacte ; ce chemin est reserve aux scopes appeles par le
/// wrapper framebuffer/report, jamais a une mutation generique du WM.
pub fn sans_bkl<R>(site: Site, f: impl FnOnce() -> R) -> R {
    SCOPES.fetch_add(1, Ordering::Relaxed);
    let Some((depth, started)) = ouvre_fenetre(site, true) else { return f(); };
    let work_started = crate::kernel::timer::monotonic_ns();
    let result = f();
    ferme_fenetre(site, depth, started, work_started);
    result
}
