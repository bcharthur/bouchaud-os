// Reschedule IPI handler V8.
//
// The AP behaviour remains direct. If CPU0 ever receives a reschedule IPI while
// it was running userland, the same BSP-defer policy applies.

extern "x86-interrupt" fn reschedule_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let interrupted_user = from_user(&stack);
    crate::kernel::task::stall_ipi_observe(
        stack.instruction_pointer.as_u64(),
        interrupted_user,
    );
    smp::eoi_local();
    if smp::local_scheduler_timer_enabled() {
        smp::arm_local_scheduler_timer();
    }

    let mut preempt_now = false;
    {
        let _site = crate::kernel::task::SiteIrq::enter(30, 0);
        crate::kernel::task::stall_site_set(31, 0);

        crate::kernel::task::echantillonne_quantum(
            interrupted_user,
            smp::SCHED_QUANTUM_TICKS,
        );

        if crate::kernel::task::in_user_task() {
            if interrupted_user {
                preempt_now = true;
            } else if !crate::kernel::task::current_is_kernel_task() {
                crate::kernel::task::request_deferred_preempt();
            }
        }
    }

    if preempt_now {
        dispatch_irq_preempt(PREEMPT_SOURCE_RESCHEDULE);
    }
}
