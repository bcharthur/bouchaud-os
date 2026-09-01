impl Task {
    /// Cree une tache prete a demarrer en ring 3 avec la trame donnee.
    pub fn new(process: Arc<Process>, frame: TrapFrame) -> Box<Task> {
        let kstack = vec![0u8; KSTACK_SIZE];
        let kstack_top = (kstack.as_ptr() as u64 + KSTACK_SIZE as u64) & !0xF;
        let fpu = vec![0u8; 512 + 16];
        let fpu_area = (fpu.as_ptr() as u64 + 15) & !0xF;
        unsafe {
            let area = fpu_area as *mut u8;
            core::ptr::copy_nonoverlapping(0x037Fu16.to_le_bytes().as_ptr(), area, 2);
            core::ptr::copy_nonoverlapping(0x1F80u32.to_le_bytes().as_ptr(), area.add(24), 4);
            core::ptr::copy_nonoverlapping(0x0000_FFBFu32.to_le_bytes().as_ptr(), area.add(28), 4);
        }

        let mut task = Box::new(Task {
            tid: alloc_tid(),
            process,
            state: EtatAtomique::neuf(TaskState::Ready),
            priorite: Priorite::Normale,
            affinity_mask: 0,
            runq_cpu: CoeurAtomique::neuf(u8::MAX),
            last_cpu: u8::MAX,
            on_cpu: CoeurSigneAtomique::neuf(-1),
            switching_out: DrapeauAtomique::neuf(false),
            last_migration_ns: 0,
            recent_runtime_ns: 0,
            slice_start_ns: 0,
            ready_since_ns: EcheanceAtomique::neuf(0),
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
            futex_key: EcheanceAtomique::neuf(0),
            wait_queue_key: CleAtomique::neuf(0),
            wake_deadline_ns: EcheanceAtomique::neuf(0),
            waiting_for_child: DrapeauAtomique::neuf(false),
            fresh: true,
            ticks_cpu: 0,
            noyau: false,
            entree_noyau: None,
        });
        amorce_pile(&mut task, task_trampoline, 0x0000_0002);
        task
    }

    pub fn new_kernel(process: Arc<Process>, entree: fn() -> !) -> Box<Task> {
        let mut task = Task::new(process, TrapFrame::new_user(0, 0));
        task.noyau = true;
        task.affinity_mask = 1;
        task.runq_cpu.range(0);
        task.last_cpu = 0;
        task.entree_noyau = Some(entree);
        amorce_pile(&mut task, kernel_task_trampoline, 0x0000_0202);
        task
    }

    fn fpu_ptr(&self) -> u64 { self.fpu_area }
}

fn amorce_pile(task: &mut Task, trampoline: extern "C" fn() -> !, rflags: u64) {
    unsafe {
        let mut sp = task.kstack_top as *mut u64;
        sp = sp.sub(1); *sp = trampoline as *const () as usize as u64;
        sp = sp.sub(1); *sp = rflags;
        for _ in 0..6 { sp = sp.sub(1); *sp = 0; }
        task.ctx.rsp = sp as u64;
    }
}

fn online_affinity_mask() -> u64 {
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS).min(64);
    if online >= 64 { u64::MAX } else { (1u64 << online) - 1 }
}

#[inline]
fn allowed_on(task: &Task, cpu: usize) -> bool {
    cpu < 64 && task.affinity_mask & (1u64 << cpu) != 0
}

fn running_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state != TaskState::Zombie && t.on_cpu == cpu as i8 && !t.switching_out.charge()
    }).count()
}

fn queue_pressure(cpu_id: usize) -> usize {
    crate::arch::x86_64::cpu_local::CpuId::from_index(cpu_id)
        .map(|id| crate::arch::x86_64::cpu_local::local(id).run_queue_len())
        .unwrap_or(0)
}

fn choose_runq_cpu(mask: u64) -> u8 {
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS);
    let mut best_cpu = 0usize;
    let mut best_score = usize::MAX;
    for cpu_id in 0..online {
        if cpu_id >= 64 || mask & (1u64 << cpu_id) == 0 { continue; }
        let rq = queue_pressure(cpu_id);
        let running = running_count_cpu(cpu_id);
        let measured = cpu::load_percent_cpu(cpu_id) as usize;
        let bsp_penalty = if cpu_id == 0 && online > 1 { 24 } else { 0 };
        let score = rq.saturating_mul(32)
            .saturating_add(running.saturating_mul(16))
            .saturating_add(measured)
            .saturating_add(bsp_penalty);
        if score < best_score { best_score = score; best_cpu = cpu_id; }
    }
    best_cpu as u8
}

/// Publish a Ready task exactly once to its owning physical runqueue.
/// P0-NG1 additionally timestamps the ready edge and requests a safe-point on
/// the target CPU. The target IPI is still sent only when that CPU is idle; a
/// running CPU consumes `need_resched` at its next safe kernel boundary.
fn publish_ready(index: usize) {
    if index >= tasks().len() || tasks()[index].state != TaskState::Ready
        || tasks()[index].on_cpu >= 0 || tasks()[index].switching_out.charge()
    { return; }

    if tasks()[index].ready_since_ns == 0 {
        tasks()[index].ready_since_ns.range(crate::kernel::timer::monotonic_ns());
    }
    let target = if allowed_on(&tasks()[index], tasks()[index].runq_cpu.charge() as usize) {
        tasks()[index].runq_cpu.charge() as usize
    } else {
        choose_runq_cpu(tasks()[index].affinity_mask) as usize
    };
    tasks()[index].runq_cpu.range(target as u8);
    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(target) {
        crate::arch::x86_64::cpu_local::local(id).enqueue(index);
        crate::kernel::scheduler::preempt::request_cpu(target);
    }
    if cpu::is_idle(target) { smp::reschedule_cpu(target); }
}

pub fn register(mut task: Box<Task>) -> usize {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Processus);
    let _kernel = smp_lock::enter();
    if task.noyau {
        task.affinity_mask = 1;
        task.runq_cpu.range(0);
        task.last_cpu = 0;
    } else {
        if task.affinity_mask == 0 {
            task.affinity_mask = online_affinity_mask();
        } else {
            task.affinity_mask &= online_affinity_mask();
            if task.affinity_mask == 0 { task.affinity_mask = online_affinity_mask(); }
        }
        if task.runq_cpu == u8::MAX || !allowed_on(&task, task.runq_cpu.charge() as usize) {
            task.runq_cpu.range(choose_runq_cpu(task.affinity_mask));
        }
    }
    task.on_cpu.range(-1);
    task.switching_out.range(false);

    // Le registre choisit l'emplacement lui-meme, sous son propre verrou :
    // c'est la SEULE section critique qui reste sur ce chemin. Le predicat dit
    // ce qu'est un emplacement recyclable -- une tache morte, sur aucun coeur,
    // et qui n'est pas en train de commuter. Une tache qui commute encore
    // possede sa pile noyau ; la reecrire la ferait reprendre sur une autre.
    let index = registre_ajoute(task, |ancienne| {
        ancienne.state == TaskState::Zombie
            && ancienne.on_cpu < 0
            && !ancienne.switching_out.charge()
    })
    .expect("registre des taches plein");

    {
        let registered = &tasks()[index];
        let process = &registered.process;
        let metadata = process.metadata.lock();
        crate::serial_println!(
            "[SMP-TASK] idx={} tid={} pid={} rq={} last={} aff={:#x} on={} kernel={} prio={:?} name={}",
            index, registered.tid, process.pid, registered.runq_cpu,
            registered.last_cpu, registered.affinity_mask, registered.on_cpu,
            registered.noyau, registered.priorite, metadata.name.as_str(),
        );
    }
    publish_ready(index);
    index
}

fn index_of(tid: u32) -> Option<usize> { tasks().iter().position(|t| t.tid == tid) }
pub fn by_tid(tid: u32) -> Option<&'static mut Task> {
    let index = index_of(tid)?;
    Some(unsafe { &mut *(tasks().get_mut(index).unwrap() as *mut Task) })
}
pub fn live_count() -> usize {
    tasks().iter().filter(|t| t.state != TaskState::Zombie).count()
}
fn ready_count() -> usize { tasks().iter().filter(|t| t.state == TaskState::Ready).count() }
fn ready_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state == TaskState::Ready && t.on_cpu < 0 && !t.switching_out.charge()
            && t.runq_cpu.charge() as usize == cpu && allowed_on(t, cpu)
    }).count()
}
fn stealable_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state == TaskState::Ready && t.on_cpu < 0 && !t.switching_out.charge() && !t.noyau
            && t.runq_cpu.charge() as usize != cpu && allowed_on(t, cpu)
    }).count()
}
fn running_count() -> usize {
    tasks().iter().filter(|t| t.state != TaskState::Zombie && t.on_cpu >= 0).count()
}
