/// Reveille les taches d'un processus qui dorment, pour qu'elles constatent
/// un signal en attente.
pub fn wake_for_signal(pid: u32) {
    for index in 0..tasks().len() {
        if tasks()[index].state == TaskState::Blocked
            && tasks()[index].process.pid == pid
        {
            tasks()[index].futex_key.range(0);
            tasks()[index].wait_queue_key.range(0);
            tasks()[index].wake_deadline_ns.range(0);
            tasks()[index].waiting_for_child.range(false);
            tasks()[index].state.range(TaskState::Ready);
            publish_ready(index);
        }
    }
}

// BOUCHAUD_FINAL_V12_DETACHED_WAIT
//
// Preparation is under the WaitQueue BKL guard. Finish starts only after that
// guard has been dropped, so no WaitQueue-owned KernelGuard spans schedule().

pub(crate) fn prepare_park_current_on_detached(
    wait_queue_key: usize,
    deadline_ns: Option<u64>,
) {
    debug_assert!(
        smp_lock::held_by_current_cpu(),
        "task: detached wait prepare sans BKL"
    );

    {
        let task = current();
        task.wait_queue_key.range(wait_queue_key);
        task.wake_deadline_ns.range(deadline_ns.unwrap_or(0));
        task.state.range(TaskState::Blocked);
    }

    if let Some(deadline) = deadline_ns {
        arme_echeance(deadline);
    }
}

/// Returns `(notified_before_deadline, number_of_schedule_loops)`.
pub(crate) fn finish_park_current_on_detached(
    deadline_ns: Option<u64>,
) -> (bool, u64) {
    #[cfg(debug_assertions)]
    debug_assert_eq!(
        smp_lock::profondeur_locale(),
        0,
        "task: detached wait doit commencer sans BKL"
    );

    let mut loops = 0u64;

    #[inline]
    fn trace_detached_schedule(moment: &str, boucle: u64, depth: usize) {
        let cpu = local_cpu();
        let (index, _, _, _, _) = stall_probe_context_pour(cpu);
        let tid = usermode::per_cpu_for(cpu).current;
        let pid = pid_pour_sonde(cpu);
        crate::serial_println_brut!(
            "[BKL-DETACHED] {} cpu={} task={} tid={} pid={} depth={} loop={}",
            moment, cpu, index, tid, pid, depth, boucle,
        );
    }

    loop {
        let blocked = {
            let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
            let _kernel = smp_lock::enter();
            current().state == TaskState::Blocked
        };

        if !blocked {
            break;
        }

        loops = loops.saturating_add(1);
        let depth_before = smp_lock::profondeur_locale();
        smp_lock::note_detached_check(1, loops, depth_before);
        trace_detached_schedule("before_schedule", loops, depth_before);
        schedule();
        let depth_after = smp_lock::profondeur_locale();
        smp_lock::note_detached_check(2, loops, depth_after);
        if depth_before == 0 && depth_after != 0 {
            // Geler AVANT tout formatage : les autres CPU ne peuvent plus
            // ecraser la transition fautive pendant l'impression du contexte.
            smp_lock::vide_enregistreur();
        }
        trace_detached_schedule("after_schedule", loops, depth_after);
        if depth_before == 0 && depth_after != 0 {
            crate::serial_println_brut!(
                "[BKL-DETACHED] VIOLATION schedule depth {}->{} loop={}",
                depth_before, depth_after, loops,
            );
        }
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            depth_after, depth_before,
            "task: detached wait schedule a change la profondeur BKL"
        );
    }

    let notified = match deadline_ns {
        Some(deadline) => crate::kernel::timer::monotonic_ns() < deadline,
        None => true,
    };

    {
        let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
        let _kernel = smp_lock::enter();
        let task = current();
        task.wait_queue_key.range(0);
        task.wake_deadline_ns.range(0);
    }

    let depth_final = smp_lock::profondeur_locale();
    smp_lock::note_detached_check(3, loops, depth_final);
    if depth_final != 0 {
        smp_lock::vide_enregistreur();
    }
    trace_detached_schedule("before_final_assert", loops, depth_final);
    if depth_final != 0 {
        crate::serial_println_brut!(
            "[BKL-DETACHED] VIOLATION final depth={} loops={}", depth_final, loops,
        );
    }
    #[cfg(debug_assertions)]
    debug_assert_eq!(
        depth_final,
        0,
        "task: detached wait a rendu le BKL"
    );

    (notified, loops)
}

/// Endort la tache courante sur une WaitQueue. L'appelant doit avoir valide la
/// generation sous le BKL juste avant cet appel pour fermer le lost wakeup.
pub(crate) fn park_current_on(wait_queue_key: usize) {
    {
        let task = current();
        task.wait_queue_key.range(wait_queue_key);
        task.state.range(TaskState::Blocked);
    }
    let profondeur_entree = smp_lock::profondeur_locale();
    while current().state == TaskState::Blocked {
        schedule();
    }
    verifie_profondeur_rendue("park_current_on", profondeur_entree);
    current().wait_queue_key.range(0);
}

/// Endort la tache sur une WaitQueue jusqu'a notification ou echeance.
///
/// Le deadline partage le mecanisme de `sleep_ticks`: l'IRQ timer remet la
/// tache Ready. La cle de queue reste posee jusqu'au reveil, de sorte qu'une
/// notification et l'echeance puissent courir sans perdre le reveil.
pub(crate) fn park_current_on_until(wait_queue_key: usize, deadline_ns: u64) -> bool {
    {
        let task = current();
        task.wait_queue_key.range(wait_queue_key);
        task.wake_deadline_ns.range(deadline_ns);
        task.state.range(TaskState::Blocked);
    }
    arme_echeance(deadline_ns);
    let profondeur_entree = smp_lock::profondeur_locale();
    while current().state == TaskState::Blocked {
        schedule();
    }
    verifie_profondeur_rendue("park_current_on_until", profondeur_entree);
    let notified = crate::kernel::timer::monotonic_ns() < deadline_ns;
    let task = current();
    task.wait_queue_key.range(0);
    task.wake_deadline_ns.range(0);
    notified
}

/// Reveille au plus `limit` taches inscrites sur la queue.
pub(crate) fn wake_wait_queue(wait_queue_key: usize, limit: usize) -> usize {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
    let _kernel = smp_lock::enter();
    let mut woke = 0;
    for index in 0..tasks().len() {
        if woke == limit {
            break;
        }
        if tasks()[index].state == TaskState::Blocked
            && tasks()[index].wait_queue_key == wait_queue_key
        {
            tasks()[index].wait_queue_key.range(0);
            tasks()[index].state.range(TaskState::Ready);
            publish_ready(index);
            woke += 1;
        }
    }
    woke
}

/// Y a-t-il un signal livrable pour la tache courante ?
///
/// Consulte par les attentes bloquantes (`poll`, `wait4`, futex) : une attente
/// sans limite de temps doit pouvoir etre interrompue par un signal.
pub fn signal_pending() -> bool {
    match try_current() {
        Some(task) => task.process.signals.lock().next_deliverable().is_some(),
        None => false,
    }
}

/// Termine de force toutes les taches (utilise apres une faute fatale).
pub fn kill_all(code: i32) {
    for task in tasks().iter_mut() {
        marque_zombie(task);
        task.process.lifecycle.lock().exit_code = code;
    }
}
