// --- Diagnostic --------------------------------------------------------------

/// Affiche la table des taches utilisateur (commande `tasks`).
pub fn print_table() {
    let list = tasks();
    if list.is_empty() {
        crate::println!("aucune tache utilisateur (ring 3) active");
        return;
    }
    crate::println!("  TID  PID  ETAT      PAGES  NOM");
    for task in list.iter() {
        let process = &task.process;
        let metadata = process.metadata.lock();
        let pages = process.mm.lock().space.mapped_pages();
        let state = match task.state {
            TaskState::Ready => "ready",
            TaskState::Blocked => "blocked",
            TaskState::Zombie => "zombie",
        };
        crate::println!(
            "  {:>3}  {:>3}  {:<8}  {:>5}  {}",
            task.tid,
            process.pid,
            state,
            pages,
            metadata.name
        );
    }
}

/// Cree un processus vide (espace d'adressage neuf, descripteurs standards).
pub fn new_process(name: &str, cwd: usize) -> Option<Arc<Process>> {
    let space = AddressSpace::new()?;
    let pid = crate::kernel::process::spawn(name, crate::users::session().uid());
    let process = Arc::new(Process {
        pid,
        parent: 0,
        resource_group_id: pid,
        resource_group_name: name.to_string(),
        mm: Arc::new(Mm::new(MmState { space, brk_start: 0, brk: 0,
            mmap_next: crate::kernel::vmm::user_mmap_base(), partages: Vec::new(),
            limite_as: 0, promesses: Vec::new(), clean_pages: Vec::new() })),
        files: Arc::new(FileTable::new(FdTable::new())),
        metadata: SpinLock::new(ProcessMetadata { name: name.to_string(), cwd,
            uid: crate::users::session().uid() as u32,
            gid: crate::users::session().uid() as u32, ecran: None }),
        lifecycle: SpinLock::new(ProcessLifecycle { exit_code: 0, zombie: false, threads: 1 }),
        signals: SpinLock::new(crate::kernel::signal::SignalState::default()),
    });
    PROCESSES.lock().push(process.clone());
    Some(process)
}

/// Enregistre un processus cree par `fork` (espace deja duplique).
pub fn register_process(process: Arc<Process>) {
    PROCESSES.lock().push(process);
}
