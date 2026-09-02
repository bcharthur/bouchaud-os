/// Choisit la prochaine tache prete apres `after`.
///
/// Un tourniquet a deux etages. On cherche d'abord une tache **interactive**
/// prete, en repartant de `after` pour que plusieurs taches interactives se
/// relaient equitablement entre elles. A defaut, on prend la premiere prete,
/// quelle qu'elle soit.
///
/// La borne `TOURS_INTERACTIFS_MAX` est ce qui separe une priorite d'une
/// famine : passe ce nombre de tours consecutifs, l'etage interactif est
/// ignore pour un tour et le tourniquet ordinaire reprend. Une tache normale
/// avance donc toujours, meme face a une tache interactive qui ne se bloque
/// jamais.
// BOUCHAUD_GATE0_POST_SWITCH_HANDOFF_V2
//
// La tache sortante n'est plus publiee au moment ou `ctx.rsp` change.
// Elle devient schedulable uniquement depuis la continuation entrante, donc
// APRES le changement physique de pile. `ctx.rsp` redevient un simple contexte
// machine et n'est plus un drapeau de synchronisation.
fn runnable_local(task: &Task, cpu: usize) -> bool {
    task.state == TaskState::Ready
        && task.on_cpu < 0
        && !task.switching_out.charge()
        && task.runq_cpu.charge() as usize == cpu
        && allowed_on(task, cpu)
}

fn runnable_steal(task: &Task, cpu: usize) -> bool {
    task.state == TaskState::Ready
        && task.on_cpu < 0
        && !task.switching_out.charge()
        && !task.noyau
        && task.runq_cpu.charge() as usize != cpu
        && allowed_on(task, cpu)
}

// Ce que l'election JETTE, par CPU.
//
// `pick_next` consomme une entree de file et peut la rejeter pour deux raisons
// tres differentes. Aucune des deux ne laissait la moindre trace, alors que la
// seconde -- une entree valide rejetee -- retire definitivement la tache de
// toute file : personne ne la republie.
static PICK_JETEE_GENERATION: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static PICK_JETEE_NON_ELIGIBLE: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

pub fn pick_jetees(cpu: usize) -> (u64, u64) {
    if cpu >= MAX_CPUS {
        return (0, 0);
    }
    (
        PICK_JETEE_GENERATION[cpu].load(Ordering::Relaxed),
        PICK_JETEE_NON_ELIGIBLE[cpu].load(Ordering::Relaxed),
    )
}

fn pick_next(_after: usize, cpu: usize) -> Option<usize> {
    let len = tasks().len();
    if len == 0 { return None; }

    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu) {
        let local = crate::arch::x86_64::cpu_local::local(id);
        while let Some(mot) = local.dequeue() {
            // Une entree dont la generation ne correspond plus designe une
            // tache MORTE dont l'emplacement a ete recycle. La servir
            // ordonnancerait la tache suivante, qui n'a rien demande : on la
            // jette.
            let identite = TacheId::depuis_mot(mot);
            let Some(tache) = registre_tache_id(identite) else {
                PICK_JETEE_GENERATION[cpu.min(MAX_CPUS - 1)]
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            };
            if identite.emplacement() < len && runnable_local(&tache, cpu) {
                return Some(identite.emplacement());
            }
            // L'entree vient d'etre CONSOMMEE et n'est pas servie. La tache
            // n'est plus dans aucune file : si elle etait encore eligible,
            // elle vient d'etre perdue.
            PICK_JETEE_NON_ELIGIBLE[cpu.min(MAX_CPUS - 1)].fetch_add(1, Ordering::Relaxed);
        }
    }

    let now = crate::kernel::timer::monotonic_ns();
    if now < STEAL_RETRY_AFTER_NS[cpu].load(Ordering::Relaxed) {
        return None;
    }
    let online = smp::schedulable_cpus().min(MAX_CPUS);
    let donor = (0..online)
        .filter(|&candidate| candidate != cpu)
        .map(|candidate| (candidate, queue_pressure(candidate)))
        // Garder une tache au donneur: en dessous de deux elements le vol ne
        // cree aucun parallelisme et ne fait que deplacer le prochain quantum.
        .filter(|(_, pressure)| *pressure > 1)
        .max_by_key(|(_, pressure)| *pressure)
        .map(|(candidate, _)| candidate);
    let Some(donor) = donor else {
        // BOUCHAUD_P1_STEAL_STERILE_BACKOFF_V1
        // Personne n'est assez charge : c'est un refus d'EQUILIBRE. Il se
        // compte -- `rej_bal` etait publie et n'etait jamais incremente, donc
        // toujours nul -- et il se temporise : rescaner tous les CPU au
        // prochain `pick_next` ne repondra pas autre chose tant que personne
        // ne s'est charge, et ce scan a lieu le gros verrou en main.
        STEAL_REJECT_BALANCE[cpu].fetch_add(1, Ordering::Relaxed);
        STEAL_RETRY_AFTER_NS[cpu]
            .store(now.saturating_add(STEAL_BACKOFF_STERILE_NS), Ordering::Relaxed);
        return None;
    };

    STEAL_ATTEMPTS[cpu].fetch_add(1, Ordering::Relaxed);

    const MIN_MIGRATION_RESIDENCY_NS: u64 = 20_000_000;
    let Some(donor_id) = crate::arch::x86_64::cpu_local::CpuId::from_index(donor) else {
        return None;
    };
    let donor_queue = crate::arch::x86_64::cpu_local::local(donor_id);
    let Some(mot) = donor_queue.steal() else {
        // La pression lue n'existait deja plus : la file s'est videe entre le
        // scan et le vol. Meme conclusion, meme temporisation.
        STEAL_REJECT_BALANCE[cpu].fetch_add(1, Ordering::Relaxed);
        STEAL_RETRY_AFTER_NS[cpu]
            .store(now.saturating_add(STEAL_BACKOFF_STERILE_NS), Ordering::Relaxed);
        return None;
    };
    let identite = TacheId::depuis_mot(mot);
    // Meme controle que ci-dessus : ne jamais voler une incarnation perimee.
    let Some(candidate) = registre_tache_id(identite) else {
        STEAL_REJECT_BALANCE[cpu].fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let index = identite.emplacement();
    if !runnable_steal(&candidate, cpu) || candidate.runq_cpu.charge() as usize != donor {
        donor_queue.enqueue(mot);
        STEAL_RETRY_AFTER_NS[cpu].store(now.saturating_add(2_000_000), Ordering::Relaxed);
        return None;
    }
    {
        if candidate.last_migration_ns != 0
            && now.saturating_sub(candidate.last_migration_ns.charge()) < MIN_MIGRATION_RESIDENCY_NS
        {
            STEAL_REJECT_AFFINITY[cpu].fetch_add(1, Ordering::Relaxed);
            donor_queue.enqueue(mot);
            STEAL_RETRY_AFTER_NS[cpu].store(
                now.saturating_add(MIN_MIGRATION_RESIDENCY_NS), Ordering::Relaxed,
            );
            return None;
        }
    }
    tasks()[index].runq_cpu.range(cpu as u8);
    RUNQ_STEALS[cpu].fetch_add(1, Ordering::Relaxed);
    STEAL_RETRY_AFTER_NS[cpu].store(0, Ordering::Relaxed);
    Some(index)
}

/// Change la classe d'ordonnancement du processus courant.
///
/// Rend l'ancienne. Toutes les taches du processus suivent : une priorite
/// s'applique a un programme, pas a l'un de ses fils — un navigateur dont
/// seule la moitie des fils serait prioritaire aurait une interface qui saccade
/// une fois sur deux.
pub fn pose_priorite(priorite: Priorite) -> Priorite {
    let pid = current().process.pid;
    let ancienne = current().priorite.charge();
    for task in tasks().iter() {
        if task.process.pid == pid {
            task.priorite.range(priorite);
        }
    }
    ancienne
}

/// La classe d'ordonnancement du processus courant.
pub fn priorite() -> Priorite {
    current().priorite.charge()
}

/// Second invariant du meme point de passage : **on ne commute jamais
/// interruptions coupees**.
///
/// [`schedule`] n'est appelee que depuis du code de tache — un appel systeme,
/// une attente volontaire —, jamais depuis un gestionnaire d'interruption (la
/// preemption sur IRQ0 passe par [`preempt_from_irq`], qui ne vient pas ici).
/// Dans ce contexte `IF` vaut toujours 1, et il le faut : la tache qui attend
/// s'arrete sur un `hlt` dont seul le tick du timer la tirera.
///
/// Un `IF=0` a cet endroit signalerait une fuite du drapeau — un `cli` sans
/// `sti`, ou une commutation qui ne rendrait pas son RFLAGS a la tache reprise
/// (voir [`switch_context`]). La panique arrive alors dans le coupable, au lieu
/// du gel silencieux qu'on constaterait autrement plusieurs instructions plus
/// loin, sur le `hlt` de la victime.
#[inline]
fn debug_assert_interrupts_enabled() {
    #[cfg(debug_assertions)]
    debug_assert!(
        cpu::interrupts_enabled(),
        "task: commutation demandee interruptions coupees — le `hlt` d'attente \
         figerait la machine (invariant : voir switch_context)"
    );
}

/// Dort jusqu'a la prochaine interruption en garantissant que le Big Kernel
/// Lock n'est jamais conserve pendant HLT.
///
/// Cette primitive est la seule autorisee depuis les attentes ABI qui dorment
/// directement (sigsuspend/pause, WASI clock poll). `syscall_dispatch` garde
/// un BKL externe pendant `abi::handle`; la suspension explicite est donc
/// obligatoire avant HLT.
pub fn wait_for_interrupt_releasing_bkl() {
    debug_assert_interrupts_enabled();
    let profondeur_entree = smp_lock::profondeur_locale();
    // BOUCHAUD_COMPTA_IDLE_V1 : replier AVANT le hlt, gros verrou encore tenu
    // -- `tasks()` l'exige.
    let rearmer = suspend_compta_pour_idle();
    // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
    cpu::prepare_scheduler_idle();
    let depth = smp_lock::suspend_for_schedule();

    #[cfg(debug_assertions)]
    debug_assert!(
        !smp_lock::held_by_current_cpu(),
        "task: HLT interdit tant que le BKL est detenu"
    );

    cpu::commit_scheduler_idle();
    smp_lock::resume_after_schedule(depth);
    if rearmer {
        rearme_compta_apres_idle();
    }
    verifie_profondeur_rendue("wait_for_interrupt_releasing_bkl", profondeur_entree);
}

// BOUCHAUD_P0_CONTRAT_PROFONDEUR_V1
//
// LE CONTRAT, ET POURQUOI IL N'ETAIT NULLE PART
// ---------------------------------------------
// Toute primitive bloquante du noyau rend la main en gardant le gros verrou a
// la profondeur exacte ou elle l'a trouve. `suspend_for_schedule` la met a
// zero, `resume_after_schedule` la restaure : le contrat est simple, et il
// n'etait verifie nulle part.
//
// La consequence pratique est ce qui a rendu
// `smp_lock: release sans acquisition` si difficile a attribuer. Une primitive
// qui perd une profondeur ne provoque AUCUNE erreur : la panique arrive plus
// tard, au Drop d'un garde quelconque -- souvent d'une autre fonction, parfois
// d'une autre tache. La victime n'est pas le coupable, et la trace accuse le
// mauvais code.
//
// Ces post-conditions transforment ce panic differe et anonyme en echec
// immediat et NOMME. Elles ne masquent rien : elles s'ajoutent aux assertions
// de `release_one`, qui restent intactes.
#[inline]
fn verifie_profondeur_rendue(site: &str, attendue: usize) {
    #[cfg(debug_assertions)]
    {
        let rendue = smp_lock::profondeur_locale();
        if rendue != attendue {
            smp_lock::vide_enregistreur();
            panic!(
                "task: {} a rendu une profondeur BKL de {} au lieu de {} \
                 (contrat de suspension rompu)",
                site, rendue, attendue,
            );
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (site, attendue);
}

/// Meme controle, mais applique a la SORTIE d'une portee, quelle qu'elle soit.
///
/// `schedule()` a plusieurs retours -- pas de tache courante, rien a elire,
/// commutation effective. Un controle ecrit a la main sur chacun d'eux se
/// serait desynchronise au premier retour ajoute ; celui-ci ne le peut pas.
struct ControleProfondeur {
    site: &'static str,
    attendue: usize,
}

impl Drop for ControleProfondeur {
    fn drop(&mut self) {
        // Le garde de `schedule()` n'est pas encore relache quand ce Drop
        // s'execute : la profondeur attendue est donc celle d'entree, PLUS le
        // niveau que `schedule()` detient encore.
        verifie_profondeur_rendue(self.site, self.attendue.saturating_add(1));
    }
}

/// Rend la main : bascule sur une autre tache prete s'il y en a une.
///
/// Renvoie `true` si un changement de tache a eu lieu. Si la tache courante est
/// la seule prete, la fonction attend une interruption (`hlt`) et rend la main a
/// l'appelant, qui doit reevaluer sa condition d'attente.
pub fn schedule() -> bool {
    let profondeur_entree = smp_lock::profondeur_locale();
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
    let _kernel = smp_lock::enter();
    let _controle = ControleProfondeur { site: "schedule", attendue: profondeur_entree };
    complete_switch_handoff();
    let cur = current_index_raw();
    if cur == NO_TASK { return false; }
    debug_assert_interrupts_enabled();
    wake_sleepers();
    let cpu_id = local_cpu();
    let next = match pick_next(cur, cpu_id) {
        Some(next) if next != cur => next,
        _ => {
            if tasks()[cur].state != TaskState::Ready {
                // Ne jamais dormir en tenant le BKL : les autres CPU doivent
                // pouvoir entrer dans leurs syscalls pendant notre HLT.
                // BOUCHAUD_COMPTA_IDLE_V1 : c'est ICI que le bureau passait
                // ses sommeils depuis Gate 1B, et c'est ce repli qui manquait.
                let rearmer = suspend_compta_pour_idle();
                // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
                cpu::prepare_scheduler_idle();
                let depth = smp_lock::suspend_for_schedule();
                #[cfg(debug_assertions)]
                debug_assert!(
                    !smp_lock::held_by_current_cpu(),
                    "task: schedule HLT interdit tant que le BKL est detenu"
                );
                cpu::commit_scheduler_idle();
                smp_lock::resume_after_schedule(depth);
                if rearmer {
                    rearme_compta_apres_idle();
                }
            }
            return false;
        }
    };
    switch_to(cur, next);
    true
}

fn switch_to(from: usize, to: usize) {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
    let _kernel = smp_lock::enter();
    complete_switch_handoff();
    let cpu_id = local_cpu();
    let (from_ptr, to_ptr) = unsafe {
        let list = tasks();
        let from_ptr = unsafe { registre_pointeur_ordonnanceur(from) }.expect("registre: tache absente");
        let to_ptr = unsafe { registre_pointeur_ordonnanceur(to) }.expect("registre: tache absente");

        CONTEXT_SWITCHES.fetch_add(1, Ordering::Relaxed);
        account_slice_end(&mut *from_ptr);
        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();
        deactivate_task_space(&*from_ptr, cpu_id);
        prepare_switch_handoff(from, &mut *from_ptr, cpu_id);
        // PAS de on_cpu=-1, PAS d'enqueue ici. Cette pile est encore active.
        mark_task_running(&mut *to_ptr, cpu_id);

        set_current_index(to);
        install(&mut *to_ptr);
        (from_ptr, to_ptr)
    };

    let depth = smp_lock::suspend_for_schedule();
    smp_lock::note_switch(true, from, to);
    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp); }
    smp_lock::note_switch(false, from, to);
    smp_lock::resume_after_schedule(depth);
    complete_switch_handoff();
}

/// Retour definitif au fil noyau appelant (la tache courante est terminee).
fn switch_to_kernel() -> ! {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
    let _kernel = smp_lock::enter();
    complete_switch_handoff();
    let cpu_id = local_cpu();
    let cur = current_index_raw();
    let from_ptr = unsafe {
        let list = tasks();
        let ptr = unsafe { registre_pointeur_ordonnanceur(cur) }.expect("registre: tache absente");
        // Replier AVANT de rendre `on_cpu` negatif : depuis que la
        // comptabilite d'appel systeme vit par CPU, c'est ici -- et non plus a
        // chaque `account_kernel_exit` -- que la derniere tranche de cette
        // tache devient durable. Sans ce repli, le temps passe entre le dernier
        // changement de contexte et la fin de la tache serait perdu, et pire,
        // impute a la tache suivante installee sur ce CPU.
        account_slice_end(&mut *ptr);
        deactivate_task_space(&*ptr, cpu_id);
        prepare_switch_handoff(cur, &mut *ptr, cpu_id);
        ptr
    };
    set_current_index(NO_TASK);
    clear_current_process_local();
    set_current_is_kernel(false);
    usermode::per_cpu().current = 0;
    crate::kernel::vmm::activate_kernel();
    let target_rsp = kernel_ctx().rsp;
    let depth = smp_lock::suspend_for_schedule();
    smp_lock::note_switch(true, cur, NO_TASK);
    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, target_rsp); }
    smp_lock::note_switch(false, cur, NO_TASK);
    smp_lock::resume_after_schedule(depth);
    unreachable!("task: reprise d'une tache terminee")
}

/// Boucle idle/scheduler des AP. Le contexte `KERNEL_CTX[cpu]` est la pile de
/// cette boucle ; `switch_to_kernel` y revient lorsqu'un processus local n'a
/// plus de tache executable.
pub fn secondary_cpu_loop() -> ! {
    let cpu_id = local_cpu();
    assert!(cpu_id != 0, "task: secondary_cpu_loop sur BSP");
    set_current_index(NO_TASK);
    clear_current_process_local();
    set_current_is_kernel(false);
    usermode::per_cpu().current = 0;

    loop {
        let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
        let _kernel = smp_lock::enter();
        stall_site_set(50, current_index_raw() as u64);
        // Avant le premier register() du BSP, ne meme pas materialiser TASKS :
        // cela permet d'activer les AP juste avant l'autorun sans mettre le boot
        // historique en concurrence avec une allocation secondaire.
        let aucune_tache = registre_longueur() == 0;
        if aucune_tache {
            // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
            cpu::prepare_scheduler_idle();
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            cpu::commit_scheduler_idle();
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
            continue;
        }
        wake_sleepers();
        if let Some(next) = pick_next(NO_TASK, cpu_id) {
            let to_ptr = unsafe {
                let list = tasks();
                let ptr = unsafe { registre_pointeur_ordonnanceur(next) }.expect("registre: tache absente");
                mark_task_running(&mut *ptr, cpu_id);
                ptr
            };
            set_current_index(next);
            unsafe { install(&mut *to_ptr); }
            let kernel_rsp = &mut kernel_ctx().rsp as *mut u64;
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            unsafe { switch_context(kernel_rsp, (*to_ptr).ctx.rsp); }
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
            stall_site_set(54, SWITCH_PENDING[cpu_id].load(Ordering::Acquire) as u64);
            complete_switch_handoff();
            stall_site_set(50, current_index_raw() as u64);
            set_current_index(NO_TASK);
            clear_current_process_local();
            set_current_is_kernel(false);
            usermode::per_cpu().current = 0;
            stall_site_set(55, 0);
            crate::kernel::vmm::activate_kernel();
            stall_site_set(50, NO_TASK as u64);
        } else {
            // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
            cpu::prepare_scheduler_idle();
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            cpu::commit_scheduler_idle();
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
        }
    }
}
