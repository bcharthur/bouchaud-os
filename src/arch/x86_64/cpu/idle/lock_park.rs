// Parking BKL : même handshake IF/idle que le scheduler, mais réveil provoqué
// par la libération du verrou.

pub fn prepare_lock_park() {
    debug_assert!(
        interrupts_enabled(),
        "cpu: prepare_lock_park requires IF=1"
    );
    let cpu = hardware_cpu_index();
    idle_next_seq(cpu);
    LOCK_PREPARES[cpu].fetch_add(1, Ordering::Relaxed);
    unsafe { asm!("cli", options(nomem, nostack)); }
    idle_enter(cpu);
    idle_trace_phase(cpu, IDLE_PHASE_LOCK_PREPARED);
}

pub fn commit_lock_park() {
    debug_assert!(
        !interrupts_enabled(),
        "cpu: commit_lock_park requires IF=0"
    );
    let cpu = hardware_cpu_index();
    LOCK_COMMITS[cpu].fetch_add(1, Ordering::Relaxed);
    idle_trace_phase(cpu, IDLE_PHASE_LOCK_COMMIT);

    if cpu == 0 && BSP_SAFE_IDLE_DIAGNOSTIC {
        // Diagnostic identique au scheduler : le BSP ne dort jamais sur HLT
        // lorsqu'il attend le BKL. Il retente après une courte phase PAUSE.
        LOCK_SAFE_RETURNS[cpu].fetch_add(1, Ordering::Relaxed);
        idle_trace_phase(cpu, IDLE_PHASE_LOCK_SAFE);
        idle_exit(cpu);
        unsafe { asm!("sti", options(nomem, nostack)); }
        bsp_safe_relax();
        idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
        return;
    }

    idle_trace_phase(cpu, IDLE_PHASE_LOCK_HLT);
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
    LOCK_WAKES[cpu].fetch_add(1, Ordering::Relaxed);
    idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
}

pub fn abort_lock_park() {
    debug_assert!(
        !interrupts_enabled(),
        "cpu: abort_lock_park requires IF=0"
    );
    let cpu = hardware_cpu_index();
    idle_exit(cpu);
    idle_trace_phase(cpu, IDLE_PHASE_RUNNING);
    unsafe { asm!("sti", options(nomem, nostack)); }
}

pub fn wake_parked_cpu(cpu: usize) {
    smp::reschedule_cpu(cpu);
}
