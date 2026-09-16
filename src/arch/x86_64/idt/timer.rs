// PIT handler V8 + P0-NG1 deferred kernel preemption request.

// BOUCHAUD_TIMER_NG_BALANCED_V2
//
// Variante compatible TRIGKEY BLACKBOX.
// Les reveils restent actifs a chaque tick sur CPU0 pour garder le bureau
// fluide, tandis que les scans/diagnostics lourds sont exclus du hard IRQ BSP
// lorsque plusieurs CPU sont réellement disponibles.
extern "x86-interrupt" fn timer_interrupt_handler(stack: InterruptStackFrame) {
    smp::note_premiere_irq(InterruptIndex::Timer.as_u8(), stack.instruction_pointer.as_u64(), stack.stack_pointer.as_u64());
    let _gs = GsGuard::enter(&stack);
    let interrupted_user = from_user(&stack);
    let blackbox_cpu = crate::arch::x86_64::usermode::cpu_index();
    let _blackbox_irq = crate::kernel::blackbox::timer_irq(
        blackbox_cpu,
        stack.instruction_pointer.as_u64(),
        interrupted_user,
    );

    timer::tick();
    crate::kernel::blackbox::timer_stage(blackbox_cpu, 2);

    let idle = crate::arch::x86_64::cpu::account_timer_tick(interrupted_user);
    notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    crate::kernel::blackbox::timer_stage(blackbox_cpu, 3);

    // =======================================================================
    // BOUCHAUD_TIMER_DEUX_MODES_V1 : la frontiere du contrat d'IRQ0
    // =======================================================================
    //
    // MODE AMORCAGE -- au-dessus de cette ligne :
    //     horloge, comptabilite atomique, fin d'interruption. Rien d'autre.
    //     Aucun appel ne suppose une tache courante.
    //
    // MODE RUNTIME -- au-dessous :
    //     reveils, watchdog, echantillonnage de tache, preemption,
    //     ordonnancement. Tout cela suppose que `CURRENT` existe.
    //
    // Le passage de l'un a l'autre est `smp::enable_scheduler()`, et lui seul.
    //
    // # LE DEFAUT QUE CETTE LIGNE CORRIGE
    //
    // Le test ne portait que sur `bootstrap_in_progress()`. Or ce drapeau
    // tombe dans `Drop for SmpBootstrapGuard`, JUSTE AVANT le `sti` -- alors
    // que `scheduler_enabled` est encore faux et qu'aucune tache n'est
    // installee sur le BSP. La toute premiere IRQ0 apres la restauration de
    // l'IF entrait donc dans le chemin complet, dans un etat ou `CURRENT`
    // n'existe pas : c'est exactement la frontiere ou le releve physique du
    // 14 septembre place sa double faute, `task=<aucune>`.
    //
    // Les deux drapeaux sont desormais consultes ensemble, et la barriere est
    // placee aussi tot que possible : tout ce qui la precede a ete verifie
    // ligne a ligne comme ne touchant que des atomiques.
    //
    // `account_timer_tick` RESTE au-dessus, et ce n'est pas un oubli : sa
    // lecture a montre qu'il ne manipule que des compteurs atomiques et les
    // drapeaux d'inactivite par CPU -- il ne dereference aucune tache. Le
    // laisser au-dessus garde une comptabilite de temps juste pendant tout
    // l'amorcage.
    if !politique_vecteurs::timer_runtime_pret(
        smp::bootstrap_in_progress(),
        smp::scheduler_enabled(),
    ) {
        crate::kernel::blackbox::timer_stage(blackbox_cpu, 99);
        return;
    }

    crate::kernel::task::note_rip_timer(
        stack.instruction_pointer.as_u64(),
        interrupted_user,
    );

    let ticks = timer::ticks();
    let quantum = ticks % smp::SCHED_QUANTUM_TICKS == 0;
    let online = smp::schedulable_cpus().max(1).min(64);
    let balanced_bsp = blackbox_cpu == 0 && online > 1;

    if quantum && !smp::local_scheduler_timer_enabled() {
        let targets = crate::kernel::task::running_user_cpu_mask();
        let mut target_cpu = 1usize;
        while target_cpu < online {
            if targets & (1u64 << target_cpu) != 0 {
                smp::reschedule_cpu(target_cpu);
            }
            target_cpu += 1;
        }
    }

    // BOUCHAUD_SONDES_QUI_NE_TOURNENT_NULLE_PART_V1
    //
    // Cet appel etait garde par `!balanced_bsp`, c'est-a-dire saute des que le
    // coeur zero n'est pas seul. L'intention -- sortir les diagnostics lourds
    // du hard IRQ du BSP quand d'autres coeurs sont disponibles -- suppose que
    // QUELQU'UN D'AUTRE les execute. Personne ne le fait : IRQ0 n'est livree
    // qu'au BSP, et aucun chemin AP n'appelle cette sonde. Sur toute machine a
    // plus d'un coeur, elle ne tournait donc NULLE PART.
    //
    // Consequence, mesuree sur le releve TRIGKEY du 16 septembre : zero ligne
    // `[SMP-SNAPSHOT]`, `[SCHED-FILE]` et `[SCHED-TACHE]` sur seize coeurs en
    // ligne -- precisement les trois sondes dont le commentaire de
    // `signale_etat_ordonnancement` dit qu'elles sont les seules a distinguer
    // « la tache est en file et son coeur dort » de « la tache attend quelque
    // chose qui ne vient pas ».
    //
    // Le cout etait deja borne PAR LA SONDE : elle rend la main en trois
    // instructions 999 tics sur 1000, et n'imprime qu'une fois par periode.
    // Limiter par la frequence est le bon controle ; ne jamais appeler ne l'est
    // pas.
    crate::kernel::task::stall_probe_from_timer();
    crate::kernel::blackbox::timer_stage(blackbox_cpu, 4);

    let mut preempt_now = false;
    {
        let _site = crate::kernel::task::SiteIrq::enter(60, 0);
        crate::kernel::task::stall_site_set(61, 0);

        crate::kernel::blackbox::timer_stage(blackbox_cpu, 5);

        // Le point indispensable que le timer minimal avait retire.
        crate::kernel::sync::reveil::flush_interface_irq();

        crate::kernel::blackbox::timer_stage(blackbox_cpu, 6);

        // L'ECHANTILLONNAGE RESTE GARDE, LE CHIEN DE GARDE NON.
        //
        // `echantillonne_tache_bsp` compte le temps CPU de la tache courante.
        // En SMP, le coeur zero recoit AUSSI les interruptions de quantum, et
        // `reschedule.rs` y appelle `echantillonne_quantum` : compter les deux
        // doublerait la comptabilite du BSP. Cette garde-ci est donc juste.
        //
        // `watchdog_from_timer` n'a rien a voir avec la comptabilite : il
        // surveille le battement du bureau et crie quand il s'arrete. Le
        // garder derriere la meme condition l'a desactive sur toute machine a
        // plus d'un coeur -- c'est-a-dire sur la machine de reference, et
        // exactement pendant le blocage qu'il existe pour nommer. Il coute
        // deux lectures atomiques par tic et n'imprime qu'une fois toutes les
        // dix secondes.
        if !balanced_bsp && !idle {
            crate::kernel::task::echantillonne_tache_bsp();
        }
        crate::kernel::blackbox::timer_stage(blackbox_cpu, 7);
        crate::kernel::task::watchdog_from_timer();

        crate::kernel::blackbox::timer_stage(blackbox_cpu, 8);

        if quantum && crate::kernel::task::in_user_task() {
            if interrupted_user {
                preempt_now = true;
            } else if !crate::kernel::task::current_is_kernel_task() {
                crate::kernel::scheduler::preempt::request_local();
                crate::kernel::task::request_deferred_preempt();
            }
        }
    }

    crate::kernel::blackbox::timer_stage(blackbox_cpu, 9);

    if preempt_now {
        dispatch_irq_preempt(PREEMPT_SOURCE_TIMER);
    }
}
