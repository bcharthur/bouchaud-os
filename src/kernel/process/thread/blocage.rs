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
    // L'assertion « sous gros verrou » a disparu parce que la PRECONDITION a
    // disparu, non pour faire passer un controle : ce chemin est justement
    // celui qu'on sort du verrou. Ce qui reste vrai, et qui compte, est que
    // seule la tache COURANTE se gare elle-meme -- personne d'autre n'a le
    // droit de la declarer bloquee.
    debug_assert!(
        current_index_raw() != NO_TASK,
        "task: parking demande hors de toute tache"
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

/// Annule un parking publie mais pas encore effectif.
///
/// Le protocole sans gros verrou publie `Blocked` AVANT de relire la
/// generation de la file. Quand cette relecture montre qu'un reveil est deja
/// passe, il faut defaire la publication -- sinon la tache resterait bloquee
/// en attendant un reveil qui a deja eu lieu.
pub(crate) fn annule_park_courant() {
    let task = current();
    task.wait_queue_key.range(0);
    task.wake_deadline_ns.range(0);
    task.state.range(TaskState::Ready);
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
        // Lecture d'un champ ATOMIQUE de notre PROPRE tache : le gros verrou
        // n'y apportait rien. Il etait pris, relache, et repris a chaque tour
        // de cette boucle d'attente -- des milliers de fois par seconde, pour
        // une seule instruction de chargement.
        let blocked = current().state == TaskState::Blocked;

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
        // Deux ecritures atomiques sur notre propre tache. Personne d'autre ne
        // les lit pour decider quoi que ce soit a cet instant : la tache est en
        // train de reprendre la main, donc elle n'est plus candidate au reveil.
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
/// Reveille jusqu'a `limit` taches arretees sur cette file. SANS GROS VERROU.
///
/// # Ce qui a rendu le verrou inutile
///
/// Cette boucle le prenait pour deux raisons, et les deux ont disparu.
///
/// La table pouvait se REALLOUER sous les pieds du lecteur : le registre a
/// maintenant des emplacements a adresse stable, lus sans verrou.
///
/// Le « lire l'etat, decider, ecrire l'etat » n'etait atomique que grace a
/// lui : deux CPU reveillant la meme tache l'auraient vue bloquee tous les
/// deux, et l'auraient mise deux fois en file d'execution. C'est desormais un
/// `compare_exchange` -- exactement un gagnant, par construction.
///
/// # Reveil superflu, et pourquoi il est acceptable
///
/// La cle est lue avant la transition. Une tache qui changerait de file entre
/// les deux serait reveillee pour rien. C'est sans danger : toute attente du
/// noyau reverifie sa condition en reprenant la main -- c'est la forme
/// `while etat == Blocked { schedule() }`. Rendre ce cas impossible couterait
/// un verrou par tache, pour supprimer un reveil rare et inoffensif.
pub(crate) fn wake_wait_queue(wait_queue_key: usize, limit: usize) -> usize {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Readiness);
    let mut woke = 0;
    for index in 0..registre_longueur() {
        if woke == limit {
            break;
        }
        let Some(tache) = registre_tache(index) else { continue };
        if tache.wait_queue_key != wait_queue_key {
            continue;
        }
        // Le gagnant du compare_exchange est le seul a poursuivre.
        if !tache.state.echange(TaskState::Blocked, TaskState::Ready) {
            continue;
        }
        tache.wait_queue_key.range(0);
        publish_ready(index);
        woke += 1;
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
