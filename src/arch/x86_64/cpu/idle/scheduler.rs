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
