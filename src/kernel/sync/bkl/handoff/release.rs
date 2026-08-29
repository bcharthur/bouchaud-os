// Publication et réveil du waiter ordinaire choisi.

#[inline]
fn handoff_prepare_target(target_cpu: usize) {
    if target_cpu >= MAX_CPUS {
        return;
    }

    let now = crate::kernel::timer::monotonic_ns();

    // Publier le timestamp avant la cible : un contender qui voit la cible ne
    // doit jamais la considérer immédiatement expirée faute de timestamp.
    HANDOFF_SINCE_NS.store(now, Ordering::SeqCst);
    let old = HANDOFF_TARGET.swap(token(target_cpu), Ordering::SeqCst);

    if old != FREE && old != token(target_cpu) {
        HANDOFF_REPLACEMENTS.fetch_add(1, Ordering::Relaxed);
    }
    HANDOFF_PREPARED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn handoff_send_wake(target_cpu: usize) {
    TOTAL_WAKE_IPIS.fetch_add(1, Ordering::Relaxed);
    HANDOFF_WAKEUPS.fetch_add(1, Ordering::Relaxed);
    COMPTES.note_wake(target_cpu);
    crate::arch::x86_64::cpu::wake_parked_cpu(target_cpu);
}

/// Si un handoff frais existe déjà, il est conservé jusqu'à son claim.
///
/// Ce cas arrive notamment si un contender a réussi son CAS dans la minuscule
/// fenêtre OWNER<-FREE -> publication du handoff. La réservation n'est pas
/// remplacée : le waiter sélectionné reste donc le prochain favorisé.
#[inline]
fn handoff_reveille_reserve_si_gare(releasing_cpu: usize) -> bool {
    let Some(target) = handoff_target_fresh() else {
        return false;
    };

    if target == releasing_cpu {
        // Un propriétaire qui était lui-même la cible aurait dû claim au CAS.
        // Ne jamais laisser une réservation auto-bloquante survivre.
        let _ = HANDOFF_TARGET.compare_exchange(
            token(target),
            FREE,
            Ordering::SeqCst,
            Ordering::Acquire,
        );
        HANDOFF_EXPIRATIONS.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    if PARKED.load(Ordering::SeqCst) & (1u64 << target) != 0 {
        handoff_send_wake(target);
    }
    true
}

/// Réveille à nouveau la cible après le rollback d'un barger si elle s'est
/// déjà rendormie depuis le premier IPI.
#[inline]
fn handoff_reveille_apres_rollback(releasing_cpu: usize) {
    let _ = handoff_reveille_reserve_si_gare(releasing_cpu);
}

/// Nouveau chemin ordinaire de wake : réserver AVANT l'IPI.
#[inline]
fn handoff_prepare_and_wake(target_cpu: usize) {
    handoff_prepare_target(target_cpu);
    handoff_send_wake(target_cpu);
}
