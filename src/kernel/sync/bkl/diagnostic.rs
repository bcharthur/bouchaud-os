// BOUCHAUD_BKL_HEALTH_V4
//
// Snapshot coherent "assez" pour le diagnostic : toutes les grandeurs sont
// atomiques et aucune n'est utilisee pour prendre une decision de correction.
// Le but est de distinguer trois cas qui se ressemblent visuellement :
//  1) BKL reellement bloque,
//  2) continuation affamee dans resume_after_schedule,
//  3) machine active ailleurs (page faults, calcul user, I/O) alors que le BKL
//     est libre.
//
// Aucune allocation, aucun verrou BKL, aucun parcours de structure mutable.

#[derive(Clone, Copy)]
pub struct BklHealth {
    pub owner_token: usize,
    pub owner_cpu: usize,
    pub owner_depth: usize,
    pub parked_mask: u64,
    pub resume_mask: u64,
    pub resume_oldest_ns: u64,
    pub resume_peak: u32,
    pub resume_publications: u64,
    pub resume_migrations: u64,
    pub priority_deferrals: u64,
    pub priority_rollbacks: u64,
    pub priority_wakeups: u64,
    pub priority_wake_suppressed: u64,
    pub priority_park_free_owner: u64,
    pub owner_depth_ok: bool,
    pub multiple_depth_owners: bool,
}

#[inline]
fn oldest_resume_age_ns(now: u64, mask: u64) -> u64 {
    let mut oldest = 0u64;
    let mut bits = mask;
    while bits != 0 {
        let cpu = bits.trailing_zeros() as usize;
        bits &= bits - 1;
        if cpu >= MAX_CPUS {
            continue;
        }
        let since = RESUME_SINCE_NS[cpu].load(Ordering::Relaxed);
        if since != 0 {
            oldest = oldest.max(now.saturating_sub(since));
        }
    }
    oldest
}

pub fn health_snapshot() -> BklHealth {
    let owner_token = OWNER.load(Ordering::Acquire);
    let owner_cpu = if owner_token == FREE {
        usize::MAX
    } else {
        owner_token.saturating_sub(1)
    };

    let mut depth_nonzero = 0usize;
    let mut owner_depth = 0usize;
    for cpu in 0..MAX_CPUS {
        let depth = DEPTH[cpu].load(Ordering::Acquire);
        if depth != 0 {
            depth_nonzero += 1;
        }
        if cpu == owner_cpu {
            owner_depth = depth;
        }
    }

    let owner_depth_ok = if owner_token == FREE {
        depth_nonzero == 0
    } else {
        owner_cpu < MAX_CPUS && owner_depth > 0 && depth_nonzero == 1
    };

    let resume_mask = RESUME_WAITERS.load(Ordering::SeqCst);
    let now = crate::kernel::timer::monotonic_ns();

    BklHealth {
        owner_token,
        owner_cpu,
        owner_depth,
        parked_mask: PARKED.load(Ordering::SeqCst),
        resume_mask,
        resume_oldest_ns: oldest_resume_age_ns(now, resume_mask),
        resume_peak: RESUME_WAITERS_PEAK.load(Ordering::Relaxed),
        resume_publications: RESUME_PUBLICATIONS.load(Ordering::Relaxed),
        resume_migrations: RESUME_MIGRATIONS.load(Ordering::Relaxed),
        priority_deferrals: PRIORITY_DEFERRALS.load(Ordering::Relaxed),
        priority_rollbacks: PRIORITY_ROLLBACKS.load(Ordering::Relaxed),
        priority_wakeups: PRIORITY_WAKEUPS.load(Ordering::Relaxed),
        priority_wake_suppressed: PRIORITY_WAKE_SUPPRESSED.load(Ordering::Relaxed),
        priority_park_free_owner: PRIORITY_PARK_FREE_OWNER.load(Ordering::Relaxed),
        owner_depth_ok,
        multiple_depth_owners: depth_nonzero > 1,
    }
}

/// Emet une ligne compacte, puis le contexte des CPU actifs.
/// Appelee par `comptes()`, donc au meme rythme que `[BKL-COMPTES]`.
pub fn log_health_snapshot() {
    let h = health_snapshot();
    crate::serial_println!(
        "[BKL-HEALTH] owner={} owner_cpu={} depth={} owner_depth_ok={} multi_depth={} parked={:#x} resume={:#x} resume_oldest_ns={} resume_peak={} publications={} migrations={} deferrals={} rollbacks={} prio_wakes={} prio_wake_suppressed={} park_free_owner={}",
        h.owner_token,
        h.owner_cpu,
        h.owner_depth,
        h.owner_depth_ok as u8,
        h.multiple_depth_owners as u8,
        h.parked_mask,
        h.resume_mask,
        h.resume_oldest_ns,
        h.resume_peak,
        h.resume_publications,
        h.resume_migrations,
        h.priority_deferrals,
        h.priority_rollbacks,
        h.priority_wakeups,
        h.priority_wake_suppressed,
        h.priority_park_free_owner,
    );

    // Vue dédiée du contrat waiter ordinaire -> réservation -> claim.
    log_handoff_snapshot();

    // Vue dédiée du contrat suspend -> switch -> resume.
    log_schedule_snapshot();

    // Les sites 20x/24x appartiennent au demand paging. Les voir progresser
    // pendant que owner=0 est la preuve que l'ecran peut sembler fige alors que
    // le noyau travaille hors BKL.
    let online = crate::arch::x86_64::smp::schedulable_cpus().min(MAX_CPUS);
    for cpu in 0..online {
        let (task, syscall, phase, site, aux) =
            crate::kernel::task::stall_probe_context_pour(cpu);
        if task != usize::MAX || site != 0 || syscall != u64::MAX {
            crate::serial_println!(
                "[BKL-CPU] cpu={} task={} syscall={} phase={} site={} aux={:#x} depth={}",
                cpu,
                task,
                syscall,
                phase,
                site,
                aux,
                DEPTH[cpu].load(Ordering::Acquire),
            );
        }
    }

    // Seuil volontairement diagnostique : aucune correction automatique.
    // Effacer une reservation "stale" sans savoir si la continuation vit
    // encore recreerait exactement les races que V3 a fermees.
    if !h.owner_depth_ok || h.resume_oldest_ns >= 50_000_000 {
        crate::kernel::perf::note_bkl_alert(h.owner_token as u64, h.resume_oldest_ns);
        crate::serial_println!(
            "[BKL-ALERTE] invariant_ok={} resume_oldest_ns={} owner={} parked={:#x} resume={:#x}",
            h.owner_depth_ok as u8,
            h.resume_oldest_ns,
            h.owner_token,
            h.parked_mask,
            h.resume_mask,
        );
    }
}
