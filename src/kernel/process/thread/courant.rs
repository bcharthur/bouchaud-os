#[inline]
fn set_current_index(index: usize) {
    CURRENT[local_cpu()].store(index, Ordering::Release);
}

#[inline]
fn set_current_is_kernel(value: bool) {
    CURRENT_IS_KERNEL[local_cpu()].store(value, Ordering::Release);
}

#[inline]
fn kernel_ctx() -> &'static mut Context {
    unsafe { &mut KERNEL_CTX[local_cpu()] }
}

/// RSP physique courant, uniquement pour verifier l'invariant de passation.
#[inline]
fn rsp_courant_passation() -> u64 {
    let rsp: u64;
    unsafe {
        core::arch::asm!(
            "mov {}, rsp",
            out(reg) rsp,
            options(nomem, nostack, preserves_flags)
        );
    }
    rsp
}

/// Prepare la sortie d'une tache SANS la publier.
///
/// Le BKL est tenu. La tache reste `on_cpu == cpu` et n'est pas mise en file.
/// C'est volontaire : `switch_context` sauvegarde d'abord son RSP puis change
/// de pile ; entre ces deux instructions, l'ancien CPU utilise encore la pile.
#[inline]
fn prepare_switch_handoff(index: usize, task: &mut Task, cpu: usize) {
    debug_assert!(
        smp_lock::held_by_current_cpu(),
        "task: preparation de passation sans BKL"
    );
    assert_eq!(
        task.on_cpu,
        cpu as i8,
        "task: passation d'une tache non residente sur ce CPU tid={}",
        task.tid
    );
    assert!(
        !task.switching_out.charge(),
        "task: double preparation de passation tid={}",
        task.tid
    );

    match SWITCH_PENDING[cpu].compare_exchange(
        NO_TASK,
        index,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => task.switching_out.range(true),
        Err(previous) => panic!(
            "task: passation precedente non terminee cpu={} pending={} nouveau={}",
            cpu, previous, index
        ),
    }
}

/// Publie la tache sortante depuis la pile ENTRANTE.
///
/// C'est LE point qui ferme Gate 0 : `on_cpu` ne devient negatif qu'ici.
/// Un reveil concurrent pendant la commutation peut mettre `state=Ready`, mais
/// `publish_ready()` refuse encore la tache tant que `on_cpu >= 0`. Ici, apres
/// abandon physique de l'ancienne pile, on republie exactement une fois.
fn complete_switch_handoff() {
    let cpu = local_cpu();
    let outgoing = SWITCH_PENDING[cpu].load(Ordering::Acquire);
    if outgoing == NO_TASK {
        return;
    }

    debug_assert!(
        smp_lock::held_by_current_cpu(),
        "task: completion de passation sans BKL"
    );
    assert!(
        outgoing < tasks().len(),
        "task: passation vers slot invalide cpu={} slot={}",
        cpu,
        outgoing
    );

    let publish = {
        let task = &mut tasks()[outgoing];

        assert!(
            task.switching_out.charge(),
            "task: passation pending sans switching_out tid={}",
            task.tid
        );
        assert_eq!(
            task.on_cpu,
            cpu as i8,
            "task: outgoing publie avant completion tid={} on_cpu={} cpu={}",
            task.tid,
            task.on_cpu,
            cpu
        );

        #[cfg(debug_assertions)]
        {
            let rsp = rsp_courant_passation();
            let base = task.kstack_top.saturating_sub(KSTACK_SIZE as u64);
            debug_assert!(
                rsp < base || rsp >= task.kstack_top,
                "task: publication avant abandon physique de la pile tid={} rsp={:#x} pile={:#x}..{:#x}",
                task.tid,
                rsp,
                base,
                task.kstack_top
            );
        }

        task.last_cpu = cpu as u8;
        task.runq_cpu.range(cpu as u8);
        task.on_cpu.range(-1);
        task.switching_out.range(false);
        task.state == TaskState::Ready
    };

    SWITCH_PENDING[cpu].store(NO_TASK, Ordering::Release);

    if publish {
        publish_ready(outgoing);
    }
}

/// PID du programme lance au premier plan par [`run`], 0 si aucun.
///
/// Un `exec` synchrone doit rendre la main quand CE programme se termine. Sans
/// ce reperage, `exit_current` n'avait qu'un seul critere -- « plus aucune
/// tache executable » -- et un programme qui laisse des fils vivants ne rendait
/// donc JAMAIS la main : les fils tournent, l'ordonnanceur a toujours quelqu'un
/// a servir, et l'invite ne revient pas.
///
/// C'est ce qui est arrive au run 32427953935. `BouchaudBrowserHost` quitte
/// proprement sur `window.close()`, mais ses services -- WebContent,
/// RequestServer, ImageDecoder, Compositor -- restent dans leur boucle
/// d'evenements. L'autorun ne reprenait pas, donc `power::shutdown` n'etait
/// jamais appele, donc /persist n'etait jamais ecrit a l'extinction.
static RACINE_PREMIER_PLAN: AtomicU32 = AtomicU32::new(0);

/// Le processus `pid` descend-il de `racine` (ou est-il `racine`) ?
fn descend_de(pid: u32, racine: u32) -> bool {
    let mut courant = pid;
    // La table est finie et un cycle de filiation serait une corruption : la
    // borne evite d'y tourner sans fin.
    for _ in 0..processes().len() + 1 {
        if courant == racine {
            return true;
        }
        if courant == 0 {
            return false;
        }
        let parent = processes()
            .iter()
            .find(|p| p.pid == courant)
            .map(|p| p.parent);
        match parent {
            Some(suivant) => courant = suivant,
            None => return false,
        }
    }
    false
}

/// Vue du registre des taches conservant les usages du `Vec` d'origine.
///
/// `tasks()` rendait `&'static mut Vec<Box<Task>>` : la table entiere, en
/// acces exclusif, a qui la demandait. Rien ne la protegeait -- c'est le gros
/// verrou, pris par tous les appelants, qui rendait l'ensemble sur.
///
/// Cette vue s'appuie sur `registre`, dont les emplacements ont une adresse
/// stable et se lisent sans verrou. Elle garde les memes methodes pour que la
/// migration se fasse sous-systeme par sous-systeme plutot qu'en une fois : un
/// changement de cette taille, fait d'un coup, ne se relit pas.
pub struct VueRegistre;

impl VueRegistre {
    #[inline]
    pub fn len(&self) -> usize { registre_longueur() }

    #[inline]
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &'static Task> { registre_iter() }

    #[inline]
    pub fn iter_mut(&self) -> impl Iterator<Item = &'static mut Task> {
        registre_iter_mut()
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&'static Task> {
        registre_tache(index)
    }

    #[inline]
    pub fn get_mut(&self, index: usize) -> Option<&'static mut Task> {
        registre_tache_mut(index)
    }
}

impl core::ops::Index<usize> for VueRegistre {
    type Output = Task;
    #[inline]
    fn index(&self, index: usize) -> &Task {
        registre_tache(index).expect("registre: indice de tache invalide")
    }
}

impl core::ops::IndexMut<usize> for VueRegistre {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Task {
        registre_tache_mut(index).expect("registre: indice de tache invalide")
    }
}

#[inline]
fn tasks() -> VueRegistre { VueRegistre }

/// Table des processus.
pub fn processes() -> Vec<Arc<Process>> {
    PROCESSES.lock().clone()
}

/// Retrouve un processus par son pid.
pub fn process_by_pid(pid: u32) -> Option<Arc<Process>> {
    PROCESSES.lock().iter().find(|p| p.pid == pid).cloned()
}

/// Retrouve le processus auquel appartient un thread donne.
pub fn process_of_tid(tid: u32) -> Option<Arc<Process>> {
    tasks()
        .iter()
        .find(|t| t.tid == tid)
        .map(|t| t.process.clone())
}

/// Alloue un identifiant de tache.
pub fn alloc_tid() -> u32 {
    NEXT_TID.fetch_add(1, Ordering::Relaxed)
}

/// Y a-t-il une tache utilisateur en cours ?
pub fn in_user_task() -> bool {
    current_index_raw() != NO_TASK
}

// BOUCHAUD_P0_TARGETED_SCHED_IPI_V1
/// The BSP timer reads only atomics here, before any BKL attempt.
/// Idle CPUs and kernel threads are excluded. A user task temporarily inside a
/// syscall remains included so it can still receive its periodic quantum.
pub fn running_user_cpu_mask() -> u64 {
    let online = smp::schedulable_cpus().min(MAX_CPUS).min(64);
    let mut mask = 0u64;
    let mut cpu = 1usize;

    while cpu < online {
        let current = CURRENT[cpu].load(Ordering::Acquire);
        let kernel_task = CURRENT_IS_KERNEL[cpu].load(Ordering::Acquire);
        if current != NO_TASK && !kernel_task {
            mask |= 1u64 << cpu;
        }
        cpu += 1;
    }

    mask
}

/// Tache courante du CPU local.
pub fn current() -> &'static mut Task {
    let index = current_index_raw();
    assert!(index != NO_TASK, "task: aucune tache active sur ce CPU");
    unsafe { &mut *(tasks().get_mut(index).unwrap() as *mut Task) }
}

/// Tache courante, si elle existe.
pub fn try_current() -> Option<&'static mut Task> {
    if in_user_task() {
        Some(current())
    } else {
        None
    }
}

/// Processus de la tache courante.
pub fn current_process() -> Arc<Process> {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Ordonnanceur);
    let _kernel = smp_lock::enter();
    current().process.clone()
}

/// Clone the current process from CPU-local stable ownership. `SpinLockIrq`
/// prevents a same-CPU interrupt from observing a half-published Arc.
///
/// Les interruptions sont coupees autour de `local_cpu()` **et** de la prise du
/// verrou. Sans cela, l'index de CPU est lu, la tache migre, et l'on verrouille
/// la case d'un CPU qui n'est plus le notre — donc l'identite d'une autre
/// tache. Le verrou seul ne suffisait pas : il ne protege que ce qui vient
/// apres lui.
pub fn current_process_local() -> Option<Arc<Process>> {
    interrupts::without_interrupts(|| {
        CURRENT_PROCESS[local_cpu()].lock().as_ref().map(Arc::clone)
    })
}

/// Identite de la tache courante, lue **sans le gros verrou noyau**.
///
/// C'est une COPIE, pas une vue : le `tid` est un entier, le processus est un
/// `Arc` dont le compteur est incremente. Rien ne pointe plus vers la table des
/// taches une fois la fonction rendue.
pub struct IdentiteCourante {
    pub tid: u32,
    pub process: Arc<Process>,
}

/// Lit l'identite de la tache courante depuis le domaine CPU-local.
///
/// # Le domaine
///
/// Deux emplacements, tous deux ecrits par [`install`] pendant un changement de
/// contexte, c'est-a-dire sous le gros verrou :
///
///  * `usermode::per_cpu().current` — le `tid`, dans le bloc par-CPU adresse par
///    `GS`. Il y etait deja : `install` l'ecrit depuis toujours, pour que le
///    stub d'entree des appels systeme sache qui appelle.
///  * `CURRENT_PROCESS[cpu]` — un `Arc<Process>` (`None` pour un fil noyau).
///
/// Aucun des deux ne touche `TASKS`. C'est tout l'objet de cette fonction : la
/// table des taches est un `static mut Vec<Box<Task>>` sans verrou, et c'est
/// elle — pas le processus, pas le tid — qui obligeait `getpid` a prendre le
/// gros verrou.
///
/// # Preuve de duree de vie
///
/// *Publication.* `install` a six appelants — `task_trampoline`, `switch_to`,
/// `secondary_cpu_loop`, `run`, `run_noyau` et `preempt_from_irq` — et les six
/// tiennent le gros verrou au moment de l'appel (verifie un par un).
/// Un CPU ne peut donc jamais observer une publication a moitie faite : elle est
/// faite par lui-meme, ou sous un verrou qu'il devrait prendre pour la voir.
///
/// *Changement.* Les deux champs ne changent qu'a un changement de contexte sur
/// CE CPU. Comme cette fonction coupe les interruptions, aucun changement ne
/// peut s'intercaler entre la lecture du numero de CPU et celle des champs : ni
/// preemption par le PIT, ni IPI de quantum. Un autre CPU, lui, n'ecrit jamais
/// dans notre case.
///
/// *Destruction.* Le `tid` est une valeur : rien a detruire. Le `Process` est
/// tenu par un `Arc` dont on prend une part avant de rendre la main ; il survit
/// donc a la mort de la tache, meme si celle-ci se termine juste apres.
///
/// *Fin de tache, exec, idle.* `switch_to_kernel` et `secondary_cpu_loop`
/// remettent les deux champs a zero (`clear_current_process_local` et
/// `per_cpu().current = 0`) AVANT de quitter la pile de la tache. Un CPU au
/// repos rend donc `None`, il ne rend pas l'identite du dernier occupant.
///
/// # Pourquoi pas un `&'static mut Task`
///
/// Parce qu'il ne serait pas sur. `register` recycle un emplacement zombie par
/// `tasks()[index] = task`, ce qui **detruit** l'ancienne `Box<Task>` : toute
/// reference qui lui aurait survecu pendrait dans le vide. La regle est donc
/// qu'aucune reference vers une `Task` ne sorte de ce module par ce chemin.
pub fn identite_courante() -> Option<IdentiteCourante> {
    let identite = interrupts::without_interrupts(|| {
        let cpu = local_cpu();
        let process = CURRENT_PROCESS[cpu].lock().as_ref().map(Arc::clone)?;
        let tid = usermode::per_cpu().current as u32;
        // `NEXT_TID` part de 100 et ne fait que croitre : aucune tache ne porte
        // le tid 0. C'est donc la valeur que `switch_to_kernel`,
        // `secondary_cpu_loop` et l'initialisation ecrivent pour dire « aucune
        // tache ici ». La rencontrer alors qu'un `Process` est publie voudrait
        // dire qu'une publication a ete coupee en deux, ce que le gros verrou
        // interdit : on refuse de deviner.
        if tid == 0 {
            return None;
        }
        Some(IdentiteCourante { tid, process })
    });
    if identite.is_none() {
        IDENTITE_REPLI.fetch_add(1, Ordering::Relaxed);
    }
    identite
}

fn clear_current_process_local() {
    *CURRENT_PROCESS[local_cpu()].lock() = None;
}

pub fn fault_retry_yield() {
    PF_BKL_ENTERS.fetch_add(1, Ordering::Relaxed);
    FAULT_RETRY_YIELDS.fetch_add(1, Ordering::Relaxed);
    if !schedule() {
        // With no local peer, schedule() deliberately does not HLT a Ready
        // task. A continuously mutating remote mapping would otherwise leave
        // this fault in an unbounded busy loop; wait for the next IRQ instead.
        debug_assert!(!smp_lock::held_by_current_cpu());
        cpu::wait_for_interrupt();
    }
}

pub fn fault_retry_chain_complete(chain: u64) {
    FAULT_RETRY_MAX_CHAIN.fetch_max(chain, Ordering::Relaxed);
}

pub fn fault_retry_stats() -> (u64, u64) {
    (
        FAULT_RETRY_YIELDS.load(Ordering::Relaxed),
        FAULT_RETRY_MAX_CHAIN.load(Ordering::Relaxed),
    )
}

pub fn pf_bkl_enters() -> u64 {
    PF_BKL_ENTERS.load(Ordering::Relaxed)
}

/// Temps processeur consomme par un processus, en millisecondes.
///
/// Le profileur par echantillonnage incremente `ticks_cpu` de la tache courante
/// a chaque IRQ0. Le PIT battant a `TICKS_PER_SECOND` = 1000 Hz, un tick vaut
/// donc exactement une milliseconde de processeur — et la somme sur les taches
/// du processus est son temps CPU.
///
/// Sert a `clock_gettime(CLOCK_PROCESS_CPUTIME_ID)`. Sans cette horloge, un
/// programme ne peut pas distinguer une attente qui dort d'une attente qui
/// brule un cœur : les deux durent le meme temps au mur. C'est exactement la
/// question que posait la reparation de la couche UDP.
pub fn cpu_time_ms(pid: u32) -> u64 {
    let mut total = 0u64;
    for task in tasks().iter() {
        if task.process.pid == pid {
            total = total.saturating_add(task.ticks_cpu);
        }
    }
    total * (1000 / crate::kernel::timer::TICKS_PER_SECOND).max(1)
}

