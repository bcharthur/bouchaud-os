impl Task {
    /// Cree une tache prete a demarrer en ring 3 avec la trame donnee.
    pub fn new(process: Arc<Process>, frame: TrapFrame) -> Box<Task> {
        let kstack = vec![0u8; KSTACK_SIZE];
        // Sommet aligne 16 : l'ABI System V l'exige avant chaque `call`, et le
        // stub d'entree syscall empile un nombre pair de quadmots.
        let kstack_top = (kstack.as_ptr() as u64 + KSTACK_SIZE as u64) & !0xF;
        // Le tampon vit sur le tas : son adresse ne bouge pas quand la `Task`
        // est deplacee dans son `Box`.
        let fpu = vec![0u8; 512 + 16];
        let fpu_area = (fpu.as_ptr() as u64 + 15) & !0xF;
        // Etat FPU initial valide : une zone `fxsave` toute a zero donnerait
        // MXCSR = 0, c'est-a-dire toutes les exceptions SSE demasquees — la
        // premiere division flottante du programme leverait alors #XF.
        unsafe {
            let area = fpu_area as *mut u8;
            core::ptr::copy_nonoverlapping(0x037Fu16.to_le_bytes().as_ptr(), area, 2); // FCW
            core::ptr::copy_nonoverlapping(0x1F80u32.to_le_bytes().as_ptr(), area.add(24), 4); // MXCSR
            core::ptr::copy_nonoverlapping(0x0000_FFBFu32.to_le_bytes().as_ptr(), area.add(28), 4);
            // MXCSR_MASK
        }

        let mut task = Box::new(Task {
            tid: alloc_tid(),
            process,
            state: TaskState::Ready,
            // Toute tache nait normale. C'est a elle de se declarer
            // interactive — un programme qui ne demande rien ne doit pas
            // pouvoir prendre le pas sur l'interface par accident.
            priorite: Priorite::Normale,
            affinity_mask: 0,
            runq_cpu: u8::MAX,
            last_cpu: u8::MAX,
            on_cpu: -1,
            switching_out: false,
            last_migration_ns: 0,
            recent_runtime_ns: 0,
            slice_start_ns: 0,
            last_account_ns: 0,
            user_cpu_ns: 0,
            kernel_cpu_ns: 0,
            cpu_ns: [0; MAX_CPUS],
            in_kernel: false,
            context_switches: 0,
            migrations: 0,
            frame,
            ctx: Context::default(),
            kstack,
            kstack_top,
            fpu,
            fpu_area,
            fs_base: 0,
            clear_child_tid: 0,
            futex_key: 0,
            wait_queue_key: 0,
            wake_deadline_ns: 0,
            waiting_for_child: false,
            fresh: true,
            ticks_cpu: 0,
            noyau: false,
            entree_noyau: None,
        });

        // RFLAGS de depart : bit 1 reserve a 1, `IF` a 0. Le trampoline n'a
        // pas besoin des interruptions — `resume_usermode` commence par un
        // `cli` — et l'`iretq` qui l'acheve rendra a la tache le RFLAGS de
        // sa trame ring 3.
        amorce_pile(&mut task, task_trampoline, 0x0000_0002);
        task
    }

    /// Cree un fil noyau : meme ordonnancement, mais il execute `entree` en
    /// ring 0 au lieu de partir en ring 3.
    ///
    /// `process` n'est la que pour les champs que tout le noyau consulte sans
    /// se demander qui les porte (pid, table de descripteurs). Son espace
    /// d'adressage n'est jamais active : [`install`] bascule sur celui du noyau
    /// pour un fil noyau.
    pub fn new_kernel(process: Arc<Process>, entree: fn() -> !) -> Box<Task> {
        // La trame ring 3 n'a aucun sens ici ; elle reste a zero et n'est jamais
        // restauree, puisque le trampoline noyau ne fait pas d'`iretq`.
        let mut task = Task::new(process, TrapFrame::new_user(0, 0));
        task.noyau = true;
        task.affinity_mask = 1;
        task.runq_cpu = 0;
        task.last_cpu = 0;
        task.entree_noyau = Some(entree);
        // Un fil noyau demarre **interruptions actives** : rien ne les
        // retablira pour lui plus tard. Sans `IF`, sa premiere attente
        // s'arreterait sur un `hlt` que plus aucun tick ne pourrait lever.
        amorce_pile(&mut task, kernel_task_trampoline, 0x0000_0202);
        task
    }

    /// Adresse de la zone `fxsave` (alignee 16, dans `self.fpu`).
    fn fpu_ptr(&self) -> u64 {
        self.fpu_area
    }
}

/// Amorce de pile noyau : le premier `switch_context` vers cette tache depile
/// six registres callee-saved et un RFLAGS, puis fait `ret` sur `trampoline`.
/// La disposition doit etre le miroir exact des `push` de `switch_context`.
fn amorce_pile(task: &mut Task, trampoline: extern "C" fn() -> !, rflags: u64) {
    unsafe {
        let mut sp = task.kstack_top as *mut u64;
        sp = sp.sub(1);
        *sp = trampoline as *const () as usize as u64; // adresse de retour
        sp = sp.sub(1);
        *sp = rflags;
        for _ in 0..6 {
            sp = sp.sub(1);
            *sp = 0; // rbp, rbx, r12, r13, r14, r15
        }
        task.ctx.rsp = sp as u64;
    }
}

/// Masque des CPU logiques actuellement utilisables.
fn online_affinity_mask() -> u64 {
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS).min(64);
    if online >= 64 { u64::MAX } else { (1u64 << online) - 1 }
}

#[inline]
fn allowed_on(task: &Task, cpu: usize) -> bool {
    cpu < 64 && task.affinity_mask & (1u64 << cpu) != 0
}

fn running_count_cpu(cpu: usize) -> usize {
    tasks().iter()
        .filter(|t| {
            t.state != TaskState::Zombie
                && t.on_cpu == cpu as i8
                && !t.switching_out
        })
        .count()
}

fn queue_pressure(cpu_id: usize) -> usize {
    crate::arch::x86_64::cpu_local::CpuId::from_index(cpu_id)
        .map(|id| crate::arch::x86_64::cpu_local::local(id).run_queue_len())
        .unwrap_or(0)
}

/// Placement initial d'un THREAD. Le score combine pression de runqueue,
/// nombre de taches deja running et charge mesuree. CPU0 recoit une petite
/// penalite car il porte le desktop/PIC, sans devenir interdit au userland.
fn choose_runq_cpu(mask: u64) -> u8 {
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS);
    let mut best_cpu = 0usize;
    let mut best_score = usize::MAX;

    for cpu_id in 0..online {
        if cpu_id >= 64 || mask & (1u64 << cpu_id) == 0 {
            continue;
        }
        let rq = queue_pressure(cpu_id);
        let running = running_count_cpu(cpu_id);
        let measured = cpu::load_percent_cpu(cpu_id) as usize;
        let bsp_penalty = if cpu_id == 0 && online > 1 { 24 } else { 0 };
        let score = rq.saturating_mul(32)
            .saturating_add(running.saturating_mul(16))
            .saturating_add(measured)
            .saturating_add(bsp_penalty);
        if score < best_score {
            best_score = score;
            best_cpu = cpu_id;
        }
    }
    best_cpu as u8
}

/// Publish a Ready task exactly once to its owning physical runqueue and wake
/// only that CPU when it is halted.
fn publish_ready(index: usize) {
    if index >= tasks().len() || tasks()[index].state != TaskState::Ready
        || tasks()[index].on_cpu >= 0
        || tasks()[index].switching_out
    {
        return;
    }
    let target = if allowed_on(&tasks()[index], tasks()[index].runq_cpu as usize) {
        tasks()[index].runq_cpu as usize
    } else {
        choose_runq_cpu(tasks()[index].affinity_mask) as usize
    };
    tasks()[index].runq_cpu = target as u8;
    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(target) {
        crate::arch::x86_64::cpu_local::local(id).enqueue(index);
    }
    if cpu::is_idle(target) { smp::reschedule_cpu(target); }
}

/// Ajoute une tache a la table et renvoie son indice. En SMP les indices ne
/// bougent jamais : un slot zombie est recycle au lieu de compacter le Vec.
pub fn register(mut task: Box<Task>) -> usize {
    let _kernel = smp_lock::enter();

    if task.noyau {
        task.affinity_mask = 1;
        task.runq_cpu = 0;
        task.last_cpu = 0;
    } else {
        if task.affinity_mask == 0 {
            task.affinity_mask = online_affinity_mask();
        } else {
            task.affinity_mask &= online_affinity_mask();
            if task.affinity_mask == 0 {
                task.affinity_mask = online_affinity_mask();
            }
        }
        if task.runq_cpu == u8::MAX || !allowed_on(&task, task.runq_cpu as usize) {
            task.runq_cpu = choose_runq_cpu(task.affinity_mask);
        }
    }
    task.on_cpu = -1;
    task.switching_out = false;

    let reuse = tasks().iter().position(|old| {
        old.state == TaskState::Zombie && old.on_cpu < 0 && !old.switching_out
    });
    let index = if let Some(index) = reuse {
        tasks()[index] = task;
        index
    } else {
        let list = tasks();
        list.push(task);
        list.len() - 1
    };

    {
        let registered = &tasks()[index];
        let process = &registered.process;
        let metadata = process.metadata.lock();
        crate::serial_println!(
            "[SMP-TASK] idx={} tid={} pid={} rq={} last={} aff={:#x} on={} kernel={} prio={:?} name={}",
            index,
            registered.tid,
            process.pid,
            registered.runq_cpu,
            registered.last_cpu,
            registered.affinity_mask,
            registered.on_cpu,
            registered.noyau,
            registered.priorite,
            metadata.name.as_str(),
        );
    }

    publish_ready(index);
    index
}

/// Indice d'une tache par son tid.
fn index_of(tid: u32) -> Option<usize> {
    tasks().iter().position(|t| t.tid == tid)
}

/// Tache par tid.
pub fn by_tid(tid: u32) -> Option<&'static mut Task> {
    let index = index_of(tid)?;
    Some(unsafe { &mut *(&mut **tasks().get_mut(index).unwrap() as *mut Task) })
}

/// Nombre de taches vivantes (non zombies).
pub fn live_count() -> usize {
    tasks()
        .iter()
        .filter(|t| t.state != TaskState::Zombie)
        .count()
}

/// Nombre de taches pretes.
fn ready_count() -> usize {
    tasks().iter().filter(|t| t.state == TaskState::Ready).count()
}

fn ready_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state == TaskState::Ready
            && t.on_cpu < 0
            && !t.switching_out
            && t.runq_cpu as usize == cpu
            && allowed_on(t, cpu)
    }).count()
}

fn stealable_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state == TaskState::Ready
            && t.on_cpu < 0
            && !t.switching_out
            && !t.noyau
            && t.runq_cpu as usize != cpu
            && allowed_on(t, cpu)
    }).count()
}

fn running_count() -> usize {
    tasks().iter().filter(|t| t.state != TaskState::Zombie && t.on_cpu >= 0).count()
}

