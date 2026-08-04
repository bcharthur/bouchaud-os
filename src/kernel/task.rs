//! Taches utilisateur : threads, changement de contexte, futex.
//!
//! Un **processus** ([`Process`]) possede un espace d'adressage, une table de
//! descripteurs, un `brk` et une zone `mmap`. Une **tache** ([`Task`]) est un
//! fil d'execution : c'est l'unite ordonnancee. `clone(CLONE_THREAD)` cree une
//! tache de plus dans le meme processus, exactement comme sous Linux — c'est ce
//! dont `pthread_create` (donc Qt, donc Python) a besoin.
//!
//! ## Deux piles par tache
//!
//! - la **pile utilisateur**, dans l'espace d'adressage du processus ;
//! - la **pile noyau**, privee, sur laquelle s'executent ses appels systeme.
//!   C'est elle qui rend le blocage possible : quand une tache s'endort dans un
//!   `futex`, son etat noyau reste sur sa propre pile pendant qu'une autre tache
//!   utilise la sienne.
//!
//! ## Ou l'ordonnanceur peut-il commuter ?
//!
//! - a un point de blocage volontaire (`futex`, `nanosleep`, `sched_yield`,
//!   lecture bloquante) ;
//! - sur IRQ0 **uniquement si le timer a interrompu du code ring 3**
//!   ([`preempt_from_irq`]).
//!
//! Le noyau lui-meme n'est jamais preempte : il n'est pas reentrant (son
//! allocateur et ses pilotes prennent des verrous tournants), et le preempter
//! provoquerait des interblocages sur un CPU unique. Une tache utilisateur, en
//! revanche, ne detient aucun verrou noyau : la preempter est sans risque.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::usermode::{self, TrapFrame};
use crate::kernel::fd::FdTable;
use crate::kernel::vmm::AddressSpace;

/// Taille de la pile noyau d'une tache (64 KiB).
const KSTACK_SIZE: usize = 64 * 1024;

/// Etat d'ordonnancement d'une tache.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TaskState {
    /// Prete a s'executer (ou en cours).
    Ready,
    /// En attente d'un evenement (futex, sommeil, entree).
    Blocked,
    /// Terminee, en attente de nettoyage.
    Zombie,
}

/// Contexte noyau sauvegarde lors d'un changement de tache.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {
    /// Sommet de pile noyau sauvegarde (tout le reste y est empile).
    pub rsp: u64,
}

/// Ressources partagees par tous les threads d'un meme programme.
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub space: AddressSpace,
    pub files: FdTable,
    /// Debut et sommet courant du tas `brk`.
    pub brk_start: u64,
    pub brk: u64,
    /// Prochaine adresse libre pour `mmap`.
    pub mmap_next: u64,
    /// Repertoire courant (index de nœud RAMFS).
    pub cwd: usize,
    /// Code de sortie renseigne par `exit_group`.
    pub exit_code: i32,
    /// Nombre de threads encore vivants.
    pub threads: usize,
    /// uid/gid vus par le programme.
    pub uid: u32,
    pub gid: u32,
}

/// Un fil d'execution utilisateur.
pub struct Task {
    pub tid: u32,
    pub process: Rc<RefCell<Process>>,
    pub state: TaskState,
    /// Etat ring 3 quand la tache n'est pas en cours d'execution.
    pub frame: TrapFrame,
    /// Contexte noyau (pile) pour le changement de tache.
    pub ctx: Context,
    /// Pile noyau privee.
    kstack: Vec<u8>,
    pub kstack_top: u64,
    /// Zone `fxsave` (512 octets alignes 16) pour l'etat FPU/SSE.
    fpu: Vec<u8>,
    fpu_area: u64,
    /// Base FS (TLS de la libc) propre au thread.
    pub fs_base: u64,
    /// Adresse ecrite a la mort du thread (`set_tid_address`), pour pthread_join.
    pub clear_child_tid: u64,
    /// Cle du futex attendu, si la tache est bloquee dessus.
    pub futex_key: u64,
    /// Tick a partir duquel un sommeil se termine (0 = pas de sommeil).
    pub wake_tick: u64,
    /// La tache n'a pas encore rejoint le ring 3.
    pub fresh: bool,
}

static mut TASKS: Option<Vec<Box<Task>>> = None;
static mut CURRENT: usize = usize::MAX;
static mut NEXT_TID: u32 = 100;
/// Contexte du fil noyau (shell/bureau) qui a lance le programme.
static mut KERNEL_CTX: Context = Context { rsp: 0 };
/// Une tache utilisateur a-t-elle demande une preemption ?
static mut NEED_RESCHED: bool = false;

fn tasks() -> &'static mut Vec<Box<Task>> {
    unsafe {
        if TASKS.is_none() {
            TASKS = Some(Vec::new());
        }
        TASKS.as_mut().unwrap()
    }
}

/// Alloue un identifiant de tache.
pub fn alloc_tid() -> u32 {
    unsafe {
        let tid = NEXT_TID;
        NEXT_TID += 1;
        tid
    }
}

/// Y a-t-il une tache utilisateur en cours ?
pub fn in_user_task() -> bool {
    unsafe { CURRENT != usize::MAX }
}

/// Tache courante.
///
/// # Panique
/// Si aucune tache utilisateur n'est active (appel depuis le shell).
pub fn current() -> &'static mut Task {
    let index = unsafe { CURRENT };
    assert!(index != usize::MAX, "task: aucune tache utilisateur active");
    unsafe { &mut *(&mut **tasks().get_mut(index).unwrap() as *mut Task) }
}

/// Tache courante, si elle existe.
pub fn try_current() -> Option<&'static mut Task> {
    if in_user_task() { Some(current()) } else { None }
}

/// Processus de la tache courante.
pub fn current_process() -> Rc<RefCell<Process>> {
    current().process.clone()
}

impl Task {
    /// Cree une tache prete a demarrer en ring 3 avec la trame donnee.
    pub fn new(process: Rc<RefCell<Process>>, frame: TrapFrame) -> Box<Task> {
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
            core::ptr::copy_nonoverlapping(0x0000_FFBFu32.to_le_bytes().as_ptr(), area.add(28), 4); // MXCSR_MASK
        }

        let mut task = Box::new(Task {
            tid: alloc_tid(),
            process,
            state: TaskState::Ready,
            frame,
            ctx: Context::default(),
            kstack,
            kstack_top,
            fpu,
            fpu_area,
            fs_base: 0,
            clear_child_tid: 0,
            futex_key: 0,
            wake_tick: 0,
            fresh: true,
        });

        // Amorce de pile noyau : le premier `switch_context` vers cette tache
        // depile six registres callee-saved puis fait `ret` sur le trampoline.
        unsafe {
            let mut sp = task.kstack_top as *mut u64;
            sp = sp.sub(1);
            *sp = task_trampoline as *const () as usize as u64; // adresse de retour
            for _ in 0..6 {
                sp = sp.sub(1);
                *sp = 0; // rbp, rbx, r12, r13, r14, r15
            }
            task.ctx.rsp = sp as u64;
        }
        task
    }

    /// Adresse de la zone `fxsave` (alignee 16, dans `self.fpu`).
    fn fpu_ptr(&self) -> u64 {
        self.fpu_area
    }
}

/// Ajoute une tache a la table et renvoie son indice.
pub fn register(task: Box<Task>) -> usize {
    let list = tasks();
    list.push(task);
    list.len() - 1
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
    tasks().iter().filter(|t| t.state != TaskState::Zombie).count()
}

/// Nombre de taches pretes.
fn ready_count() -> usize {
    tasks().iter().filter(|t| t.state == TaskState::Ready).count()
}

// --- Changement de contexte --------------------------------------------------

/// Sauvegarde les registres callee-saved sur la pile courante, bascule sur la
/// pile `to`, et y restaure les registres. Le retour se fait sur l'adresse
/// empilee par la sauvegarde symetrique (ou par l'amorce de `Task::new`).
///
/// # Securite
/// `from` doit pointer sur un `Context` valide et `to` sur une pile noyau
/// preparee par cette meme fonction ou par `Task::new`.
#[unsafe(naked)]
unsafe extern "C" fn switch_context(from: *mut u64, to: u64) {
    core::arch::naked_asm!(
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push rbx",
        "push rbp",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop rbp",
        "pop rbx",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",
        "ret",
    )
}

/// Point d'entree d'une tache neuve : installe son contexte puis part en ring 3.
extern "C" fn task_trampoline() -> ! {
    let task = current();
    install(task);
    let frame = task.frame;
    unsafe { usermode::resume_usermode(&frame) }
}

/// Installe le contexte materiel d'une tache : espace d'adressage, pile noyau,
/// base FS (TLS) et etat FPU.
fn install(task: &mut Task) {
    unsafe {
        task.process.borrow().space.activate();
        usermode::set_kernel_stack(task.kstack_top);
        usermode::set_fs_base(task.fs_base);
        usermode::per_cpu().current = task.tid as u64;
        // La zone est initialisee a un etat FPU valide des `Task::new` : on peut
        // restaurer inconditionnellement, y compris au premier passage.
        usermode::fxrstor(task.fpu_ptr() as *const u8);
        task.fresh = false;
    }
}

/// Choisit la prochaine tache prete apres `after` (tourniquet).
fn pick_next(after: usize) -> Option<usize> {
    let list = tasks();
    let len = list.len();
    if len == 0 {
        return None;
    }
    for offset in 1..=len {
        let index = (after.wrapping_add(offset)) % len;
        if list[index].state == TaskState::Ready {
            return Some(index);
        }
    }
    None
}

/// Rend la main : bascule sur une autre tache prete s'il y en a une.
///
/// Renvoie `true` si un changement de tache a eu lieu. Si la tache courante est
/// la seule prete, la fonction attend une interruption (`hlt`) et rend la main a
/// l'appelant, qui doit reevaluer sa condition d'attente.
pub fn schedule() -> bool {
    let cur = unsafe { CURRENT };
    if cur == usize::MAX {
        return false;
    }
    wake_sleepers();
    let next = match pick_next(cur) {
        Some(n) if n != cur => n,
        _ => {
            // Personne d'autre : si la tache courante est bloquee, on attend une
            // interruption plutot que de bruler du CPU.
            if tasks()[cur].state != TaskState::Ready {
                cpu::hlt();
            }
            return false;
        }
    };
    switch_to(cur, next);
    true
}

/// Bascule de la tache `from` vers la tache `to`.
fn switch_to(from: usize, to: usize) {
    unsafe {
        let list = tasks();
        let from_ptr = &mut **list.get_mut(from).unwrap() as *mut Task;
        let to_ptr = &mut **list.get_mut(to).unwrap() as *mut Task;

        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();

        CURRENT = to;
        install(&mut *to_ptr);

        switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp);
    }
}

/// Retour definitif au fil noyau appelant (la tache courante est terminee).
fn switch_to_kernel() -> ! {
    unsafe {
        let cur = CURRENT;
        let list = tasks();
        let from_ptr = &mut **list.get_mut(cur).unwrap() as *mut Task;
        CURRENT = usize::MAX;
        usermode::per_cpu().current = 0;
        crate::kernel::vmm::activate_kernel();
        switch_context(&mut (*from_ptr).ctx.rsp, KERNEL_CTX.rsp);
    }
    // `switch_context` ne revient jamais ici : la tache est zombie.
    unreachable!("task: reprise d'une tache terminee")
}

/// Marque la tache courante terminee et rend la main.
///
/// Si d'autres threads du programme tournent encore, on bascule sur eux ;
/// sinon, retour au fil noyau qui a lance le programme.
pub fn exit_current(code: i32) -> ! {
    let cur = unsafe { CURRENT };
    {
        let task = current();
        task.state = TaskState::Zombie;
        // pthread_join s'appuie sur cette ecriture suivie d'un futex_wake.
        let clear = task.clear_child_tid;
        if clear != 0 {
            let process = task.process.clone();
            let mut process = process.borrow_mut();
            process.space.write(clear, &0u32.to_le_bytes());
            drop(process);
            futex_wake(clear, 1);
        }
        let process = task.process.clone();
        let mut process = process.borrow_mut();
        if process.threads > 0 {
            process.threads -= 1;
        }
        process.exit_code = code;
    }

    wake_sleepers();
    if let Some(next) = pick_next(cur) {
        if next != cur {
            // La pile noyau de la tache morte reste vivante jusqu'au nettoyage
            // fait par `reap` depuis le fil noyau.
            unsafe {
                let list = tasks();
                let from_ptr = &mut **list.get_mut(cur).unwrap() as *mut Task;
                let to_ptr = &mut **list.get_mut(next).unwrap() as *mut Task;
                CURRENT = next;
                install(&mut *to_ptr);
                switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp);
            }
            unreachable!("task: reprise d'une tache terminee")
        }
    }
    switch_to_kernel()
}

/// Termine tous les threads du processus courant (`exit_group`).
pub fn exit_group(code: i32) -> ! {
    let (pid, tid) = {
        let task = current();
        (task.process.borrow().pid, task.tid)
    };
    for task in tasks().iter_mut() {
        if task.tid != tid && task.process.borrow().pid == pid {
            task.state = TaskState::Zombie;
        }
    }
    exit_current(code)
}

/// Lance une tache depuis le fil noyau et attend la fin du programme.
///
/// Renvoie le code de sortie du processus.
pub fn run(first: Box<Task>) -> i32 {
    let process = first.process.clone();
    let index = register(first);
    unsafe {
        CURRENT = index;
        let list = tasks();
        let to_ptr = &mut **list.get_mut(index).unwrap() as *mut Task;
        install(&mut *to_ptr);
        switch_context(&mut KERNEL_CTX.rsp, (*to_ptr).ctx.rsp);
    }
    // Retour ici quand plus aucune tache n'est prete.
    crate::kernel::vmm::activate_kernel();
    unsafe { CURRENT = usize::MAX };
    let (code, pid) = {
        let borrowed = process.borrow();
        (borrowed.exit_code, borrowed.pid)
    };
    reap();
    // Retire l'entree de la table `ps` : le programme n'existe plus.
    crate::kernel::process::kill(pid);
    code
}

/// Detruit les taches zombies (piles noyau, espaces d'adressage).
pub fn reap() {
    tasks().retain(|t| t.state != TaskState::Zombie);
}

/// Termine de force toutes les taches (utilise apres une faute fatale).
pub fn kill_all(code: i32) {
    for task in tasks().iter_mut() {
        task.state = TaskState::Zombie;
        task.process.borrow_mut().exit_code = code;
    }
}

// --- Preemption --------------------------------------------------------------

/// Appele par IRQ0 quand le timer a interrompu du code ring 3.
///
/// On ne commute que si une autre tache est prete : sinon on economise deux
/// changements de contexte par tick.
pub fn preempt_from_irq() {
    if unsafe { CURRENT } == usize::MAX {
        return;
    }
    wake_sleepers();
    if ready_count() < 2 {
        return;
    }
    let cur = unsafe { CURRENT };
    if let Some(next) = pick_next(cur) {
        if next != cur {
            switch_to(cur, next);
        }
    }
}

/// Signale qu'une commutation est souhaitable au prochain point sur.
pub fn set_need_resched() {
    unsafe { NEED_RESCHED = true };
}

/// Consomme le drapeau de preemption.
pub fn take_need_resched() -> bool {
    unsafe {
        let value = NEED_RESCHED;
        NEED_RESCHED = false;
        value
    }
}

// --- Sommeil -----------------------------------------------------------------

/// Endort la tache courante pendant `ticks` ticks du timer.
pub fn sleep_ticks(ticks: u64) {
    let deadline = crate::kernel::timer::ticks() + ticks.max(1);
    {
        let task = current();
        task.wake_tick = deadline;
        task.state = TaskState::Blocked;
    }
    while crate::kernel::timer::ticks() < deadline {
        if !schedule() {
            cpu::hlt();
        }
        if current().state == TaskState::Ready {
            break; // reveille par un signal / un futex
        }
    }
    let task = current();
    task.wake_tick = 0;
    task.state = TaskState::Ready;
}

/// Reveille les taches dont le sommeil est echu.
fn wake_sleepers() {
    let now = crate::kernel::timer::ticks();
    for task in tasks().iter_mut() {
        if task.state == TaskState::Blocked && task.wake_tick != 0 && now >= task.wake_tick {
            task.wake_tick = 0;
            task.futex_key = 0;
            task.state = TaskState::Ready;
        }
    }
}

/// Cede le CPU une fois (`sched_yield`).
pub fn yield_now() {
    schedule();
}

// --- Futex -------------------------------------------------------------------

/// Cle d'attente d'un futex : adresse physique du mot surveille, pour que deux
/// threads partageant la page s'accordent meme via des adresses virtuelles
/// differentes.
fn futex_key(uaddr: u64) -> u64 {
    let process = current().process.clone();
    let mut process = process.borrow_mut();
    process.space.translate(uaddr).unwrap_or(uaddr)
}

/// `FUTEX_WAIT` : endort la tache si `*uaddr == expected`.
///
/// `timeout_ticks` a 0 signifie « sans limite ». Renvoie `true` si la tache a
/// ete reveillee par un `FUTEX_WAKE`, `false` sur delai expire.
pub fn futex_wait(uaddr: u64, expected: u32, timeout_ticks: u64) -> bool {
    let key = futex_key(uaddr);
    // Verification atomique vis-a-vis des autres taches : le noyau n'est pas
    // preemptible ici, donc lire puis dormir est indivisible.
    let mut value = [0u8; 4];
    {
        let process = current().process.clone();
        let mut process = process.borrow_mut();
        if !process.space.read(uaddr, &mut value) {
            return false;
        }
    }
    if u32::from_le_bytes(value) != expected {
        return true; // EAGAIN cote appelant
    }

    let deadline = if timeout_ticks == 0 { 0 } else { crate::kernel::timer::ticks() + timeout_ticks };
    {
        let task = current();
        task.futex_key = key;
        task.wake_tick = deadline;
        task.state = TaskState::Blocked;
    }

    loop {
        if !schedule() {
            cpu::hlt();
            wake_sleepers();
        }
        // L'ordre des deux tests compte. `wake_sleepers` remet la tache en
        // `Ready` des que son echeance est atteinte, exactement comme le ferait
        // un `FUTEX_WAKE` : tester l'etat en premier ferait passer tout delai
        // expire pour un reveil. La libc croirait alors avoir ete signalee, se
        // rendormirait pour la meme duree, et `pthread_cond_timedwait`
        // attendrait un multiple de ce qu'on lui a demande.
        let expired = deadline != 0 && crate::kernel::timer::ticks() >= deadline;
        let task = current();
        if expired {
            task.futex_key = 0;
            task.wake_tick = 0;
            task.state = TaskState::Ready;
            return false;
        }
        if task.state == TaskState::Ready {
            task.futex_key = 0;
            task.wake_tick = 0;
            return true;
        }
    }
}

/// `FUTEX_WAKE` : reveille jusqu'a `count` taches en attente sur `uaddr`.
/// Renvoie le nombre de taches reveillees.
pub fn futex_wake(uaddr: u64, count: u32) -> u32 {
    let key = futex_key(uaddr);
    let mut woken = 0;
    for task in tasks().iter_mut() {
        if woken >= count {
            break;
        }
        if task.state == TaskState::Blocked && task.futex_key == key {
            task.futex_key = 0;
            task.wake_tick = 0;
            task.state = TaskState::Ready;
            woken += 1;
        }
    }
    woken
}

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
        let process = task.process.borrow();
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
            process.space.mapped_pages(),
            process.name
        );
    }
}

/// Cree un processus vide (espace d'adressage neuf, descripteurs standards).
pub fn new_process(name: &str, cwd: usize) -> Option<Rc<RefCell<Process>>> {
    let space = AddressSpace::new()?;
    let pid = crate::kernel::process::spawn(name, crate::users::session().uid());
    Some(Rc::new(RefCell::new(Process {
        pid,
        name: name.to_string(),
        space,
        files: FdTable::new(),
        brk_start: 0,
        brk: 0,
        mmap_next: crate::kernel::vmm::user_mmap_base(),
        cwd,
        exit_code: 0,
        threads: 1,
        uid: crate::users::session().uid() as u32,
        gid: crate::users::session().uid() as u32,
    })))
}
