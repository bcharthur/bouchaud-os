/// Variante non bloquante, utile aux IPI de preemption : un IPI ne doit pas
/// immobiliser un coeur utilisateur entier si un autre CPU est deja dans le noyau.
pub fn try_enter() -> Option<KernelGuard> {
    // Masquer AVANT de lire l'index : une IRQ entre les deux pourrait commuter
    // et faire reprendre cette pile ailleurs. Voir `enter`.
    let _irq = LocalIrqGuard::acquire();
    let cpu = cpu();
    let mine = token(cpu);
    try_diag_begin(cpu, 1);

    try_diag_step(cpu, 601, mine as u64);
    let owner = owner_load(Ordering::Acquire);
    if owner == mine {
        let courant = etat_charge(Ordering::Acquire);
        try_diag_step(cpu, 630, courant.depth as u64);
        if courant.depth == 0 {
            crate::serial_println_brut!(
                "[BKL-FR] VIOLATION try_reenter cpu={} owner={} depth=0", cpu, owner,
            );
            vide_enregistreur();
        }
        debug_assert!(courant.depth > 0,
            "smp_lock: OWNER local sans profondeur dans try_enter");
        let (avant, apres) = augmente_profondeur(cpu)
            .expect("smp_lock: reentrance perdue dans try_enter");
        probe_note_reenter();
        try_diag_step(cpu, 631, apres as u64);
        enregistreur::note(
            enregistreur::REENTER, cpu, owner, owner, avant, apres, usize::MAX, 1,
        );
        try_diag_end(cpu, 639, apres as u64);
        return Some(KernelGuard { cpu, active: true });
    }

    if owner != FREE {
        try_diag_end(cpu, 640, owner as u64);
        return None;
    }

    try_diag_step(cpu, 602, owner as u64);
    if !essaie_prendre_nouvel_entrant(cpu, mine) {
        try_diag_end(cpu, 641, owner_load(Ordering::Relaxed) as u64);
        return None;
    }

    try_diag_step(cpu, 620, owner_load(Ordering::Relaxed) as u64);
    try_diag_step(cpu, 621, 0);
    try_diag_step(cpu, 622, 1);

    try_diag_step(cpu, 623, 0);
    probe_note_acquire(cpu, 2);
    try_diag_step(cpu, 624, 0);

    try_diag_step(cpu, 625, 0);
    enregistreur::note(
        enregistreur::TRY_ENTER, cpu, FREE, mine, 0, 1, usize::MAX, 1,
    );
    try_diag_step(cpu, 626, 0);
    try_diag_end(cpu, 627, 0);
    Some(KernelGuard { cpu, active: true })
}

/// Comme [`try_enter`], mais **refuse la reentrance**.
///
/// # Pourquoi elle existe
///
/// Le BKL appartient a un CPU, pas a une tache. Un changement de contexte
/// effectue alors que `OWNER` designe encore ce CPU donnerait la propriete du
/// verrou a la tache ENTRANTE, qui ne l'a jamais demandee, pendant que la pile
/// de la tache sortante croit toujours la detenir. Les deux se croiraient
/// proprietaires ; la premiere a relacher libererait le verrou sous les pieds
/// de l'autre.
///
/// `try_enter` est reentrante, et c'est ce qu'il faut a ses autres appelants :
/// les gestionnaires d'interruption qui veulent seulement toucher un compteur
/// sous verrou, qu'ils l'aient deja ou non. Mais la preemption depuis une IRQ,
/// elle, va COMMUTER : elle doit acquerir depuis la profondeur zero, ou ne pas
/// acquerir du tout.
///
/// Rendre `None` quand ce CPU est deja proprietaire n'est donc pas un echec :
/// c'est la reponse « pas maintenant », que l'appelant traduit en preemption
/// differee.
pub fn try_enter_depuis_zero() -> Option<KernelGuard> {
    // Masquer AVANT de lire l'index, comme `enter` et `try_enter`.
    let _irq = LocalIrqGuard::acquire();
    let cpu = cpu();
    let mine = token(cpu);
    try_diag_begin(cpu, 2);

    // Un `OWNER` non libre couvre les deux cas de refus d'un seul test : un
    // autre CPU le detient, ou c'est nous -- et nous, c'est precisement le cas
    // qu'il ne faut pas approfondir. Une continuation en reprise a egalement
    // priorite sur cette acquisition depuis zero.
    try_diag_step(cpu, 650, mine as u64);
    let owner = owner_load(Ordering::Acquire);
    if owner != FREE {
        try_diag_end(cpu, 651, owner as u64);
        return None;
    }

    try_diag_step(cpu, 652, 0);
    if !essaie_prendre_nouvel_entrant(cpu, mine) {
        try_diag_end(cpu, 653, owner_load(Ordering::Relaxed) as u64);
        return None;
    }

    try_diag_step(cpu, 660, owner_load(Ordering::Relaxed) as u64);
    try_diag_step(cpu, 661, 1);
    probe_note_acquire(cpu, 2);
    try_diag_step(cpu, 662, 0);
    enregistreur::note(
        enregistreur::TRY_ENTER, cpu, FREE, mine, 0, 1, usize::MAX, 2,
    );
    try_diag_end(cpu, 663, 0);
    Some(KernelGuard { cpu, active: true })
}
