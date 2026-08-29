// PIT handler V8.
//
// Mouse V7 bottom-half remains unchanged. Only the final preemption dispatch is
// routed through the V8 gate.

extern "x86-interrupt" fn timer_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let interrupted_user = from_user(&stack);

    timer::tick();
    let idle = crate::arch::x86_64::cpu::account_timer_tick(interrupted_user);
    notify_end_of_interrupt(InterruptIndex::Timer.as_u8());

    crate::kernel::task::stall_probe_from_timer();
    let quantum = timer::ticks() % smp::SCHED_QUANTUM_TICKS == 0;

    if quantum && !smp::local_scheduler_timer_enabled() {
        let targets = crate::kernel::task::running_user_cpu_mask();
        let online = smp::schedulable_cpus().min(64);
        let mut cpu = 1usize;

        while cpu < online {
            if targets & (1u64 << cpu) != 0 {
                smp::reschedule_cpu(cpu);
            }
            cpu += 1;
        }
    }

    let mut preempt_now = false;
    {
        let _site = crate::kernel::task::SiteIrq::enter(60, 0);
        let Some(_kernel) = crate::kernel::smp_lock::try_enter() else {
            if quantum && crate::kernel::task::in_user_task() {
                crate::kernel::task::request_deferred_preempt();
            }
            return;
        };
        crate::kernel::task::stall_site_set(61, 0);

        // Mouse V7 bottom-half.
        crate::kernel::sync::reveil::flush_interface_irq_bkl_held();

        if !idle {
            crate::kernel::task::echantillonne_tache_bsp();
        }
        crate::kernel::task::watchdog_from_timer();

        if quantum && crate::kernel::task::in_user_task() {
            if interrupted_user {
                preempt_now = true;
            } else if !crate::kernel::task::current_is_kernel_task() {
                crate::kernel::task::request_deferred_preempt();
            }
        }
    }

    if preempt_now {
        dispatch_irq_preempt(PREEMPT_SOURCE_TIMER);
    }
}
