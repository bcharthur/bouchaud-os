// --- Preemption --------------------------------------------------------------

/// Appele par IRQ0 quand le timer a interrompu du code ring 3.
///
/// On ne commute que si une autre tache est prete : sinon on economise deux
/// changements de contexte par tick.

pub fn preempt_from_irq() {
    debug_assert!(
        !cpu::interrupts_enabled(),
        "task: preempt_from_irq appelee hors contexte IRQ"
    );

    stall_site_set(40, 0);
    debug_assert_eq!(smp_lock::profondeur_locale(), 0, "preemption ring3 avec BKL");
    complete_switch_handoff();
    if !commence_transition_ordonnanceur() {
        stall_site_clear();
        request_deferred_preempt();
        return;
    }
    stall_site_set(41, 0);

    let cur = current_index_raw();
    if cur == NO_TASK {
        termine_transition_ordonnanceur();
        return;
    }

    wake_sleepers();

    let cpu_id = local_cpu();
    // Une IRQ de quantum signifie que la tâche courante est encore runnable :
    // ce CPU n'est donc PAS idle. Voler ici une tâche distante échangeait deux
    // tâches actives entre CPU à chaque quantum et produisait le ping-pong NG4.
    // Le pull distant reste réservé au chemin schedule() lorsque la tâche
    // courante s'est réellement bloquée.
    if ready_count_cpu(cpu_id) == 0 {
        termine_transition_ordonnanceur();
        return;
    }

    let Some(next) = pick_next(cur, cpu_id) else {
        termine_transition_ordonnanceur();
        return;
    };
    if next == cur {
        termine_transition_ordonnanceur();
        return;
    }

    let (from_ptr, to_ptr) = unsafe {
        let list = tasks();
        let from_ptr = unsafe { registre_pointeur_ordonnanceur(cur) }.expect("registre: tache absente");
        let to_ptr = unsafe { registre_pointeur_ordonnanceur(next) }.expect("registre: tache absente");

        IRQ_PREEMPTIONS.fetch_add(1, Ordering::Relaxed);
        CONTEXT_SWITCHES.fetch_add(1, Ordering::Relaxed);
        account_slice_end(&mut *from_ptr);

        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();
        deactivate_task_space(&*from_ptr, cpu_id);
        prepare_switch_handoff(cur, &mut *from_ptr, cpu_id);
        // Meme si drop(kernel) precede le switch, l'outgoing reste resident.
        finalise_task_running(&mut *to_ptr, cpu_id);

        set_current_index(next);
        install(&mut *to_ptr);
        (from_ptr, to_ptr)
    };

    // La porte locale reste publiee pendant le `mov rsp`. La continuation
    // entrante la rend dans `complete_switch_handoff`, avant toute execution
    // normale de la tache.
    stall_site_clear();
    smp_lock::note_switch(true, cur, next);
    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp); }
    smp_lock::note_switch(false, cur, next);

    // Quand cette pile IRQ reprend plus tard, la passation qui vient de la
    // remettre en service est deja acquittee par la continuation precedente.
    stall_site_set(42, 0);
    complete_switch_handoff();
    stall_site_clear();
}

fn add_current_ticks(delta: u64) {
    let index = current_index_raw();
    if index == NO_TASK { return; }
    if let Some(task) = tasks().get(index) {
        task.ticks_cpu.range(task.ticks_cpu.charge().wrapping_add(delta));
    }
}

pub fn echantillonne(interrupted_user: bool) {
    if cpu::account_timer_tick(interrupted_user) { return; }
    add_current_ticks(1);
}

/// Compte uniquement la tache BSP apres que l'accounting machine a deja ete
/// fait hors BKL dans l'IRQ PIT.
pub fn echantillonne_tache_bsp() {
    add_current_ticks(1);
}

/// Echantillon des AP, cadence par IPI de quantum. Le PIT reste l'unique
/// horloge murale ; ici on ne fait que comptabiliser le temps CPU de la tache.
pub fn echantillonne_quantum(_interrupted_user: bool, ticks: u64) {
    add_current_ticks(ticks.max(1));
}
