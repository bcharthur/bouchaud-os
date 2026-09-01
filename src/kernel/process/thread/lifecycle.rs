/// Marque la tache courante terminee et rend la main.
///
/// Si d'autres threads du programme tournent encore, on bascule sur eux ;
/// sinon, retour au fil noyau qui a lance le programme.
pub fn exit_current(code: i32) -> ! {
    {
        let task = current();
        marque_zombie(task);
        // pthread_join s'appuie sur cette ecriture suivie d'un futex_wake.
        let clear = task.clear_child_tid;
        if clear != 0 {
            let process = task.process.clone();
            process.mm.lock().space.write(clear, &0u32.to_le_bytes());
            futex_wake(clear, 1);
        }
        let process = task.process.clone();
        let mut lifecycle = process.lifecycle.lock();
        if lifecycle.threads > 0 {
            lifecycle.threads -= 1;
        }
        lifecycle.exit_code = code;
        if lifecycle.threads == 0 {
            // Dernier thread : le processus devient zombie jusqu'a ce que son
            // parent le recolte par `wait4`. C'est ce qui permet au parent de
            // recuperer le code de sortie apres coup.
            lifecycle.zombie = true;
            // Les verrous d'enregistrement POSIX meurent avec leur detenteur.
            // Un WebContent qui plante ne doit pas laisser la base SQL du
            // navigateur verrouillee pour le reste de la session.
            crate::kernel::abi::verrous::libere_processus(process.pid);
        }
    }

    // Previent le parent : SIGCHLD, et reveil s'il attendait dans `wait4`.
    notify_parent_of_exit();

    // Le programme de premier plan vient-il de se terminer ? Alors la session
    // est finie, et ce qu'il a laisse derriere lui n'a plus personne pour
    // l'attendre. C'est la semantique POSIX d'un shell : le meneur de session
    // part, le groupe de premier plan recoit SIGHUP.
    //
    // `run` faisait deja ce menage -- mais APRES son retour, c'est-a-dire
    // jamais, puisque c'est precisement ce qui l'empechait de revenir.
    let racine = RACINE_PREMIER_PLAN.load(Ordering::Acquire);
    if racine != 0 {
        let fini = {
            let process = &current().process;
            process.lifecycle.lock().zombie && process.pid == racine
        };
        if fini {
            let mut emportes = 0usize;
            for index in 0..tasks().len() {
                if tasks()[index].state == TaskState::Zombie {
                    continue;
                }
                let pid = tasks()[index].process.pid;
                if descend_de(pid, racine) {
                    marque_zombie(&mut tasks()[index]);
                    emportes += 1;
                }
            }
            if emportes > 0 {
                crate::kernel::dmesg::log_fmt(format_args!(
                    "task: pid {} termine, {} tache(s) de sa session arretees avec lui",
                    racine, emportes
                ));
            }
        }
    }

    // Sur un AP, le contexte noyau appelant est la boucle idle : si ce CPU
    // n'a plus rien de runnable, on y revient immediatement. Les autres CPU
    // continuent independamment.
    let cpu_id = local_cpu();
    if cpu_id != 0 {
        let cur = current_index_raw();
        wake_sleepers();
        if let Some(next) = pick_next(cur, cpu_id) {
            switch_to(cur, next);
            unreachable!("task: reprise d'une tache terminee sur AP");
        }
        switch_to_kernel();
    }

    // BSP : conserve la semantique historique des lancements synchrones et du
    // desktop, mais ne choisit que des taches affinees CPU0.
    let cur = current_index_raw();
    let patience = 30 * crate::kernel::timer::TICKS_PER_SECOND;
    let mut idle_since = crate::kernel::timer::ticks();
    loop {
        wake_sleepers();
        if let Some(next) = pick_next(cur, 0) {
            if next != cur {
                switch_to(cur, next);
                unreachable!("task: reprise d'une tache terminee");
            }
        }
        if tasks().iter().all(|t| t.state == TaskState::Zombie) { break; }
        if crate::kernel::timer::ticks().wrapping_sub(idle_since) > patience {
            crate::kernel::dmesg::log("task: aucune tache executable CPU0 depuis 30 s, interblocage suppose");
            for task in tasks().iter_mut() {
                if task.runq_cpu == 0 && allowed_on(task, 0) { marque_zombie(task); }
            }
            break;
        }
        // BOUCHAUD_COMPTA_IDLE_V1
        let rearmer = suspend_compta_pour_idle();
        // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
        cpu::prepare_scheduler_idle();
        let depth = smp_lock::suspend_for_schedule();
        cpu::commit_scheduler_idle();
        smp_lock::resume_after_schedule(depth);
        if rearmer {
            rearme_compta_apres_idle();
        }
        if tasks().iter().any(|t| runnable_local(t, 0) || runnable_steal(t, 0)) {
            idle_since = crate::kernel::timer::ticks();
        }
    }
    switch_to_kernel()
}

/// Signale au parent qu'un de ses fils vient de se terminer.
///
/// Deux effets distincts, tous deux necessaires : `SIGCHLD` (que le parent
/// peut avoir choisi d'intercepter) et le reveil d'un `wait4` bloquant.
fn notify_parent_of_exit() {
    let (parent_pid, is_zombie) = {
        let process = &current().process;
        (process.parent, process.lifecycle.lock().zombie)
    };
    if !is_zombie || parent_pid == 0 {
        return;
    }
    for index in 0..tasks().len() {
        if tasks()[index].state == TaskState::Zombie {
            continue;
        }
        let matches = {
            let process = &tasks()[index].process;
            if process.pid == parent_pid {
                process.signals.lock().raise(crate::kernel::signal::SIGCHLD);
                true
            } else {
                false
            }
        };
        if matches && tasks()[index].waiting_for_child {
            tasks()[index].waiting_for_child = false;
            tasks()[index].state = TaskState::Ready;
            publish_ready(index);
        }
    }
}

/// Recense les processus fils zombies d'un pid donne.
pub fn zombie_children(parent_pid: u32) -> Vec<(u32, i32)> {
    let mut out = Vec::new();
    for process in processes().iter() {
        let lifecycle = process.lifecycle.lock();
        if process.parent == parent_pid && lifecycle.zombie {
            out.push((process.pid, lifecycle.exit_code));
        }
    }
    out
}

/// Ce pid a-t-il encore des fils (zombies ou vivants) ?
pub fn has_children(parent_pid: u32) -> bool {
    processes().iter().any(|p| p.parent == parent_pid)
}

/// Retire un processus zombie de la table (il a ete recolte).
pub fn collect_child(pid: u32) {
    PROCESSES.lock().retain(|p| p.pid != pid);
    crate::kernel::process::kill(pid);
}

/// Termine tous les threads du processus courant (`exit_group`).
pub fn exit_group(code: i32) -> ! {
    let (pid, tid, process) = {
        let task = current();
        (task.process.pid, task.tid, task.process.clone())
    };
    for task in tasks().iter_mut() {
        if task.tid != tid && task.process.pid == pid {
            marque_zombie(task);
        }
    }

    // `exit_group` termine tous les autres threads du processus. Le thread
    // courant est donc le seul encore vivant; `exit_current` le decrementera
    // de 1 a 0 et rendra le processus zombie/recoltable par `wait4`.
    process.lifecycle.lock().threads = 1;

    exit_current(code)
}

/// Lance une tache depuis le fil noyau et attend la fin du programme.
///
/// Renvoie le code de sortie du processus.
///
/// # Securite
/// A n'appeler que depuis le fil noyau appelant, `CURRENT` valant `usize::MAX`.
/// `KERNEL_CTX` est unique : un appel imbrique depuis une tache y ecraserait le
/// contexte du fil qui attend deja, et `set_current_index(usize::MAX` a la sortie
/// effacerait l'identite de la tache appelante. Les appelants verifient
/// [`in_user_task`] avant d'arriver ici — voir `exec::exec_image`.
pub fn run(mut first: Box<Task>) -> i32 {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Processus);
    let _kernel = smp_lock::enter();
    // Le thread racine d'un lancement synchrone doit revenir sur la pile
    // noyau de son CPU appelant. Lui seul est pince; les pthreads qu'il cree
    // naissent avec une affinite machine complete et peuvent etre balances.
    let caller_cpu = local_cpu();
    first.affinity_mask = 1u64 << caller_cpu;
    first.runq_cpu = caller_cpu as u8;
    first.last_cpu = caller_cpu as u8;
    let process = first.process.clone();
    let racine = process.pid;
    let index = register(first);
    let cpu_id = local_cpu();
    let to_ptr = unsafe {
        RACINE_PREMIER_PLAN.store(racine, Ordering::Release);
        let list = tasks();
        let ptr = &mut **list.get_mut(index).unwrap() as *mut Task;
        mark_task_running(&mut *ptr, cpu_id);
        ptr
    };
    set_current_index(index);
    unsafe { install(&mut *to_ptr); }
    let kernel_rsp = &mut kernel_ctx().rsp as *mut u64;
    let depth = smp_lock::suspend_for_schedule();
    unsafe { switch_context(kernel_rsp, (*to_ptr).ctx.rsp); }
    smp_lock::resume_after_schedule(depth);
    complete_switch_handoff();

    crate::kernel::vmm::activate_kernel();
    set_current_index(NO_TASK);
    clear_current_process_local();
    RACINE_PREMIER_PLAN.store(0, Ordering::Release);
    let (code, pid) = {
        (process.lifecycle.lock().exit_code, process.pid)
    };
    reap();
    for stale in processes().iter() {
        crate::kernel::process::kill(stale.pid);
    }
    PROCESSES.lock().clear();
    crate::kernel::process::kill(pid);
    code
}

pub fn run_noyau(entree: fn() -> !, nom: &str) -> i32 {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Processus);
    let _kernel = smp_lock::enter();
    if in_user_task() {
        crate::kernel::dmesg::log("task: run_noyau imbrique refuse");
        return -1;
    }
    let process = match new_process(nom, 0) {
        Some(process) => process,
        None => return -1,
    };
    let mut task = Task::new_kernel(process.clone(), entree);
    task.priorite = Priorite::Interactive;
    task.affinity_mask = 1;
    task.runq_cpu = 0;
    task.last_cpu = 0;
    let index = register(task);
    let to_ptr = unsafe {
        let list = tasks();
        let ptr = &mut **list.get_mut(index).unwrap() as *mut Task;
        mark_task_running(&mut *ptr, 0);
        ptr
    };
    set_current_index(index);
    unsafe { install(&mut *to_ptr); }
    let kernel_rsp = &mut kernel_ctx().rsp as *mut u64;
    let depth = smp_lock::suspend_for_schedule();
    unsafe { switch_context(kernel_rsp, (*to_ptr).ctx.rsp); }
    smp_lock::resume_after_schedule(depth);
    complete_switch_handoff();

    crate::kernel::vmm::activate_kernel();
    set_current_index(NO_TASK);
    clear_current_process_local();
    let (code, pid) = {
        (process.lifecycle.lock().exit_code, process.pid)
    };
    reap();
    for stale in processes().iter() {
        crate::kernel::process::kill(stale.pid);
    }
    PROCESSES.lock().clear();
    crate::kernel::process::kill(pid);
    code
}

/// Detruit les taches zombies (piles noyau, espaces d'adressage).
///
/// # Securite
/// A n'appeler que depuis le fil noyau appelant, `CURRENT` valant `usize::MAX` :
/// la table est un `Vec` et `CURRENT` en est un indice. Depuis une tache,
/// utiliser [`nettoie_zombies`].
pub fn reap() {
    // Les CURRENT per-CPU sont des indices stables. En SMP on ne compacte donc
    // jamais le Vec ; `register` recycle les slots zombies. En UP, conserver le
    // comportement historique est sans risque.
    if smp::schedulable_cpus() <= 1 {
        tasks().retain(|t| t.state != TaskState::Zombie);
    }
}

pub fn nettoie_zombies() {
    if smp::schedulable_cpus() <= 1 && current_index_raw() == NO_TASK {
        reap();
    }
    // SMP : aucun deplacement d'indice ; reclamation au prochain register().
}

/// Change la classe d'ordonnancement de toutes les taches d'un processus.
///
/// Variante de [`pose_priorite`] pour un processus **autre** que le courant :
/// le gestionnaire de fenetres declare interactif le navigateur qu'il vient de
/// lancer, sans que celui-ci ait a le demander.
pub fn pose_priorite_de(pid: u32, priorite: Priorite) {
    for task in tasks().iter_mut() {
        if task.process.pid == pid {
            task.priorite = priorite;
        }
    }
}

/// Le processus est-il termine, et avec quel code ?
pub fn code_de_sortie(pid: u32) -> Option<i32> {
    processes().iter().find_map(|p| {
        let lifecycle = p.lifecycle.lock();
        if p.pid == pid && lifecycle.zombie {
            Some(lifecycle.exit_code)
        } else {
            None
        }
    })
}
/// Un processus et tous ses descendants, du plus proche au plus lointain.
///
/// Un navigateur n'est pas un processus, c'est un arbre : l'interface forke un
/// renderer par onglet, qui peut lui-meme forker. Fermer la fenetre en ne tuant
/// que la racine laisserait les renderers tourner sans personne pour lire ce
/// qu'ils produisent — du calcul pur, indefiniment, sur un cœur unique.
pub fn arbre_de(racine: u32) -> Vec<u32> {
    let mut cibles = vec![racine];
    let mut index = 0;
    while index < cibles.len() {
        let parent = cibles[index];
        for process in processes().iter() {
            if process.parent == parent && !cibles.contains(&process.pid) {
                cibles.push(process.pid);
            }
        }
        index += 1;
    }
    cibles
}

/// Termine de force toutes les taches d'un processus.
///
/// Employe quand le proprietaire d'une fenetre disparait : un client dont plus
/// personne ne compose la surface n'a plus de raison de peindre, et le laisser
/// vivre laisserait aussi vivante la surface qu'il projette.
pub fn tue_processus(pid: u32, code: i32) {
    let courant = try_current().map(|t| t.tid);
    for task in tasks().iter_mut() {
        if Some(task.tid) == courant {
            continue;
        }
        if task.process.pid == pid {
            marque_zombie(task);
        }
    }
    if let Some(process) = process_by_pid(pid) {
        let mut lifecycle = process.lifecycle.lock();
        lifecycle.threads = 0;
        lifecycle.exit_code = code;
        lifecycle.zombie = true;
    }
}

/// Termine tous les autres threads du processus courant.
///
/// Utilise par `execve` : apres le remplacement de l'image, il ne doit rester
/// qu'un fil, sinon les autres reprendraient dans un espace d'adressage qui
/// n'existe plus.
pub fn terminate_sibling_threads() {
    let (pid, tid) = {
        let task = current();
        (task.process.pid, task.tid)
    };
    for task in tasks().iter_mut() {
        if task.tid != tid && task.process.pid == pid {
            marque_zombie(task);
        }
    }
}

/// Called with the BKL immediately before returning from a syscall. An exec
/// may have retired this sibling while it ran an audited BKL-bypass syscall.
/// Ne rentre pas en espace utilisateur si la tache courante a ete tuee.
///
/// Appelee a la sortie de CHAQUE appel systeme, pour une condition qui est
/// fausse presque toujours. Elle lisait `current().state`, donc `TASKS`, donc
/// exigeait le gros verrou a chaque retour d'appel systeme.
///
/// Chemin commun : une lecture atomique d'un drapeau par CPU, rien d'autre.
///
/// Chemin rare : le drapeau est leve, on prend le gros verrou et on RELIT
/// l'etat reel avant d'agir. Le drapeau n'est donc qu'un filtre -- s'il est
/// devenu obsolete (la tache qu'il visait a deja quitte ce CPU), la relecture
/// le constate et l'on repart sans rien faire.
///
/// Une tache qui s'execute ne change pas de CPU sans passer par un changement
/// de contexte, et un zombie n'est jamais reordonnance : le drapeau du CPU
/// courant designe donc bien la tache courante.
// BOUCHAUD_V16_2_ZOMBIE_RETIRE_NONRETURNING
//
// Un sibling tué par execve peut encore se trouver dans un syscall sans BKL.
// À la sortie du syscall, il est déjà Zombie. `schedule()` n'est cependant pas
// une primitive "ne revient jamais": s'il n'existe momentanément aucune autre
// tâche locale prête, il peut rendre la main après son chemin idle. Revenir
// ensuite dans le syscall d'un zombie est interdit et provoquait le panic
// "zombie task resumed after exec quiescence".
//
// La retraite exec est différente d'un `exit_current`: execve a déjà fixé la
// comptabilité de cycle de vie du processus à un seul thread après la
// quiescence. On ne décrémente donc pas `lifecycle.threads` et on ne notifie pas
// le parent. On retire uniquement CETTE tâche de son CPU, puis on choisit une
// autre tâche locale ou on retourne définitivement au contexte noyau du CPU.
fn retire_exec_zombie_current() -> ! {
    let cpu_id = local_cpu();
    let cur = current_index_raw();

    wake_sleepers();
    if let Some(next) = pick_next(cur, cpu_id) {
        if next != cur {
            switch_to(cur, next);
            unreachable!("task: reprise d'un sibling zombie apres exec");
        }
    }

    // Aucun runnable local à cet instant. Le contexte noyau/AP idle reprendra
    // le scheduling. La pile de ce sibling ne doit plus jamais être réactivée.
    switch_to_kernel()
}

pub fn retire_current_if_zombie() {
    let cpu = interrupts::without_interrupts(local_cpu);
    if !RETRAITE_DEMANDEE[cpu].load(Ordering::Acquire) {
        return;
    }
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Processus);
    let _kernel = smp_lock::enter();
    RETRAITE_DEMANDEE[cpu].store(false, Ordering::Release);
    if in_user_task() && current().state == TaskState::Zombie {
        retire_exec_zombie_current();
    }
}

