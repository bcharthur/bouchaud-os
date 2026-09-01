// --- Preemption --------------------------------------------------------------

/// Appele par IRQ0 quand le timer a interrompu du code ring 3.
///
/// On ne commute que si une autre tache est prete : sinon on economise deux
/// changements de contexte par tick.

pub fn preempt_from_irq() {
    // BOUCHAUD_SMP4_DEADLOCK_FIX
    //
    // Une IRQ ne doit jamais attendre le BKL avec IF=0. Si le verrou est
    // occupe, on differe simplement la preemption.
    debug_assert!(
        !cpu::interrupts_enabled(),
        "task: preempt_from_irq appelee hors contexte IRQ"
    );

    stall_site_set(40, 0);
    // Le BKL appartient au CPU, pas a la tache. Commuter alors que `OWNER`
    // designe encore ce CPU donnerait le verrou a la tache ENTRANTE, qui ne l'a
    // jamais demande, pendant que la pile sortante croirait toujours le tenir.
    //
    // `try_enter` est REENTRANTE : si le contexte interrompu detenait deja le
    // verrou, elle aurait rendu un garde de profondeur N+1, et le `drop`
    // ci-dessous ne serait redescendu qu'a N -- `OWNER` reste nous, et l'on
    // commute quand meme. Le commentaire « le BKL est libere AVANT le switch
    // IRQ » n'aurait alors ete vrai que par accident.
    //
    // Cet appel-ci refuse la reentrance. Ce n'est pas une regression : sur ce
    // chemin, `preempt_now` n'est arme que si l'IRQ a interrompu du RING 3
    // (`from_user`), donc un contexte qui ne peut rien detenir. Le refus est
    // donc la ceinture, pas le comportement nominal -- et s'il se declenche, il
    // se compte et se DIFFERE au lieu de casser silencieusement.
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
    let Some(kernel) = smp_lock::try_enter_depuis_zero() else {
        stall_site_clear();
        if smp_lock::held_by_current_cpu() {
            PREEMPT_IRQ_BKL_TENU.fetch_add(1, Ordering::Relaxed);
        }
        request_deferred_preempt();
        return;
    };
    stall_site_set(41, 0);

    let cur = current_index_raw();
    if cur == NO_TASK {
        return;
    }

    complete_switch_handoff();
    wake_sleepers();

    let cpu_id = local_cpu();
    // Une IRQ de quantum signifie que la tâche courante est encore runnable :
    // ce CPU n'est donc PAS idle. Voler ici une tâche distante échangeait deux
    // tâches actives entre CPU à chaque quantum et produisait le ping-pong NG4.
    // Le pull distant reste réservé au chemin schedule() lorsque la tâche
    // courante s'est réellement bloquée.
    if ready_count_cpu(cpu_id) == 0 {
        return;
    }

    let Some(next) = pick_next(cur, cpu_id) else {
        return;
    };
    if next == cur {
        return;
    }

    let (from_ptr, to_ptr) = unsafe {
        let list = tasks();
        let from_ptr = list.get_mut(cur).unwrap() as *mut Task;
        let to_ptr = list.get_mut(next).unwrap() as *mut Task;

        IRQ_PREEMPTIONS.fetch_add(1, Ordering::Relaxed);
        CONTEXT_SWITCHES.fetch_add(1, Ordering::Relaxed);
        account_slice_end(&mut *from_ptr);

        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();
        deactivate_task_space(&*from_ptr, cpu_id);
        prepare_switch_handoff(cur, &mut *from_ptr, cpu_id);
        // Meme si drop(kernel) precede le switch, l'outgoing reste resident.
        mark_task_running(&mut *to_ptr, cpu_id);

        set_current_index(next);
        install(&mut *to_ptr);
        (from_ptr, to_ptr)
    };

    // Le BKL est libere AVANT le switch IRQ. Quand cette pile IRQ sera reprise
    // plus tard avec IF=0, elle n'aura aucun BKL a reacquerir avant IRETQ.
    //
    // Le garde vient de `try_enter_depuis_zero` : sa profondeur est donc 1, et
    // ce `drop` libere reellement `OWNER`. L'assertion ci-dessous le verifie au
    // lieu de le supposer -- c'est l'invariant central de ce chemin.
    drop(kernel);
    debug_assert!(
        !smp_lock::held_by_current_cpu(),
        "task: changement de contexte IRQ alors que ce CPU detient encore le BKL"
    );
    // Le nouveau contexte ne doit pas heriter d'un tag "preempt kernel".
    stall_site_clear();
    smp_lock::note_switch(true, cur, next);
    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp); }
    smp_lock::note_switch(false, cur, next);

    // Ne jamais bloquer ici. Nettoyage opportuniste uniquement.
    stall_site_set(42, 0);
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
    if let Some(_kernel) = smp_lock::try_enter() {
        stall_site_set(43, 0);
        complete_switch_handoff();
    }
    stall_site_clear();
}

fn add_current_ticks(delta: u64) {
    let index = current_index_raw();
    if index == NO_TASK { return; }
    if let Some(task) = tasks().get_mut(index) {
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

