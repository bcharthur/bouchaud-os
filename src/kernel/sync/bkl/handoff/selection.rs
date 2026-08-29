// Lecture, expiration et filtrage d'une réservation handoff.

#[inline]
fn handoff_update_max(atom: &AtomicU64, value: u64) {
    let mut old = atom.load(Ordering::Relaxed);
    while value > old {
        match atom.compare_exchange_weak(
            old,
            value,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(now) => old = now,
        }
    }
}

#[inline]
fn handoff_age_ns(now: u64, target: usize) -> u64 {
    if target == FREE {
        return 0;
    }
    let since = HANDOFF_SINCE_NS.load(Ordering::Acquire);
    if since == 0 {
        return HANDOFF_LEASE_NS;
    }
    now.saturating_sub(since)
}

/// Retourne la cible si la réservation est encore valide.
///
/// Une réservation expirée est annulée par CAS. On ne remet pas
/// `HANDOFF_SINCE_NS` à zéro : il n'est lu que si TARGET != FREE et le prochain
/// préparateur le remplace avant de publier sa nouvelle cible. Cela évite une
/// course "ancien expirer efface le timestamp du nouveau handoff".
#[inline]
fn handoff_target_fresh() -> Option<usize> {
    loop {
        let target = HANDOFF_TARGET.load(Ordering::SeqCst);
        if target == FREE {
            return None;
        }

        let cpu = target.saturating_sub(1);
        let now = crate::kernel::timer::monotonic_ns();
        let age = handoff_age_ns(now, target);

        if cpu < MAX_CPUS && age < HANDOFF_LEASE_NS {
            return Some(cpu);
        }

        if HANDOFF_TARGET
            .compare_exchange(target, FREE, Ordering::SeqCst, Ordering::Acquire)
            .is_ok()
        {
            HANDOFF_EXPIRATIONS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    }
}

/// Filtre un nouvel entrant avant ET après son CAS OWNER.
///
/// Le second contrôle est essentiel : le libérateur peut publier le handoff
/// entre le premier contrôle du contender et son CAS. Dans ce cas le contender
/// rend OWNER et le waiter réservé conserve le prochain tour.
#[inline]
fn handoff_permet_nouvel_entrant(cpu: usize) -> bool {
    match handoff_target_fresh() {
        None => true,
        Some(target) if target == cpu => true,
        Some(_) => {
            HANDOFF_DEFERRALS.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

/// Décision prise après publication de PARKED quand OWNER est déjà libre.
///
/// Le waiter réservé peut repartir immédiatement ; les autres restent garés
/// jusqu'au claim, à l'expiration de la lease ou à une reprise prioritaire.
#[inline]
fn handoff_bloque_waiter(cpu: usize) -> bool {
    match handoff_target_fresh() {
        Some(target) if target != cpu => {
            HANDOFF_PARK_FREE_OWNER.fetch_add(1, Ordering::Relaxed);
            true
        }
        _ => false,
    }
}
