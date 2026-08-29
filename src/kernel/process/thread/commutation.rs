// --- Changement de contexte --------------------------------------------------

/// Sauvegarde RFLAGS et les registres callee-saved sur la pile courante,
/// bascule sur la pile `to`, et y restaure le tout. Le retour se fait sur
/// l'adresse empilee par la sauvegarde symetrique (ou par l'amorce de
/// [`Task::new`]).
///
/// ## Pourquoi RFLAGS fait partie du contexte
///
/// `IF` — le drapeau d'interruption — est un etat du CPU, pas de la pile : sans
/// ce `pushfq`/`popfq`, il traverse la commutation et suit la **nouvelle**
/// tache. Or les deux appelants n'ont pas le meme etat : [`schedule`] commute
/// depuis un appel systeme, interruptions actives, tandis que
/// [`preempt_from_irq`] commute depuis le gestionnaire du timer, ou le CPU les a
/// coupees en franchissant la porte d'interruption. La preemption d'une tache
/// livrait donc son `IF=0` a celle qui reprenait la main, au beau milieu de son
/// appel systeme. La suite dependait de ce qu'elle y faisait : le plus souvent
/// rien de visible — elle rendait la main en ring 3, ou `sysretq` remet un
/// RFLAGS correct —, mais si elle attendait dans un `poll`, un `futex` ou un
/// sommeil, son `hlt` arretait le CPU alors que plus aucune interruption ne
/// pouvait le reveiller. Machine gelee, sans faute ni message.
///
/// Sauvegarder RFLAGS rend chaque tache a l'etat d'interruption qui etait le
/// sien : la tache preemptee reprend dans son gestionnaire d'IRQ avec `IF=0`
/// (et c'est `iretq` qui le retablira), celle qui dormait dans un appel systeme
/// reprend avec `IF=1`.
///
/// # Securite
/// `from` doit pointer sur un `Context` valide et `to` sur une pile noyau
/// preparee par cette meme fonction ou par `Task::new`.
#[unsafe(naked)]
unsafe extern "C" fn switch_context(from: *mut u64, to: u64) {
    core::arch::naked_asm!(
        "pushfq",
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push rbx",
        "push rbp",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop rbp",
        "pop rbx",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",
        "popfq",
        "ret",
    )
}

/// Point d'entree d'une tache neuve : le scheduler a relache le BKL avant le
/// switch. On le reprend uniquement le temps d'installer l'etat materiel, puis
/// on le rend avant l'iretq utilisateur.
extern "C" fn task_trampoline() -> ! {
    let frame = {
        let _kernel = smp_lock::enter();
        complete_switch_handoff();
        let task = current();
        install(task);
        task.frame
    };
    unsafe { usermode::resume_usermode(&frame) }
}

/// Les fils noyau sont pin CPU0 et gardent le BKL pendant leur travail. Chaque
/// `schedule()` le suspend autour du changement de contexte.
extern "C" fn kernel_task_trampoline() -> ! {
    let _kernel = smp_lock::enter();
    complete_switch_handoff();
    let task = current();
    task.fresh = false;
    let entree = task.entree_noyau.expect("task: fil noyau sans point d'entree");
    entree()
}

/// Installe le contexte materiel d'une tache : espace d'adressage, pile noyau,
/// base FS (TLS) et etat FPU.
fn install(task: &mut Task) {
    unsafe {
        set_current_is_kernel(task.noyau);
        *CURRENT_PROCESS[local_cpu()].lock() = if task.noyau {
            None
        } else {
            Some(Arc::clone(&task.process))
        };
        if !task.noyau {
            debug_assert_eq!(
                current_process_local().map(|process| process.pid),
                Some(task.process.pid),
                "task: stale CPU-local current Process after install"
            );
        }
        // Un fil noyau n'a pas d'espace utilisateur a activer, et surtout ne
        // doit pas activer celui d'un programme : il lirait alors, sous les
        // memes adresses, la memoire du dernier processus installe.
        if task.noyau {
            crate::kernel::vmm::activate_kernel();
        } else {
            task.process.mm.activate();
        }
        usermode::set_kernel_stack(task.kstack_top);
        usermode::set_fs_base(task.fs_base);
        usermode::per_cpu().current = task.tid as u64;
        PID_LOCAL[local_cpu()].store(task.process.pid as u64, Ordering::Relaxed);
        // La zone est initialisee a un etat FPU valide des `Task::new` : on peut
        // restaurer inconditionnellement, y compris au premier passage.
        usermode::fxrstor(task.fpu_ptr() as *const u8);
        task.fresh = false;
    }
}

#[inline]
fn deactivate_task_space(task: &Task, cpu_id: usize) {
    if !task.noyau {
        task.process.mm.mark_inactive(cpu_id);
    }
}

#[inline]
fn mark_task_running(task: &mut Task, cpu_id: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    assert!(task.on_cpu < 0, "task: tentative de double execution tid={}", task.tid);
    assert!(
        !task.switching_out,
        "task: tentative de reprendre une tache dont la passation n'est pas terminee tid={}",
        task.tid
    );
    debug_assert_eq!(task.last_account_ns, 0, "task: cursor CPU encore arme hors CPU tid={}", task.tid);
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
    // Rearme le bloc de comptabilite de ce CPU pour la tache entrante, et
    // restaure de quel cote du mur elle s'etait arretee.
    COMPTA_DEBUT_NS[cpu_id].store(now, Ordering::Relaxed);
    COMPTA_USER_NS[cpu_id].store(0, Ordering::Relaxed);
    COMPTA_NOYAU_NS[cpu_id].store(0, Ordering::Relaxed);
    COMPTA_EN_NOYAU[cpu_id].store(task.in_kernel, Ordering::Relaxed);
    // Nouvelle tache sur ce CPU : la retraite eventuellement demandee par
    // l'ancienne ne la concerne pas.
    RETRAITE_DEMANDEE[cpu_id].store(task.state == TaskState::Zombie, Ordering::Release);
}

