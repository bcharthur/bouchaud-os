// Diagnostic du chemin idle/HLT.

#[derive(Clone, Copy, Debug, Default)]
pub struct IdleCpuSnapshot {
    pub cpu: usize,
    pub idle: bool,
    pub idle_age_ns: u64,
    pub phase: u8,
    pub phase_age_ns: u64,
    pub seq: u64,
    pub sched_prepares: u64,
    pub sched_commits: u64,
    pub sched_wakes: u64,
    pub sched_safe_returns: u64,
    pub lock_prepares: u64,
    pub lock_commits: u64,
    pub lock_wakes: u64,
    pub lock_safe_returns: u64,
    pub wfi_enters: u64,
    pub wfi_wakes: u64,
    pub wfi_safe_returns: u64,
    pub sleep_max_ns: u64,
}

#[inline]
pub const fn idle_phase_name(phase: u8) -> &'static str {
    match phase {
        IDLE_PHASE_SCHED_PREPARED => "sched-prepared",
        IDLE_PHASE_SCHED_COMMIT => "sched-commit",
        IDLE_PHASE_SCHED_HLT => "sched-hlt",
        IDLE_PHASE_SCHED_SAFE => "sched-safe",
        IDLE_PHASE_LOCK_PREPARED => "lock-prepared",
        IDLE_PHASE_LOCK_COMMIT => "lock-commit",
        IDLE_PHASE_LOCK_HLT => "lock-hlt",
        IDLE_PHASE_LOCK_SAFE => "lock-safe",
        IDLE_PHASE_WFI_HLT => "wfi-hlt",
        IDLE_PHASE_WFI_SAFE => "wfi-safe",
        IDLE_PHASE_GENERIC_HLT => "generic-hlt",
        _ => "running",
    }
}

pub fn idle_diagnostic_snapshot(cpu: usize) -> IdleCpuSnapshot {
    if cpu >= smp::MAX_CPUS {
        return IdleCpuSnapshot::default();
    }
    let now = crate::kernel::timer::monotonic_ns();
    let idle = IDLE[cpu].load(Ordering::Acquire);
    let idle_since = IDLE_SINCE_NS[cpu].load(Ordering::Acquire);
    let phase_since = IDLE_PHASE_SINCE_NS[cpu].load(Ordering::Acquire);
    IdleCpuSnapshot {
        cpu,
        idle,
        idle_age_ns: if idle && idle_since != 0 {
            now.saturating_sub(idle_since)
        } else {
            0
        },
        phase: IDLE_PHASE[cpu].load(Ordering::Acquire),
        phase_age_ns: if phase_since == 0 {
            0
        } else {
            now.saturating_sub(phase_since)
        },
        seq: IDLE_SEQ[cpu].load(Ordering::Relaxed),
        sched_prepares: SCHED_PREPARES[cpu].load(Ordering::Relaxed),
        sched_commits: SCHED_COMMITS[cpu].load(Ordering::Relaxed),
        sched_wakes: SCHED_WAKES[cpu].load(Ordering::Relaxed),
        sched_safe_returns: SCHED_SAFE_RETURNS[cpu].load(Ordering::Relaxed),
        lock_prepares: LOCK_PREPARES[cpu].load(Ordering::Relaxed),
        lock_commits: LOCK_COMMITS[cpu].load(Ordering::Relaxed),
        lock_wakes: LOCK_WAKES[cpu].load(Ordering::Relaxed),
        lock_safe_returns: LOCK_SAFE_RETURNS[cpu].load(Ordering::Relaxed),
        wfi_enters: WFI_ENTERS[cpu].load(Ordering::Relaxed),
        wfi_wakes: WFI_WAKES[cpu].load(Ordering::Relaxed),
        wfi_safe_returns: WFI_SAFE_RETURNS[cpu].load(Ordering::Relaxed),
        sleep_max_ns: IDLE_SLEEP_MAX_NS[cpu].load(Ordering::Relaxed),
    }
}

/// Appelé depuis le rapport BKL périodique, donc pas depuis l'IRQ PIT.
///
/// Cette séparation est volontaire : le timer ne fait que mettre à jour des
/// atomiques. L'I/O série reste dans un contexte de diagnostic déjà existant.
pub fn log_idle_snapshot() {
    let now = crate::kernel::timer::monotonic_ns();
    let pit_last = LAST_PIT_NS.load(Ordering::Acquire);
    crate::serial_println!(
        "[IDLE-DIAG] bsp_safe={} bsp_hlt={} pauses={} current_cpu={} if_current={} pit_ticks={} pit_age_ns={} idle_mask={:#x}",
        BSP_SAFE_IDLE_DIAGNOSTIC as u8,
        BSP_HLT_ENABLED as u8,
        BSP_SAFE_IDLE_PAUSES,
        hardware_cpu_index(),
        interrupts_enabled() as u8,
        PIT_TICKS_SEEN.load(Ordering::Relaxed),
        if pit_last == 0 { 0 } else { now.saturating_sub(pit_last) },
        idle_mask(),
    );

    let online = smp::schedulable_cpus().max(1).min(smp::MAX_CPUS);
    for cpu in 0..online {
        let s = idle_diagnostic_snapshot(cpu);
        crate::serial_println!(
            "[IDLE-CPU] cpu={} phase={}({}) phase_age_ns={} idle={} idle_age_ns={} seq={} sched={}/{}/{}/safe{} lock={}/{}/{}/safe{} wfi={}/{}/safe{} sleep_max_ns={}",
            cpu,
            s.phase,
            idle_phase_name(s.phase),
            s.phase_age_ns,
            s.idle as u8,
            s.idle_age_ns,
            s.seq,
            s.sched_prepares,
            s.sched_commits,
            s.sched_wakes,
            s.sched_safe_returns,
            s.lock_prepares,
            s.lock_commits,
            s.lock_wakes,
            s.lock_safe_returns,
            s.wfi_enters,
            s.wfi_wakes,
            s.wfi_safe_returns,
            s.sleep_max_ns,
        );
    }
    crate::drivers::mouse::log_diagnostic();
    crate::arch::x86_64::idt::log_preempt_irq_diagnostic();
    crate::gui::desktop_bkl::log_diagnostic();
    crate::kernel::sync::reveil::log_interface_wait_snapshot();
    crate::kernel::sync::log_wait_word_stats();
    crate::fs::persistance::log_transaction_stats();
    crate::kernel::readahead::log_stats();
}
