pub fn enter() -> KernelGuard {
    let mut active_spins = 0usize;
    let wait_start = crate::kernel::timer::monotonic_ns();

    loop {
        // Ne masquer les IRQ que pour le snapshot + la transition locale.
        {
            let _irq = LocalIrqGuard::acquire();
            // BOUCHAUD_P0_BKL_CPU_SOUS_MASQUE_V1
            //
            // L'index du CPU est relu a CHAQUE tour, sous interruptions
            // masquees, et non capture une fois avant la boucle. Entre deux
            // tours les interruptions sont actives : une IPI de preemption
            // peut commuter, et cette pile noyau reprendre sur un autre coeur.
            // L'index capture designerait alors un CPU etranger. Meme si
            // OWNER+DEPTH sont maintenant indivisibles, le mot atomique
            // porterait le jeton du mauvais coeur et sa liberation serait
            // attribuee a une autre continuation.
            let cpu = cpu();
            let mine = token(cpu);
            let owner = owner_load(Ordering::Acquire);

            if owner == mine {
                let courant = etat_charge(Ordering::Acquire);
                if courant.depth == 0 {
                    crate::serial_println_brut!(
                        "[BKL-FR] VIOLATION reenter cpu={} owner={} depth=0", cpu, owner,
                    );
                    vide_enregistreur();
                }
                debug_assert!(courant.depth > 0,
                    "smp_lock: OWNER local sans profondeur a la reentrance");
                let (avant, apres) = augmente_profondeur(cpu)
                    .expect("smp_lock: reentrance perdue dans enter");
                probe_note_reenter();
                enregistreur::note(
                    enregistreur::REENTER, cpu, owner, owner,
                    avant, apres, usize::MAX, 0,
                );
                return KernelGuard { cpu, active: true };
            }

            if owner == FREE && essaie_prendre_nouvel_entrant(cpu, mine) {
                // Aucun handler local ne peut voir OWNER=mine avec DEPTH=0.
                probe_note_acquire(cpu, 1);
                enregistreur::note(
                    enregistreur::ENTER, cpu, FREE, mine, 0, 1, usize::MAX, 0,
                );
                solde_parkings(cpu);
                note_attente(
                    TENUE_SYSCALL[cpu].load(Ordering::Relaxed),
                    crate::kernel::timer::monotonic_ns().saturating_sub(wait_start),
                    1,
                    cpu,
                );
                return KernelGuard { cpu, active: true };
            }
        }

        // Spin court puis HLT : ne plus bruler un coeur entier sur contention.
        wait_for_owner_change(&mut active_spins, false);
    }
}
