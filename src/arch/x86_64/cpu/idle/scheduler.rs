// Attente CPU générale et handshake idle du scheduler.

pub fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)); }
    }
}

pub fn hlt() {
    let cpu = hardware_cpu_index();
    idle_next_seq(cpu);
    idle_trace_phase(cpu, IDLE_PHASE_GENERIC_HLT);
    idle_enter(cpu);
    unsafe { asm!("hlt", options(nostack, preserves_flags)); }
    idle_exit(cpu);
    idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
}

pub fn wait_for_interrupt() {
    let cpu = hardware_cpu_index();
    idle_next_seq(cpu);
    WFI_ENTERS[cpu].fetch_add(1, Ordering::Relaxed);
    idle_enter(cpu);

    if cpu == 0 && BSP_SAFE_IDLE_DIAGNOSTIC {
        IDLE_PHASE[cpu].store(IDLE_PHASE_WFI_SAFE, Ordering::Release);
        WFI_SAFE_RETURNS[cpu].fetch_add(1, Ordering::Relaxed);
        idle_exit(cpu);
        unsafe { asm!("sti", options(nomem, nostack)); }
        bsp_safe_relax();
        idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
        return;
    }

    idle_trace_phase(cpu, IDLE_PHASE_WFI_HLT);
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
    WFI_WAKES[cpu].fetch_add(1, Ordering::Relaxed);
    idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
}

// Scheduler sleep : PREPARE est exécuté avec le BKL encore possédé.
// Il coupe IF et publie IDLE avant que le BKL ne soit relâché.
pub fn prepare_scheduler_idle() {
    debug_assert!(
        interrupts_enabled(),
        "cpu: prepare_scheduler_idle requires IF=1"
    );
    let cpu = hardware_cpu_index();
    idle_next_seq(cpu);
    SCHED_PREPARES[cpu].fetch_add(1, Ordering::Relaxed);
    unsafe { asm!("cli", options(nomem, nostack)); }
    idle_enter(cpu);
    idle_trace_phase(cpu, IDLE_PHASE_SCHED_PREPARED);
}

pub fn commit_scheduler_idle() {
    debug_assert!(
        !interrupts_enabled(),
        "cpu: commit_scheduler_idle requires IF=0"
    );
    let cpu = hardware_cpu_index();
    SCHED_COMMITS[cpu].fetch_add(1, Ordering::Relaxed);
    idle_trace_phase(cpu, IDLE_PHASE_SCHED_COMMIT);

    // BOUCHAUD_C1_REVEIL_PERDU_IDLE_V1
    //
    // DERNIERE RELECTURE AVANT DE DORMIR
    // ----------------------------------
    // `prepare_scheduler_idle` a coupe les interruptions et publie
    // `IDLE=true`. Entre l'election qui n'a rien trouve et cette publication,
    // un autre coeur a pu faire, DANS CET ORDRE : mettre une tache dans notre
    // file, puis lire `is_idle` -- et le lire FAUX, puisque nous ne nous
    // etions pas encore declares. Il n'envoie alors aucun IPI, et nous
    // dormons avec une tache en file et personne pour venir la chercher.
    //
    // Le `sti; hlt` ci-dessous ne protege pas de cela : il ne ferme que la
    // fenetre entre `sti` et `hlt`, ou une interruption DEJA envoyee ne
    // pourrait pas se perdre. Ici, aucune interruption n'a ete envoyee.
    //
    // La relecture doit donc se faire APRES la publication de `IDLE` et
    // interruptions coupees. C'est l'ORDRE qui ferme la fenetre, pas sa
    // taille : desormais, ou bien le reveilleur voit `is_idle` vrai et envoie
    // l'IPI, ou bien nous voyons sa tache et ne dormons pas. Les deux
    // lectures sont croisees, donc au moins l'une des deux voit l'autre.
    //
    // C'est ici et non chez les appelants : les quatre chemins qui
    // s'endorment passent tous par cette fonction, et tous savent deja
    // reexaminer leur etat quand elle rend la main.
    //
    // File verrouillee (`None`) : on renonce aussi a dormir. Perdre un `hlt`
    // ne coute qu'un tour de boucle ; perdre un reveil fige la machine.
    let file_vide = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu)
        .and_then(|id| crate::arch::x86_64::cpu_local::local(id).file_non_vide_essai())
        == Some(false);
    if !file_vide {
        SCHED_ABANDONS[cpu].fetch_add(1, Ordering::Relaxed);
        idle_exit(cpu);
        unsafe { asm!("sti", options(nomem, nostack)); }
        idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
        return;
    }

    if cpu == 0 && BSP_SAFE_IDLE_DIAGNOSTIC {
        // P0 diagnostic : aucune instruction HLT sur le BSP.
        //
        // Puisque nous ne dormons pas, il n'existe plus de fenêtre de lost
        // wakeup. On rend l'état non-idle AVANT de réactiver IF, puis on laisse
        // les interruptions pendantes être livrées et on retourne au scheduler.
        SCHED_SAFE_RETURNS[cpu].fetch_add(1, Ordering::Relaxed);
        idle_trace_phase(cpu, IDLE_PHASE_SCHED_SAFE);
        idle_exit(cpu);
        unsafe { asm!("sti", options(nomem, nostack)); }
        bsp_safe_relax();
        idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
        return;
    }

    idle_trace_phase(cpu, IDLE_PHASE_SCHED_HLT);
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
    SCHED_WAKES[cpu].fetch_add(1, Ordering::Relaxed);
    idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
}
