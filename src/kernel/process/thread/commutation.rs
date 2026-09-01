// --- Changement de contexte --------------------------------------------------

#[unsafe(naked)]
unsafe extern "C" fn switch_context(from: *mut u64, to: u64) {
    core::arch::naked_asm!(
        "pushfq", "push r15", "push r14", "push r13", "push r12", "push rbx", "push rbp",
        "mov [rdi], rsp", "mov rsp, rsi",
        "pop rbp", "pop rbx", "pop r12", "pop r13", "pop r14", "pop r15", "popfq", "ret",
    )
}

extern "C" fn task_trampoline() -> ! {
    let frame = {
        let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
        let _kernel = smp_lock::enter();
        complete_switch_handoff();
        let task = current();
        install(task);
        task.frame
    };
    unsafe { usermode::resume_usermode(&frame) }
}

extern "C" fn kernel_task_trampoline() -> ! {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
    let _kernel = smp_lock::enter();
    complete_switch_handoff();
    let task = current();
    task.fresh = false;
    let entree = task.entree_noyau.expect("task: fil noyau sans point d'entree");
    entree()
}

fn install(task: &mut Task) {
    unsafe {
        set_current_is_kernel(task.noyau);
        *CURRENT_PROCESS[local_cpu()].lock() = if task.noyau { None } else { Some(Arc::clone(&task.process)) };
        if !task.noyau {
            debug_assert_eq!(
                current_process_local().map(|process| process.pid), Some(task.process.pid),
                "task: stale CPU-local current Process after install"
            );
        }
        if task.noyau { crate::kernel::vmm::activate_kernel(); }
        else { task.process.mm.activate(); }
        usermode::set_kernel_stack(task.kstack_top);
        usermode::set_fs_base(task.fs_base);
        usermode::per_cpu().current = task.tid as u64;
        PID_LOCAL[local_cpu()].store(task.process.pid as u64, Ordering::Relaxed);
        usermode::fxrstor(task.fpu_ptr() as *const u8);
        task.fresh = false;
    }
}

#[inline]
fn deactivate_task_space(task: &Task, cpu_id: usize) {
    if !task.noyau { task.process.mm.mark_inactive(cpu_id); }
}

#[inline]
fn mark_task_running(task: &mut Task, cpu_id: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    assert!(task.on_cpu < 0, "task: tentative de double execution tid={}", task.tid);
    assert!(!task.switching_out,
        "task: tentative de reprendre une tache dont la passation n'est pas terminee tid={}", task.tid);
    debug_assert_eq!(task.last_account_ns, 0,
        "task: cursor CPU encore arme hors CPU tid={}", task.tid);

    if task.ready_since_ns != 0 {
        crate::kernel::scheduler::latency::record(
            now.saturating_sub(task.ready_since_ns),
            task.priorite == Priorite::Interactive,
        );
        task.ready_since_ns = 0;
    }

    if task.last_cpu != u8::MAX && task.last_cpu as usize != cpu_id {
        CPU_MIGRATIONS[cpu_id].fetch_add(1, Ordering::Relaxed);
        task.migrations = task.migrations.saturating_add(1);
        task.last_migration_ns = now;
        if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu_id) {
            crate::arch::x86_64::cpu_local::local(id).note_migration();
        }
    }
    task.last_cpu = cpu_id as u8;
    task.runq_cpu = cpu_id as u8;
    task.on_cpu = cpu_id as i8;
    task.switching_out = false;
    task.slice_start_ns = now;
    task.last_account_ns = now;
    task.context_switches = task.context_switches.saturating_add(1);
    COMPTA_DEBUT_NS[cpu_id].store(now, Ordering::Relaxed);
    COMPTA_USER_NS[cpu_id].store(0, Ordering::Relaxed);
    COMPTA_NOYAU_NS[cpu_id].store(0, Ordering::Relaxed);
    COMPTA_EN_NOYAU[cpu_id].store(task.in_kernel, Ordering::Relaxed);
    RETRAITE_DEMANDEE[cpu_id].store(task.state == TaskState::Zombie, Ordering::Release);
}
