// Priorité de reprise et anti-barging.
//
// Une continuation qui revient d'un changement de contexte avait déjà le BKL.
// Elle est donc prioritaire sur un nouvel entrant jusqu'à restauration exacte
// de sa profondeur. Ce fichier ne contient que ce protocole.

/// Publie le CPU sur lequel cette continuation attend de restaurer son BKL.
///
/// Si une future evolution autorise une migration pendant la boucle de reprise,
/// on pose d'abord le nouveau bit puis on retire l'ancien : il n'existe ainsi
/// jamais de fenetre ou un nouvel entrant pourrait croire qu'aucune reprise
/// prioritaire n'attend.
#[inline]
fn publie_attente_reprise(cpu_reserve: &mut usize, cpu_courant: usize) {
    if *cpu_reserve == cpu_courant {
        return;
    }

    let maintenant = crate::kernel::timer::monotonic_ns();
    let depuis = if *cpu_reserve < MAX_CPUS {
        RESUME_MIGRATIONS.fetch_add(1, Ordering::Relaxed);
        RESUME_SINCE_NS[*cpu_reserve].swap(0, Ordering::Relaxed)
    } else {
        RESUME_PUBLICATIONS.fetch_add(1, Ordering::Relaxed);
        maintenant
    };

    // Publier le nouveau CPU AVANT de retirer l'ancien garde l'invariant V3 :
    // jamais de fenetre sans reservation pendant une migration.
    let apres = RESUME_WAITERS.fetch_or(1u64 << cpu_courant, Ordering::SeqCst)
        | (1u64 << cpu_courant);
    RESUME_SINCE_NS[cpu_courant].store(
        if depuis == 0 { maintenant } else { depuis },
        Ordering::Relaxed,
    );
    RESUME_WAITERS_PEAK.fetch_max(apres.count_ones(), Ordering::Relaxed);

    if *cpu_reserve < MAX_CPUS {
        RESUME_WAITERS.fetch_and(!(1u64 << *cpu_reserve), Ordering::SeqCst);
    }
    *cpu_reserve = cpu_courant;
}

/// Retire la reservation d'une continuation qui vient effectivement de
/// reacquerir OWNER. OWNER est deja non libre a cet instant : rendre la
/// priorite ne cree donc aucune fenetre d'acquisition concurrente.
#[inline]
fn retire_attente_reprise(cpu_reserve: &mut usize) {
    if *cpu_reserve < MAX_CPUS {
        RESUME_WAITERS.fetch_and(!(1u64 << *cpu_reserve), Ordering::SeqCst);
        RESUME_SINCE_NS[*cpu_reserve].store(0, Ordering::Relaxed);
        *cpu_reserve = usize::MAX;
    }
}

#[inline]
fn reprise_prioritaire_en_attente() -> bool {
    RESUME_WAITERS.load(Ordering::SeqCst) != 0
}


/// Essaie une acquisition depuis profondeur zero pour un NOUVEL entrant.
///
/// Double verification necessaire : une reprise peut publier son bit entre
/// notre premier test et le CAS sur OWNER. Si elle s'est annoncee avant notre
/// seconde verification, on rend immediatement OWNER sans ouvrir d'intervalle
/// de comptabilite et on lui laisse la priorite. Si elle s'annonce apres cette
/// verification, notre acquisition est deja logiquement anterieure et il n'y
/// a pas de barging.
#[inline]
fn essaie_prendre_nouvel_entrant(cpu: usize, mine: usize) -> bool {
    try_diag_step(cpu, 610, RESUME_WAITERS.load(Ordering::Relaxed));
    if reprise_prioritaire_en_attente() {
        PRIORITY_DEFERRALS.fetch_add(1, Ordering::Relaxed);
        try_diag_step(cpu, 618, 1);
        return false;
    }

    // V10 : ne pas passer devant le waiter ordinaire explicitement choisi.
    try_diag_step(cpu, 611, HANDOFF_TARGET.load(Ordering::Relaxed) as u64);
    if !handoff_permet_nouvel_entrant(cpu) {
        try_diag_step(cpu, 618, 2);
        return false;
    }

    try_diag_step(cpu, 612, owner_load(Ordering::Relaxed) as u64);
    if essaie_acquerir_etat(cpu, 1, Ordering::SeqCst, Ordering::Acquire).is_err() {
        try_diag_step(cpu, 618, 3);
        return false;
    }
    // À partir d'ici OWNER nous appartient. Si un freeze affiche 613..617,
    // on sait que le CAS a réussi et que la panne est dans le post-CAS.
    try_diag_step(cpu, 613, mine as u64);

    try_diag_step(cpu, 614, RESUME_WAITERS.load(Ordering::Relaxed));
    if reprise_prioritaire_en_attente() {
        PRIORITY_ROLLBACKS.fetch_add(1, Ordering::Relaxed);
        remplace_profondeur_possedee(cpu, 1, 0, Ordering::SeqCst)
            .expect("smp_lock: rollback prioritaire sans ownership");
        // La reprise peut deja etre garee. Le reveil cible privilegie
        // RESUME_WAITERS ; si elle tourne encore, aucun IPI n'est necessaire.
        wake_parked_waiters(cpu);
        try_diag_step(cpu, 618, 4);
        return false;
    }

    // Le libérateur a pu publier un handoff ENTRE notre pré-contrôle et le CAS.
    // Si nous ne sommes pas la cible, rendre immédiatement OWNER. La
    // réservation reste active et sera réveillée si elle s'est rendormie.
    try_diag_step(cpu, 615, HANDOFF_TARGET.load(Ordering::Relaxed) as u64);
    if !handoff_permet_nouvel_entrant(cpu) {
        HANDOFF_ROLLBACKS.fetch_add(1, Ordering::Relaxed);
        remplace_profondeur_possedee(cpu, 1, 0, Ordering::SeqCst)
            .expect("smp_lock: rollback handoff sans ownership");
        handoff_reveille_apres_rollback(cpu);
        try_diag_step(cpu, 618, 5);
        return false;
    }

    try_diag_step(cpu, 616, HANDOFF_TARGET.load(Ordering::Relaxed) as u64);
    handoff_claim_after_acquire(cpu);
    try_diag_step(cpu, 617, HANDOFF_TARGET.load(Ordering::Relaxed) as u64);
    true
}
