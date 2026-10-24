pub use crate::kernel::continuation::ContexteExecution;

pub fn contexte_execution() -> ContexteExecution {
    interrupts::without_interrupts(|| {
        let cpu = local_cpu();
        let index = CURRENT[cpu].load(Ordering::Acquire);
        crate::kernel::continuation::contexte(
            cpu, (index != NO_TASK).then_some(index),
            CONTINUATION_ETAT.load(Ordering::Acquire) != CONTINUATION_LIBRE,
        )
    })
}

#[track_caller]
fn exige_tache_courante() -> usize {
    match contexte_execution() {
        ContexteExecution::Tache(index) => index,
        autre => panic!("task: primitive de tache depuis {:?} appelant={}",
            autre, core::panic::Location::caller()),
    }
}

// One synchronous BSP caller; its stack remains parked until the scoped
// root has exited. Kernel tasks use their own context and never touch this
// slot. No allocation, fabricated Task identity, or erased NO_TASK assertion.
struct AppelBsp {
    donnees: *mut (),
    execute: unsafe fn(*mut ()),
}
static mut APPEL_BSP: Option<AppelBsp> = None;

fn entree_attente_bsp() -> ! {
    let appel = unsafe { (&*core::ptr::addr_of!(APPEL_BSP)).as_ref().expect("appel BSP absent") };
    unsafe { (appel.execute)(appel.donnees); }
    exit_current(0)
}

/// Run a blocking kernel operation in an actual scheduled task. The BSP
/// continuation can sleep and let its own CPU run workers even under SMP1.
/// Idle and interrupt callers must not borrow a task's parking state.
pub(crate) fn avec_tache<F: FnOnce() -> R, R>(operation: F) -> R {
    match contexte_execution() {
        ContexteExecution::Tache(_) => return operation(),
        ContexteExecution::Idle => panic!("task: attente bloquante depuis idle"),
        ContexteExecution::ContinuationBsp => {}
    }
    assert!(interrupts::are_enabled(), "attente BSP avec IF=0");
    let local = crate::arch::x86_64::cpu_local::local(
        crate::arch::x86_64::cpu_local::CpuId::from_index(local_cpu()).unwrap());
    assert_eq!(local.irq_depth(), 0, "attente BSP depuis un handler");
    assert_eq!(local.verrous_simples(), 0, "attente BSP sous verrou tournant");
    assert_eq!(local.preempt_count(), 0, "attente BSP non preemptible");

    struct Appel<F, R> { operation: Option<F>, resultat: Option<R> }
    unsafe fn execute<F: FnOnce() -> R, R>(donnees: *mut ()) {
        let appel = &mut *donnees.cast::<Appel<F, R>>();
        appel.resultat = Some(appel.operation.take().expect("appel BSP repete")());
    }
    let mut appel = Appel { operation: Some(operation), resultat: None };
    let process = new_process("bsp-attente", 0).expect("creation attente BSP");
    unsafe {
        assert!((&*core::ptr::addr_of!(APPEL_BSP)).is_none(), "attente BSP imbriquee");
        APPEL_BSP = Some(AppelBsp {
            donnees: (&mut appel as *mut Appel<F, R>).cast(), execute: execute::<F, R>,
        });
    }
    // run() retires only this root and its descendants, preserving services.
    let code = run(Task::new_kernel(process, entree_attente_bsp));
    unsafe { APPEL_BSP = None; }
    assert_eq!(code, 0, "echec de la tache d'attente BSP");
    assert_eq!(contexte_execution(), ContexteExecution::ContinuationBsp);
    appel.resultat.expect("continuation BSP reprise sans resultat")
}

pub fn banc_contexte_bsp() -> i32 {
    assert_eq!(contexte_execution(), ContexteExecution::ContinuationBsp);
    let local = crate::arch::x86_64::cpu_local::local(
        crate::arch::x86_64::cpu_local::CpuId::from_index(local_cpu()).unwrap());
    let entrees = local.irq_context.entries();
    let debut = crate::kernel::timer::monotonic_ns();
    sleep_ticks(2);
    assert!(crate::kernel::timer::monotonic_ns() >= debut + 2_000_000);
    let file = crate::kernel::sync::WaitQueue::new();
    let limite = crate::kernel::timer::monotonic_ns() + 2_000_000;
    assert!(!file.wait_until_noyau(file.ticket(), limite));
    assert_eq!(contexte_execution(), ContexteExecution::ContinuationBsp);
    fn service() -> ! { loop { sleep_ticks(2); } }
    fn racine() -> ! { exit_current(0) }
    assert!(spawn_noyau(service, "banc-service-survivant"));
    let survivant = processes().into_iter()
        .find(|p| p.metadata.lock().name == "banc-service-survivant")
        .expect("service de banc absent").pid;
    assert_eq!(run_noyau(racine, "banc-racine-ephemere"), 0);
    assert!(processes().iter().any(|p| p.pid == survivant),
        "run_noyau a detruit un service hors session");
    crate::kernel::process::kill(survivant);
    assert_eq!(local.irq_depth(), 0, "profondeur IRQ abandonnee sur la continuation BSP");
    assert!(local.irq_context.entries() > entrees, "aucune entree IRQ suivie");
    assert!(local.irq_context.maximum() >= 1);
    crate::serial_println!("CONTEXTE_IRQ_OK profondeur=0 entrees={} maximum={}",
        local.irq_context.entries(), local.irq_context.maximum());
    crate::serial_println!("CONTEXTE_LIFECYCLE_OK service_hors_session_preserve=1");
    crate::serial_println!("CONTEXTE_BSP_OK sommeil=1 waitqueue=1 retour_sans_tache=1");
    0
}
