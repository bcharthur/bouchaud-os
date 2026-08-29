// Restauration du BKL après blocage/context switch.

/// Reprend le BKL avec exactement la profondeur qu'avait la tache avant son
/// changement de contexte.
pub fn resume_after_schedule(depth: usize) {
    if depth == 0 {
        return;
    }

    let wait_start = crate::kernel::timer::monotonic_ns();
    let mut spins = 0usize;
    let mut cpu_reserve = usize::MAX;
    let mut tentatives = 0u64;

    {
        let _irq = LocalIrqGuard::acquire();
        let cpu = cpu();
        publie_attente_reprise(&mut cpu_reserve, cpu);
        let owner = OWNER.load(Ordering::Relaxed);
        note_schedule_resume_begin(cpu, depth, owner);
        enregistreur::note(
            enregistreur::RESUME_BEGIN,
            cpu,
            owner,
            owner,
            DEPTH[cpu].load(Ordering::Relaxed),
            depth,
            usize::MAX,
            depth as u64,
        );
    }

    loop {
        {
            let _irq = LocalIrqGuard::acquire();
            // Relu a chaque tour, sous masque. Une pile REPRISE est justement
            // celle qui vient de changer de coeur : capturer l'index une fois
            // avant la boucle, alors que l'attente ci-dessous peut a son tour
            // etre preemptee, est la faute la plus facile a commettre ici.
            let cpu = cpu();
            let mine = token(cpu);
            publie_attente_reprise(&mut cpu_reserve, cpu);
            RESUME_ACTIVE_DEPTH[cpu].store(depth, Ordering::Relaxed);
            RESUME_ACTIVE_ATTEMPTS[cpu].store(tentatives, Ordering::Relaxed);

            if OWNER
                .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Aucun handler local ne peut observer OWNER=mine avant que
                // la profondeur de la pile reprise soit restauree.
                let avant = DEPTH[cpu].load(Ordering::Relaxed);
                DEPTH[cpu].store(depth, Ordering::Relaxed);
                // Une continuation scheduler reste plus prioritaire que tout
                // handoff ordinaire. OWNER est déjà à nous : l'annulation ne
                // crée aucune fenêtre de barging.
                handoff_cancel_for_resume();
                retire_attente_reprise(&mut cpu_reserve);
                probe_note_acquire(cpu, 3);
                enregistreur::note(
                    enregistreur::RESUME_OK, cpu, FREE, mine,
                    avant, depth, usize::MAX, depth as u64,
                );
                solde_parkings(cpu);
                let attente =
                    crate::kernel::timer::monotonic_ns().saturating_sub(wait_start);
                note_attente(
                    TENUE_SYSCALL[cpu].load(Ordering::Relaxed), attente, 3, cpu,
                );
                // La MEME attente, isolee : c'est la reprise apres commutation
                // qu'on soupconne de durer des secondes, et un cumul global la
                // noierait dans celui de tous les `enter` du systeme.
                COMPTES.note_reprise(attente);
                note_schedule_resume_ok(cpu, depth, attente, tentatives);
                return;
            }
        }

        // BOUCHAUD_BKL_RESUME_PARK_V1
        //
        // # Ce que le busy-wait coutait
        //
        // Cette boucle etait un `spin_loop()` pur, sans jamais se garer. La
        // raison invoquee -- « avec des IPI d'ordonnancement cibles, plus aucun
        // battement de 4 ms ne garantit le reveil » -- ne tient pas : chaque
        // liberation appelle `wake_parked_waiters`, que ce soit par
        // `release_one` ou par `suspend_for_schedule`. Un CPU inscrit dans
        // `PARKED` est donc TOUJOURS reveille, et le protocole de publication
        // ci-dessous ferme la course du reveil perdu.
        //
        // Le prix, lui, etait bien reel. Sous TCG les quatre vCPU se partagent
        // les coeurs de l'hote : un vCPU qui tourne a vide vole le temps dont
        // le DETENTEUR a besoin pour finir. Plus il attend, plus il tient. Le
        // runtime le montrait sans ambiguite :
        //
        //     [SMP-PROV] owner=1 held=690ms depth=1 syscall=poll/attente
        //     [BKL-MAX-HOLD] ns=29562372510 origine=resume_after_schedule
        //     window_ns=11353070412   <- une fenetre de 5 s en a pris 11
        //     [gui] client actif 0 trames (silence 61818 ms)
        //
        // Une tenue de 690 ms, une pointe a 29 secondes, et le compositeur
        // muet pendant une minute : c'est le figement ressenti au defilement.
        //
        // `wait_for_owner_change` garde un court spin actif -- une reprise est
        // le plus souvent immediate, le verrou vient d'etre relache par
        // nous-memes -- puis se gare. Il refuse de lui-meme de dormir dans un
        // contexte a interruptions masquees, ou dormir serait fatal, et relit
        // l'index de son CPU une fois les interruptions masquees.
        tentatives = tentatives.saturating_add(1);
        note_schedule_resume_progress(cpu_reserve, depth, tentatives);
        wait_for_owner_change(&mut spins, true);
    }
}

