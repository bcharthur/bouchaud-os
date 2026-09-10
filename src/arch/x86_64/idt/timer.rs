// PIT handler V8 + P0-NG1 deferred kernel preemption request.

// BOUCHAUD_TIMER_NG_BALANCED_V2
//
// Variante compatible TRIGKEY BLACKBOX.
// Les reveils restent actifs a chaque tick sur CPU0 pour garder le bureau
// fluide, tandis que les scans/diagnostics lourds sont exclus du hard IRQ BSP
// lorsque plusieurs CPU sont réellement disponibles.
extern "x86-interrupt" fn timer_interrupt_handler(stack: InterruptStackFrame) {
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

    if !balanced_bsp {
        crate::kernel::task::stall_probe_from_timer();
    }
    crate::kernel::blackbox::timer_stage(blackbox_cpu, 4);

    let mut preempt_now = false;
    {
        let _site = crate::kernel::task::SiteIrq::enter(60, 0);
        crate::kernel::task::stall_site_set(61, 0);

        crate::kernel::blackbox::timer_stage(blackbox_cpu, 5);

        // Le point indispensable que le timer minimal avait retire.
        crate::kernel::sync::reveil::flush_interface_irq();

        crate::kernel::blackbox::timer_stage(blackbox_cpu, 6);

        if !balanced_bsp {
            if !idle {
                crate::kernel::task::echantillonne_tache_bsp();
            }
            crate::kernel::blackbox::timer_stage(blackbox_cpu, 7);

            crate::kernel::task::watchdog_from_timer();
        }

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
