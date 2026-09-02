// Trace du contrat scheduler <-> BKL.
//
// Cette instrumentation répond à la question que les anciens logs ne pouvaient
// pas trancher :
//
//   "resume_after_schedule a-t-il attendu longtemps ?"
//                         OU
//   "resume a réussi rapidement, puis le code repris a gardé le BKL ?"
//
// C'est la distinction essentielle pour les freezes souris/clavier observés
// sous Google. Les compteurs sont atomiques, sans allocation et ne participent
// jamais aux décisions du verrou.

#[derive(Clone, Copy)]
pub struct BklScheduleSnapshot {
    pub suspend_nonzero: u64,
    pub suspend_zero: u64,
    pub switch_before: u64,
    pub switch_after: u64,
    pub resume_begin: u64,
    pub resume_ok: u64,
    pub resume_wait_total_ns: u64,
    pub resume_wait_max_ns: u64,
    pub resume_attempts_total: u64,
    pub resume_inflight: u32,
}

#[inline]
fn note_schedule_suspend(cpu: usize, depth: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    LAST_SUSPEND_NS[cpu].store(now, Ordering::Relaxed);
    LAST_SUSPEND_DEPTH[cpu].store(depth, Ordering::Relaxed);
    if depth == 0 {
        SCHED_SUSPEND_ZERO.fetch_add(1, Ordering::Relaxed);
    } else {
        SCHED_SUSPEND_NONZERO.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
fn note_schedule_owner_released(_cpu: usize, _depth: usize) {
    // Point volontairement séparé : il permet d'ajouter plus tard une
    // génération de handoff sans toucher à suspend_for_schedule().
}

#[inline]
fn note_schedule_resume_begin(cpu: usize, depth: usize, _owner: usize) {
    SCHED_RESUME_BEGIN.fetch_add(1, Ordering::Relaxed);
    RESUME_ACTIVE_DEPTH[cpu].store(depth, Ordering::Relaxed);
    RESUME_ACTIVE_ATTEMPTS[cpu].store(0, Ordering::Relaxed);
}

#[inline]
fn note_schedule_resume_progress(cpu_reserve: usize, depth: usize, attempts: u64) {
    if cpu_reserve < MAX_CPUS {
        RESUME_ACTIVE_DEPTH[cpu_reserve].store(depth, Ordering::Relaxed);
        // Une écriture par tour serait inutilement chère sous TCG. Une mise à
        // jour sur 16 suffit au diagnostic tout en gardant le chemin chaud léger.
        if attempts & 0xF == 0 {
            RESUME_ACTIVE_ATTEMPTS[cpu_reserve].store(attempts, Ordering::Relaxed);
        }
    }
}

#[inline]
fn note_schedule_resume_ok(cpu: usize, depth: usize, wait_ns: u64, attempts: u64) {
    let now = crate::kernel::timer::monotonic_ns();
    SCHED_RESUME_OK.fetch_add(1, Ordering::Relaxed);
    SCHED_RESUME_WAIT_TOTAL_NS.fetch_add(wait_ns, Ordering::Relaxed);
    SCHED_RESUME_WAIT_MAX_NS.fetch_max(wait_ns, Ordering::Relaxed);
    SCHED_RESUME_ATTEMPTS_TOTAL.fetch_add(attempts, Ordering::Relaxed);

    LAST_RESUME_OK_NS[cpu].store(now, Ordering::Release);
    LAST_RESUME_WAIT_NS[cpu].store(wait_ns, Ordering::Relaxed);
    LAST_RESUME_DEPTH[cpu].store(depth, Ordering::Relaxed);
    LAST_RESUME_ATTEMPTS[cpu].store(attempts, Ordering::Relaxed);
    RESUME_ACTIVE_DEPTH[cpu].store(0, Ordering::Relaxed);
    RESUME_ACTIVE_ATTEMPTS[cpu].store(0, Ordering::Relaxed);
}

pub fn schedule_snapshot() -> BklScheduleSnapshot {
    BklScheduleSnapshot {
        suspend_nonzero: SCHED_SUSPEND_NONZERO.load(Ordering::Relaxed),
        suspend_zero: SCHED_SUSPEND_ZERO.load(Ordering::Relaxed),
        switch_before: SCHED_SWITCH_BEFORE.load(Ordering::Relaxed),
        switch_after: SCHED_SWITCH_AFTER.load(Ordering::Relaxed),
        resume_begin: SCHED_RESUME_BEGIN.load(Ordering::Relaxed),
        resume_ok: SCHED_RESUME_OK.load(Ordering::Relaxed),
        resume_wait_total_ns: SCHED_RESUME_WAIT_TOTAL_NS.load(Ordering::Relaxed),
        resume_wait_max_ns: SCHED_RESUME_WAIT_MAX_NS.load(Ordering::Relaxed),
        resume_attempts_total: SCHED_RESUME_ATTEMPTS_TOTAL.load(Ordering::Relaxed),
        resume_inflight: RESUME_WAITERS.load(Ordering::SeqCst).count_ones(),
    }
}

/// Marque le changement de contexte lui-même, de part et d'autre du
/// `switch_context`. L'API publique reste identique à V4.
pub fn note_switch(avant: bool, from: usize, to: usize) {
    if avant {
        SCHED_SWITCH_BEFORE.fetch_add(1, Ordering::Relaxed);
    } else {
        SCHED_SWITCH_AFTER.fetch_add(1, Ordering::Relaxed);
    }

    let cpu = cpu();
    let owner = owner_load(Ordering::Relaxed);
    let depth = depth_load(cpu, Ordering::Relaxed);
    let kind = if avant {
        enregistreur::SWITCH_BEFORE
    } else {
        enregistreur::SWITCH_AFTER
    };
    enregistreur::note(
        kind,
        cpu,
        owner,
        owner,
        depth,
        depth,
        usize::MAX,
        ((from as u64 & 0xFFFF_FFFF) << 32) | (to as u64 & 0xFFFF_FFFF),
    );
}

/// Résumé périodique du pont scheduler/BKL.
///
/// Les lignes `BKL-SCHED-OWNER` sont les plus importantes : si
/// `last_resume_wait_ns` est petit mais `post_resume_hold_ns` énorme, alors
/// `resume_after_schedule()` n'est PAS en train d'attendre. Il a rendu la main
/// et le code noyau repris conserve le BKL beaucoup trop longtemps.
pub fn log_schedule_snapshot() {
    let s = schedule_snapshot();
    crate::serial_println!(
        "[BKL-SCHED] suspend_nonzero={} suspend_zero={} switch={}/{} resume={}/{} inflight={} wait_total_ns={} wait_max_ns={} attempts_total={}",
        s.suspend_nonzero,
        s.suspend_zero,
        s.switch_before,
        s.switch_after,
        s.resume_begin,
        s.resume_ok,
        s.resume_inflight,
        s.resume_wait_total_ns,
        s.resume_wait_max_ns,
        s.resume_attempts_total,
    );

    let now = crate::kernel::timer::monotonic_ns();
    let mask = RESUME_WAITERS.load(Ordering::SeqCst);
    let mut bits = mask;
    while bits != 0 {
        let c = bits.trailing_zeros() as usize;
        bits &= bits - 1;
        if c >= MAX_CPUS {
            continue;
        }
        let since = RESUME_SINCE_NS[c].load(Ordering::Relaxed);
        crate::serial_println!(
            "[BKL-SCHED-RESUME] cpu={} age_ns={} depth={} attempts={} parked={} owner={}",
            c,
            if since == 0 { 0 } else { now.saturating_sub(since) },
            RESUME_ACTIVE_DEPTH[c].load(Ordering::Relaxed),
            RESUME_ACTIVE_ATTEMPTS[c].load(Ordering::Relaxed),
            ((PARKED.load(Ordering::SeqCst) >> c) & 1),
            owner_load(Ordering::Acquire),
        );
    }

    let provenance = stall_probe_provenance();
    if provenance.coherent && provenance.owner_token != FREE && provenance.acquire_kind == 3 {
        let owner_cpu = provenance.owner_token.saturating_sub(1);
        if owner_cpu < MAX_CPUS {
            let resume_ok = LAST_RESUME_OK_NS[owner_cpu].load(Ordering::Acquire);
            let post_resume_hold_ns = if resume_ok == 0 {
                0
            } else {
                now.saturating_sub(resume_ok)
            };
            crate::serial_println!(
                "[BKL-SCHED-OWNER] cpu={} post_resume_hold_ns={} last_resume_wait_ns={} last_depth={} last_attempts={} task={} syscall={} phase={} site={} gen={}",
                owner_cpu,
                post_resume_hold_ns,
                LAST_RESUME_WAIT_NS[owner_cpu].load(Ordering::Relaxed),
                LAST_RESUME_DEPTH[owner_cpu].load(Ordering::Relaxed),
                LAST_RESUME_ATTEMPTS[owner_cpu].load(Ordering::Relaxed),
                provenance.task,
                provenance.syscall_nr,
                provenance.syscall_phase,
                provenance.site,
                provenance.generation,
            );
        }
    }
}
