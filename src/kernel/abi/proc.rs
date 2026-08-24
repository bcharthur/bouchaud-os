//! Cycle de vie des processus : `fork`, `execve`, `wait4`, signaux.
//!
//! Ces quatre appels forment un tout : un shell `fork`, l'enfant `execve`, le
//! parent `wait4`, et le noyau lui envoie `SIGCHLD`. Les implementer separement
//! n'aurait pas de sens — c'est leur enchainement qui doit fonctionner.

use alloc::sync::Arc;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::{smp, usermode::{self, TrapFrame}};
use crate::kernel::abi::{errno, user_read_u64, user_write, user_write_u32};
use crate::kernel::signal::{self, SigAction, SIG_DFL, SIG_IGN};
use crate::kernel::task::{self, FileTable, Mm, MmState, Process, ProcessLifecycle, ProcessMetadata, Task};
use crate::kernel::sync::SpinLock;
use crate::kernel::{elf, vmm};

static EXEC_QUIESCE_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static EXEC_QUIESCE_MAX_NS: AtomicU64 = AtomicU64::new(0);

pub fn exec_quiesce_stats() -> (u64, u64) {
    (
        EXEC_QUIESCE_WAIT_NS.load(Ordering::Relaxed),
        EXEC_QUIESCE_MAX_NS.load(Ordering::Relaxed),
    )
}

/// `fork` : duplique le processus courant.
///
/// La copie de l'espace d'adressage est immediate (voir
/// `AddressSpace::duplicate`). La table de descripteurs est clonee, ce qui
/// partage les objets sous-jacents — tubes, sockets, `eventfd` : c'est
/// exactement ce que POSIX demande, et ce sur quoi repose le chainage
/// `cmd1 | cmd2`.
pub fn sys_fork(frame: &TrapFrame) -> i64 {
    let parent = task::current_process();

    let (space, brk_start, brk, mmap_next, partages, limite_as, promesses, clean_pages) = {
        let mm = parent.mm.lock();
        let space = match mm.space.duplicate() {
            Some(space) => space,
            None => return -errno::ENOMEM,
        };
        (space, mm.brk_start, mm.brk, mm.mmap_next, mm.partages.clone(),
            mm.limite_as, mm.promesses.clone(), mm.clean_pages.clone())
    };
    let files = parent.files.lock().clone();
    let (cwd, uid, gid, name, ecran) = {
        let metadata = parent.metadata.lock();
        (metadata.cwd, metadata.uid, metadata.gid, metadata.name.clone(), metadata.ecran)
    };
    let signals = parent.signals.lock().clone();
    let parent_pid = parent.pid;
    let resource_group_id = parent.resource_group_id;
    let resource_group_name = parent.resource_group_name.clone();

    // `duplicate` a re-mappe les pages empruntees sur les memes frames — c'est
    // la semantique de `MAP_SHARED` a travers `fork`. Le fils tient donc
    // reellement ces plages, et doit en prendre les references : sans cela, le
    // premier `munmap` du pere evincerait des frames que le fils lit encore.
    let mut retained_clean = Vec::new();
    for mapping in &clean_pages {
        if !crate::kernel::clean_page_cache::retain(mapping.key) {
            for key in retained_clean {
                crate::kernel::clean_page_cache::release(key);
            }
            drop(space);
            return -errno::ENOMEM;
        }
        retained_clean.push(mapping.key);
    }
    for plage in &partages {
        crate::kernel::partage::mappe(plage.node);
    }

    let pid = crate::kernel::process::spawn(&name, uid as u16);
    let mut child_signals = signals;
    // Les signaux en attente ne sont pas herites : ils appartenaient au parent.
    child_signals.pending = 0;

    let child = Arc::new(Process {
        pid,
        parent: parent_pid,
        resource_group_id,
        resource_group_name,
        mm: Arc::new(Mm::new(MmState { space, brk_start, brk, mmap_next,
            partages, limite_as, promesses, clean_pages })),
        files: Arc::new(FileTable::new(files)),
        metadata: SpinLock::new(ProcessMetadata { name, cwd, uid, gid, ecran }),
        lifecycle: SpinLock::new(ProcessLifecycle { exit_code: 0, zombie: false, threads: 1 }),
        signals: SpinLock::new(child_signals),
    });
    task::register_process(child.clone());

    // L'enfant reprend a l'instruction suivant le `syscall`, avec 0 dans rax :
    // c'est la seule chose qui distingue les deux retours de `fork`.
    let mut child_frame = *frame;
    child_frame.rax = 0;
    let mut child_task = Task::new(child, child_frame);
    // La base FS doit suivre : c'est le TLS, et la premiere chose que fait la
    // libc dans l'enfant est de lire `%fs:0` pour retrouver sa structure de
    // thread. Une base nulle la ferait dereferencer l'adresse 0.
    child_task.fs_base = usermode::fs_base();
    task::register(child_task);

    pid as i64
}

/// Lit un tableau de chaines C termine par un pointeur nul (`argv`, `envp`).
fn read_string_array(addr: u64) -> Option<Vec<String>> {
    let mut out = Vec::new();
    if addr == 0 {
        return Some(out);
    }
    for index in 0..1024u64 {
        let pointer = user_read_u64(addr + index * 8)?;
        if pointer == 0 {
            break;
        }
        // La chaine peut vivre dans une page file-backed encore non
        // residente (typiquement .rodata du programme qui appelle execve).
        // `user_string` sait materialiser ces pages avant copyin.
        out.push(super::user_string(pointer)?);
    }
    Some(out)
}

/// `execve` : remplace l'image du processus courant.
///
/// Ne revient jamais en cas de succes — le processus repart au point d'entree
/// du nouveau programme.
pub fn sys_execve(path_addr: u64, argv_addr: u64, envp_addr: u64) -> i64 {
    // Tout doit etre lu **avant** de detruire l'ancien espace d'adressage :
    // le chemin, les arguments et l'environnement y vivent encore.
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => path,
        None => return -errno::EFAULT,
    };
    let argv = match read_string_array(argv_addr) {
        Some(list) => list,
        None => return -errno::EFAULT,
    };
    let envp = match read_string_array(envp_addr) {
        Some(list) => list,
        None => return -errno::EFAULT,
    };

    let process = task::current_process();
    let cwd = process.metadata.lock().cwd;

    let node = {
        let fs = crate::fs::ramfs::fs();
        match fs.resolve(&path, cwd) {
            Some(node) if fs.nodes[node].kind == crate::fs::ramfs::NodeKind::File => node,
            Some(_) => return -errno::EISDIR,
            None => return -errno::ENOENT,
        }
    };
    if elf::parse_node(node).is_err() {
        return -errno::ENOEXEC;
    }

    let mut space = match vmm::AddressSpace::new() {
        Some(space) => space,
        None => return -errno::ENOMEM,
    };

    let mut new_promises = Vec::new();
    let image = match elf::load_node_lazy(node, vmm::user_load_base(), &mut new_promises) {
        Ok(image) => image,
        Err(_) => return -errno::ENOEXEC,
    };
    let (entry, interp_base) = match image.interp.as_deref() {
        None => (image.entry, 0),
        Some(interp_path) => {
            let fs = crate::fs::ramfs::fs();
            let interp_node = match fs.resolve(interp_path, cwd) {
                Some(node) => node,
                None => return -errno::ENOENT,
            };
            match elf::load_node_lazy(interp_node, vmm::user_interp_base(), &mut new_promises) {
                Ok(interp) => (interp.entry, interp.base),
                Err(_) => return -errno::ENOEXEC,
            }
        }
    };

    let (uid, gid) = {
        let metadata = process.metadata.lock();
        (metadata.uid, metadata.gid)
    };
    let argv = if argv.is_empty() {
        alloc::vec![path.clone()]
    } else {
        argv
    };
    let layout = elf::StackLayout {
        argv: &argv,
        envp: &envp,
        image: &image,
        interp_base,
        uid,
        gid,
    };
    let stack = match elf::build_stack(&mut space, &layout) {
        Ok(stack) => stack,
        Err(_) => return -errno::ENOMEM,
    };

    // All fallible image construction is complete. Only now terminate sibling
    // tasks and retire the old identity: failed execve must leave both intact.
    task::terminate_sibling_threads();
    // Stop every sibling on the old CR3 before replacement.
    let old_identity = process.mm.lock().space.identity();
    old_identity.begin_retire();
    let self_cpu = smp::cpu_index();
    let active = old_identity.active_cpus() & !(1u64 << self_cpu);
    let depth = crate::kernel::smp_lock::suspend_for_schedule();
    for cpu in 0..smp::MAX_CPUS.min(64) {
        if active & (1u64 << cpu) != 0 {
            smp::reschedule_cpu(cpu);
        }
    }
    let wait_start = crate::kernel::timer::monotonic_ns();
    old_identity.wait_remote_quiescent(self_cpu);
    let waited = crate::kernel::timer::monotonic_ns().saturating_sub(wait_start);
    EXEC_QUIESCE_WAIT_NS.fetch_add(waited, Ordering::Relaxed);
    EXEC_QUIESCE_MAX_NS.fetch_max(waited, Ordering::Relaxed);
    crate::kernel::smp_lock::resume_after_schedule(depth);

    process.metadata.lock().name = path;
    process.files.lock().close_on_exec();
    process.signals.lock().reset_for_exec();
    process.lifecycle.lock().threads = 1;
    let (old, old_clean, old_shared) = {
        let mut mm = process.mm.lock();
        mm.brk_start = (image.end + 0x10_0000) & !0xFFF;
        mm.brk = mm.brk_start;
        mm.mmap_next = vmm::user_mmap_base();
        // L'ancien espace disparait avec ses mappages : les references qu'il
        // tenait sur le cache partage doivent partir avec lui.
        let old_shared = core::mem::take(&mut mm.partages);
        let old_clean = core::mem::take(&mut mm.clean_pages);
        mm.promesses = new_promises;
        // L'ancien espace est detruit ici. Son `Drop` remet CR3 sur la table du
        // noyau s'il etait actif : c'est pour cela qu'on active le nouveau
        // juste apres, avant de retourner en ring 3.
        let old = core::mem::replace(&mut mm.space, space);
        let new_identity = mm.space.identity();
        drop(mm);
        process.mm.replace_activation(new_identity);
        process.mm.activate();
        old_identity.mark_inactive(smp::cpu_index());
        (old, old_clean, old_shared)
    };
    task::forget_fault_space(old.pml4());
    for mapping in old_clean {
        crate::kernel::clean_page_cache::release(mapping.key);
    }
    for mapping in old_shared {
        crate::kernel::partage::demappe(mapping.node);
    }
    drop(old);

    // La tache repart de zero : nouvelle trame, pile noyau reinitialisee. La
    // pile noyau courante (celle de cet appel systeme) est abandonnee telle
    // quelle — c'est sans consequence, elle repart du sommet a la prochaine
    // entree.
    let task = task::current();
    task.fs_base = 0;
    task.clear_child_tid = 0;
    usermode::set_fs_base(0);
    usermode::set_kernel_stack(task.kstack_top);
    let frame = TrapFrame::new_user(entry, stack);
    task.frame = frame;

    // BOUCHAUD_EXECVE_NORETURN_BKL_FIX_V1
    //
    // `sys_execve` ne retourne jamais vers `syscall_dispatch`: il saute
    // directement vers le nouveau ring3 via `resume_usermode`. Le RAII
    // `KernelGuard` cree dans syscall_dispatch est donc abandonne sur l'ancienne
    // pile noyau et son Drop ne peut jamais liberer le BKL.
    //
    // Avant ce fix, un execve reussi laissait OWNER=CPU courant et DEPTH=1
    // indefiniment. Le nouveau programme pouvait etre preempte, mais toute la
    // machine continuait avec un BKL fantome, ce qui gelait Ladybird.
    //
    // Comme cette pile ne reprendra jamais, on libere explicitement toute la
    // profondeur BKL et on ferme aussi la sonde syscall avant l'iretq.
    task::stall_site_clear();
    task::stall_syscall_exit();
    task::account_resume_user_noreturn();
    let abandoned_depth = crate::kernel::smp_lock::suspend_for_schedule();
    debug_assert!(
        abandoned_depth > 0,
        "execve: chemin no-return sans BKL syscall actif"
    );

    unsafe { usermode::resume_usermode(&task.frame) }
}

/// `wait4` : attend la fin d'un processus fils.
pub fn sys_wait4(pid: i64, status_addr: u64, options: u32, _rusage: u64) -> i64 {
    const WNOHANG: u32 = 1;
    let parent_pid = task::current_process().pid;

    loop {
        let zombies = task::zombie_children(parent_pid);
        let found = zombies
            .iter()
            .find(|(child, _)| pid <= 0 || *child == pid as u32)
            .copied();

        if let Some((child_pid, code)) = found {
            if status_addr != 0 {
                // Encodage `wait` : octet de poids faible = cause, octet
                // suivant = code de sortie. Un code >= 128 signale une mort par
                // signal, comme le fait `kill_faulting_task`.
                let status = if code >= 128 {
                    (code as u32 - 128) & 0x7F
                } else {
                    ((code as u32) & 0xFF) << 8
                };
                user_write_u32(status_addr, status);
            }
            task::collect_child(child_pid);
            return child_pid as i64;
        }

        if !task::has_children(parent_pid) {
            return -errno::ECHILD;
        }
        if options & WNOHANG != 0 {
            return 0;
        }

        // Blocage jusqu'a ce qu'un fils se termine : c'est `exit_current` qui
        // remettra cette tache en etat pret.
        {
            let task = task::current();
            task.waiting_for_child = true;
            task.state = task::TaskState::Blocked;
        }
        // schedule() effectue deja HLT avec le BKL suspendu si necessaire.
        let _ = task::schedule();
        let task = task::current();
        task.waiting_for_child = false;
        task.state = task::TaskState::Ready;
    }
}

// --- Signaux -----------------------------------------------------------------

/// `rt_sigaction`.
pub fn sys_rt_sigaction(signal: u32, act: u64, oldact: u64) -> i64 {
    if signal == 0 || signal as usize > signal::NSIG {
        return -errno::EINVAL;
    }
    // SIGKILL et SIGSTOP ne peuvent etre ni interceptes ni ignores.
    if act != 0 && (signal == signal::SIGKILL || signal == signal::SIGSTOP) {
        return -errno::EINVAL;
    }
    let process = task::current_process();
    let index = signal as usize - 1;

    if oldact != 0 {
        let previous = process.signals.lock().actions[index];
        if !signal::write_sigaction(oldact, &previous) {
            return -errno::EFAULT;
        }
    }
    if act != 0 {
        let action = match signal::read_sigaction(act) {
            Some(action) => action,
            None => return -errno::EFAULT,
        };
        process.signals.lock().actions[index] = action;
    }
    0
}

/// `rt_sigprocmask`.
pub fn sys_rt_sigprocmask(how: i32, set: u64, oldset: u64) -> i64 {
    let process = task::current_process();
    let current_mask = process.signals.lock().blocked;
    if oldset != 0 && !user_write(oldset, &current_mask.to_le_bytes()) {
        return -errno::EFAULT;
    }
    if set != 0 {
        let new = match user_read_u64(set) {
            Some(value) => value,
            None => return -errno::EFAULT,
        };
        let mut signals = process.signals.lock();
        signals.blocked = match how {
            signal::SIG_BLOCK => current_mask | new,
            signal::SIG_UNBLOCK => current_mask & !new,
            signal::SIG_SETMASK => new,
            _ => return -errno::EINVAL,
        };
        // Ces deux-la ne se bloquent pas, quoi qu'on demande.
        signals.blocked &= !(1 << (signal::SIGKILL - 1));
        signals.blocked &= !(1 << (signal::SIGSTOP - 1));
    }
    0
}

/// `rt_sigreturn` : retour d'un gestionnaire de signal.
///
/// Ne renvoie pas de valeur : la trame entiere est ecrasee par l'etat
/// sauvegarde au moment de la livraison.
pub fn sys_rt_sigreturn(frame: &mut TrapFrame) -> i64 {
    match signal::restore(frame) {
        Some(mask) => {
            task::current_process().signals.lock().blocked = mask;
            frame.rax as i64
        }
        None => {
            // Trame illisible : le gestionnaire a corrompu sa pile, il n'y a
            // aucun etat coherent ou revenir.
            task::exit_group(128 + signal::SIGSEGV as i32)
        }
    }
}

/// Envoie un signal a un processus (`kill`) ou a un thread (`tkill`).
pub fn send_signal(pid: u32, signal: u32) -> i64 {
    if signal == 0 {
        // Signal 0 : simple test d'existence du processus.
        return if task::process_by_pid(pid).is_some() {
            0
        } else {
            -errno::ESRCH
        };
    }
    if signal as usize > signal::NSIG {
        return -errno::EINVAL;
    }
    match task::process_by_pid(pid) {
        Some(process) => {
            process.signals.lock().raise(signal);
            // Un signal doit reveiller une tache endormie, sinon un `SIGTERM`
            // sur un processus bloque resterait sans effet.
            task::wake_for_signal(pid);
            0
        }
        None => -errno::ESRCH,
    }
}

/// `kill(pid, sig)`.
pub fn sys_kill(pid: i64, signal: u32) -> i64 {
    if pid <= 0 {
        // Groupes de processus : on traite comme « moi-meme ».
        let self_pid = task::current_process().pid;
        return send_signal(self_pid, signal);
    }
    send_signal(pid as u32, signal)
}

/// `tkill(tid, sig)` : le premier argument est un **identifiant de thread**,
/// pas de processus.
///
/// La distinction n'est pas theorique : c'est par cet appel que `raise()` de
/// musl s'envoie un signal a lui-meme. Le confondre avec un pid fait echouer
/// tout `raise` en `ESRCH`, donc `abort()`, `assert()` et tout gestionnaire
/// declenche depuis le programme lui-meme.
pub fn sys_tkill(tid: u32, signal: u32) -> i64 {
    match task::process_of_tid(tid) {
        Some(process) => {
            let pid = process.pid;
            send_signal(pid, signal)
        }
        None => -errno::ESRCH,
    }
}

/// Livre au plus un signal en attente avant le retour en ring 3.
///
/// Appele depuis le dispatch d'appel systeme, seul endroit ou la trame ring 3
/// est a la fois accessible et modifiable.
pub fn deliver_pending(frame: &mut TrapFrame) {
    if !task::in_user_task() {
        return;
    }
    loop {
        let process = task::current_process();
        let (signal, action, blocked) = {
            let signals = process.signals.lock();
            match signals.next_deliverable() {
                None => return,
                Some(signal) => (
                    signal,
                    signals.actions[signal as usize - 1],
                    signals.blocked,
                ),
            }
        };
        process.signals.lock().clear(signal);

        if action.handler == SIG_IGN {
            continue;
        }
        if action.handler == SIG_DFL {
            match signal::default_action(signal) {
                signal::DefaultAction::Ignore => continue,
                signal::DefaultAction::Terminate => {
                    crate::serial_println!(
                        "[signal] {} non intercepte : processus termine",
                        signal::name(signal)
                    );
                    task::exit_group(128 + signal as i32);
                }
            }
        }
        // SIGKILL et SIGSTOP ne sont jamais detournables, meme avec un
        // gestionnaire installe (rt_sigaction le refuse deja, ceinture et
        // bretelles).
        if signal == signal::SIGKILL || signal == signal::SIGSTOP {
            task::exit_group(128 + signal as i32);
        }

        if !signal::deliver(frame, signal, &action, blocked) {
            crate::serial_println!("[signal] pile utilisateur inaccessible : processus termine");
            task::exit_group(128 + signal::SIGSEGV as i32);
        }

        {
            let mut signals = process.signals.lock();
            // Le signal livre est bloque pendant l'execution de son
            // gestionnaire (sauf SA_NODEFER), plus ceux demandes par sa_mask :
            // c'est ce qui evite qu'il se reentre indefiniment.
            signals.blocked |= action.mask;
            if action.flags & signal::SA_NODEFER == 0 {
                signals.blocked |= 1 << (signal - 1);
            }
            if action.flags & signal::SA_RESETHAND != 0 {
                signals.actions[signal as usize - 1] = SigAction::default();
            }
        }
        // Un seul signal par retour : le suivant sera livre au retour du
        // gestionnaire courant.
        return;
    }
}
