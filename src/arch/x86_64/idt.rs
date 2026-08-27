// BOUCHAUD_SMP4_DEADLOCK_FIX
//! IDT partagee en contenu, chargee separement sur chaque CPU.

use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crate::arch::x86_64::{gdt, ports, smp, usermode};
use crate::arch::x86_64::interrupts::{notify_end_of_interrupt, InterruptIndex};
use crate::drivers::{keyboard, mouse};
use crate::kernel::{dmesg, timer};
use crate::serial_println;

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();
static mut READY: bool = false;

pub fn state() -> &'static str {
    if unsafe { READY } {
        "initialisee (exceptions + IRQ BSP + IPI SMP)"
    } else {
        "non chargee"
    }
}

pub fn init() {
    unsafe {
        IDT.breakpoint.set_handler_fn(breakpoint_handler);
        IDT.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        IDT.page_fault.set_handler_fn(page_fault_handler);
        IDT.general_protection_fault.set_handler_fn(general_protection_handler);
        IDT.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        IDT.divide_error.set_handler_fn(divide_error_handler);
        IDT.stack_segment_fault.set_handler_fn(stack_segment_handler);
        IDT[InterruptIndex::Timer.as_usize()].set_handler_fn(timer_interrupt_handler);
        IDT[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);
        IDT[InterruptIndex::Mouse.as_usize()].set_handler_fn(mouse_interrupt_handler);
        IDT[InterruptIndex::AtaPrimary.as_usize()].set_handler_fn(ata_primary_handler);
        IDT[InterruptIndex::AtaSecondary.as_usize()].set_handler_fn(ata_secondary_handler);
        IDT[smp::RESCHEDULE_VECTOR as usize].set_handler_fn(reschedule_interrupt_handler);
        IDT[smp::PANIC_STOP_VECTOR as usize].set_handler_fn(panic_stop_handler);
        IDT[smp::TLB_SHOOTDOWN_VECTOR as usize].set_handler_fn(tlb_shootdown_interrupt_handler);
        IDT.load();
        READY = true;
    }
    dmesg::log("idt: IDT chargee (exceptions + IRQ + IPI reschedule/TLB SMP-NG2)");
}

/// IDTR est un registre par CPU : les AP rechargent la meme table immutable.
pub fn load_ap() {
    unsafe { IDT.load(); }
}

pub fn trigger_breakpoint() {
    x86_64::instructions::interrupts::int3();
}

extern "x86-interrupt" fn breakpoint_handler(stack: InterruptStackFrame) {
    let _kernel = crate::kernel::smp_lock::enter();
    println!("exception: breakpoint (int3) capturee, on continue");
    serial_println!("[cpu] breakpoint at {:?}", stack.instruction_pointer);
}

// BOUCHAUD_DF_FORENSIC_V1
//
// Un double fault est fatal, et il arrive precisement quand l'etat du noyau
// n'est plus fiable. Le gestionnaire doit donc rendre le maximum d'etat AVANT
// de faire quoi que ce soit qui pourrait echouer a son tour.
//
// Ce qu'il ne fait jamais : allouer, prendre un verrou, appeler du code qui
// pourrait fauter. `serial_println!` ecrit directement sur le port serie.
//
// Le run du 26 aout a donne `RIP=0x1b CS=8 RSP=0x18102618040 SS=16`. Dans la
// GDT de ce noyau, 0x1b est le selecteur de DONNEES UTILISATEUR : une valeur de
// segment s'etait retrouvee la ou RIP est attendu. Ce n'est pas une ecriture
// sauvage, c'est un cadre de pile decale ou un contexte restaure depuis une
// pile fausse. Distinguer les deux demande de savoir si RSP appartient encore
// a la pile noyau de la tache -- d'ou ce releve.
static PANIC_GLOBAL: AtomicBool = AtomicBool::new(false);
static CPU_FAUTIF: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Un autre CPU a-t-il deja declare la panique ?
pub fn panique_globale_en_cours() -> bool {
    PANIC_GLOBAL.load(Ordering::Acquire)
}

/// Prend la panique globale. Rend `false` si un autre CPU l'a deja prise :
/// l'appelant doit alors se taire et s'arreter, pour laisser le premier
/// produire une sortie lisible plutot que d'entrelacer deux traces.
pub fn prends_la_panique(cpu: usize) -> bool {
    if PANIC_GLOBAL
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    CPU_FAUTIF.store(cpu, Ordering::Release);
    true
}

/// Arrete definitivement ce CPU, sans rien ecrire.
pub fn arret_definitif() -> ! {
    loop {
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)) };
    }
}

/// Releve complet de l'etat au moment d'une faute fatale.
///
/// Aucune allocation, aucun verrou, aucun appel qui puisse fauter.
fn releve_faute_fatale(nom: &str, stack: &InterruptStackFrame, code: u64) {
    let cpu = smp::cpu_index();
    let rsp_frame = stack.stack_pointer.as_u64();

    serial_println!("");
    serial_println!("======== [{}] ========", nom);
    serial_println!(
        "[FAULT] cpu={} apic={} code={:#x}",
        cpu,
        crate::arch::x86_64::cpu_local::CpuId::from_index(cpu)
            .and_then(crate::arch::x86_64::cpu_local::descriptor)
            .map(|d| d.apic_id)
            .unwrap_or(u32::MAX),
        code,
    );
    serial_println!(
        "[FAULT] rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
        stack.instruction_pointer.as_u64(),
        stack.code_segment,
        stack.cpu_flags,
        rsp_frame,
        stack.stack_segment,
    );

    let (cr2, cr3) = unsafe {
        let cr2: u64;
        let cr3: u64;
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack));
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        (cr2, cr3)
    };
    serial_println!("[FAULT] cr2={:#x} cr3={:#x}", cr2, cr3);

    releve_contexte_courant(cpu, Some(rsp_frame));
    serial_println!("======== fin du releve ========");
}

/// Partie du releve qui ne depend PAS d'une trame d'exception : identite de la
/// tache, pile noyau attendue, etat du gros verrou.
///
/// Le `panic!` Rust n'a pas d'`InterruptStackFrame` -- et c'est justement lui
/// qui declenche les assertions du BKL. Il a pourtant besoin exactement de ces
/// informations-la, d'ou le partage.
///
/// `rsp_connu` n'est renseigne que quand une trame le donne ; sans lui on ne
/// peut pas conclure « pile debordee » contre « pile fausse », et on ne le
/// pretend donc pas.
pub fn releve_contexte_courant(cpu: usize, rsp_connu: Option<u64>) {
    // La pile attendue. C'est la reponse qui tranche entre « pile debordee »
    // et « contexte restaure depuis une pile fausse » : dans le premier cas
    // RSP est juste SOUS la base, dans le second il est ailleurs.
    match crate::kernel::task::identite_pour_faute() {
        Some((index, pid, tid, kstack_top, kstack_base, in_kernel)) => {
            serial_println!(
                "[FAULT] task={} pid={} tid={} nom={} in_kernel={}",
                index, pid, tid, crate::kernel::task::nom_pour_faute(), in_kernel,
            );
            serial_println!(
                "[FAULT] kstack base={:#x} top={:#x} taille={}",
                kstack_base, kstack_top, kstack_top.saturating_sub(kstack_base),
            );
            match rsp_connu {
                Some(rsp) => serial_println!(
                    "[FAULT] rsp_dans_kstack={} ecart_sous_base={}",
                    rsp >= kstack_base && rsp <= kstack_top,
                    kstack_base.saturating_sub(rsp),
                ),
                None => serial_println!("[FAULT] rsp_dans_kstack=<pas de trame>"),
            }
        }
        None => serial_println!("[FAULT] task=<aucune> (idle ou avant premier switch)"),
    }

    serial_println!(
        "[FAULT] tss_rsp0={:#x} gs_cpu_index={}",
        gdt::rsp0_courant(cpu),
        usermode::cpu_index(),
    );

    let provenance = crate::kernel::smp_lock::stall_probe_provenance();
    serial_println!(
        "[FAULT] bkl owner_token={} depth={} coherent={} acquire_kind={} acquire_seq={} release_seq={}",
        provenance.owner_token,
        crate::kernel::smp_lock::stall_probe_depth(cpu),
        provenance.coherent,
        provenance.acquire_kind,
        provenance.acquire_seq,
        provenance.release_seq,
    );
    serial_println!(
        "[FAULT] syscall_nr={} phase={} site={} aux={:#x} task_bkl={}",
        provenance.syscall_nr,
        provenance.syscall_phase,
        provenance.site,
        provenance.aux,
        provenance.task,
    );
    // Pas de compteur d'imbrication d'IRQ dans ce noyau : ne pas en inventer
    // un ici. `need_resched` dit au moins si une preemption etait en attente au
    // moment de la faute.
    serial_println!(
        "[FAULT] need_resched={} irq_profondeur=<non suivi>",
        crate::kernel::task::besoin_de_replanifier(),
    );
}

/// Recu par les CPU secondaires quand un autre a pris la panique globale.
///
/// Ne fait rien d'autre que s'arreter : ni verrou, ni journal, ni commutation.
/// Ecrire ici entrelacerait la sortie avec celle du CPU fautif.
extern "x86-interrupt" fn panic_stop_handler(_stack: InterruptStackFrame) {
    arret_definitif();
}

extern "x86-interrupt" fn double_fault_handler(stack: InterruptStackFrame, code: u64) -> ! {
    let cpu = smp::cpu_index();

    // Un second CPU qui double-faute pendant qu'on ecrit n'a rien a ajouter :
    // sa trace entrelacee rendrait les deux illisibles. Il s'arrete.
    if !prends_la_panique(cpu) {
        arret_definitif();
    }

    releve_faute_fatale("DOUBLE FAULT", &stack, code);

    // Les autres CPU sont arretes APRES le releve : s'ils s'arretaient avant,
    // une faute pendant le releve laisserait la machine muette et figee.
    smp::arrete_les_autres_cpu();

    serial_println!("*** KERNEL PANIC *** double faute, cpu={}", cpu);
    arret_definitif();
}

fn from_user(stack: &InterruptStackFrame) -> bool {
    stack.code_segment & 3 == 3
}

struct GsGuard { swapped: bool }

impl GsGuard {
    fn enter(stack: &InterruptStackFrame) -> Self {
        let swapped = from_user(stack);
        if swapped {
            unsafe { crate::arch::x86_64::usermode::swapgs() };
        }
        GsGuard { swapped }
    }
}

impl Drop for GsGuard {
    fn drop(&mut self) {
        if self.swapped {
            unsafe { crate::arch::x86_64::usermode::swapgs() };
        }
    }
}

fn kill_faulting_task(reason: &str, stack: &InterruptStackFrame) -> ! {
    let _kernel = crate::kernel::smp_lock::enter();
    let cr2 = x86_64::registers::control::Cr2::read().as_u64();
    crate::println!(
        "{} dans le programme utilisateur (rip={:#x}) : processus termine",
        reason,
        stack.instruction_pointer.as_u64()
    );
    serial_println!(
        "[cpu] {} en ring 3 : rip={:#x} rsp={:#x} cr2={:#x} flags={:#x}",
        reason,
        stack.instruction_pointer.as_u64(),
        stack.stack_pointer.as_u64(),
        cr2,
        stack.cpu_flags
    );
    crate::kernel::task::exit_group(139)
}

extern "x86-interrupt" fn general_protection_handler(stack: InterruptStackFrame, code: u64) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("faute de protection generale", &stack);
    }
    // Meme releve que le double fault : une GP en mode noyau a exactement les
    // memes causes candidates -- selecteur invalide, cadre decale, contexte
    // restaure depuis une pile fausse -- et le meme besoin de les distinguer.
    releve_faute_fatale("GENERAL PROTECTION FAULT", &stack, code);
    panic!("EXCEPTION: general protection fault (code {})\n{:#?}", code, stack);
}

extern "x86-interrupt" fn invalid_opcode_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("instruction illegale", &stack);
    }
    panic!("EXCEPTION: instruction illegale\n{:#?}", stack);
}

extern "x86-interrupt" fn divide_error_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("division par zero", &stack);
    }
    panic!("EXCEPTION: division par zero\n{:#?}", stack);
}

extern "x86-interrupt" fn stack_segment_handler(stack: InterruptStackFrame, code: u64) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("faute de pile", &stack);
    }
    releve_faute_fatale("STACK SEGMENT FAULT", &stack, code);
    panic!("EXCEPTION: faute de segment de pile (code {})", code);
}

// BOUCHAUD_SMP4_OWNER_PROVENANCE_PROBE_V3
extern "x86-interrupt" fn page_fault_handler(stack: InterruptStackFrame, code: PageFaultErrorCode) {
    let _gs = GsGuard::enter(&stack);
    let addr = x86_64::registers::control::Cr2::read();
    // L'exception s'execute dans le contexte de la tache interrompue : son
    // marqueur de site doit revenir en place en sortant, sinon la sonde ne
    // reverra jamais le site du syscall qui a pris la faute.
    let _site = crate::kernel::task::SiteIrq::enter(20, addr.as_u64());
    crate::kernel::task::stall_pf_begin(addr.as_u64());
    // Une exception user arrive avec IF masque par la porte IDT. Autoriser les
    // IPI avant toute attente de verrou: un CPU bloque sur la synchronisation
    // MM doit toujours pouvoir ACK un shootdown. IRET restaurera les RFLAGS
    // utilisateur sauvegardes dans `stack`.
    if from_user(&stack) {
        x86_64::instructions::interrupts::enable();
    }
    crate::kernel::task::stall_site_set(21, addr.as_u64());
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        let mut retries = 0u32;
        let outcome = loop {
            let outcome = crate::kernel::task::peuple_a_la_demande(
                addr.as_u64(),
                code.contains(PageFaultErrorCode::PROTECTION_VIOLATION),
            );
            if outcome != crate::kernel::task::FaultOutcome::Retry {
                break outcome;
            }
            retries = retries.wrapping_add(1);
            if retries % 8 == 0 {
                crate::kernel::task::fault_retry_yield();
            } else {
                core::hint::spin_loop();
            }
        };
        crate::kernel::task::fault_retry_chain_complete(retries as u64);
        if outcome == crate::kernel::task::FaultOutcome::Resolved {
            crate::kernel::task::stall_pf_done(addr.as_u64());
            // execve may have retired this sibling while its fault loader was
            // outside the BKL doing I/O. Do not return it to the old user CR3.
            let _kernel = crate::kernel::smp_lock::enter();
            crate::kernel::task::retire_current_if_zombie();
            return;
        }
        if outcome == crate::kernel::task::FaultOutcome::Retired {
            let _kernel = crate::kernel::smp_lock::enter();
            crate::kernel::task::retire_current_if_zombie();
        }
        crate::kernel::task::stall_pf_fail(addr.as_u64());
        crate::kernel::task::log_fault_mapping(addr.as_u64());
        crate::println!("faute de page utilisateur @ {:#x} ({:?})", addr.as_u64(), code);
        kill_faulting_task("faute de page", &stack);
    }
    serial_println!("[cpu] page fault @ {:?} code {:?}", addr, code);
    panic!("EXCEPTION: page fault @ {:?}\ncode: {:?}\n{:#?}", addr, code, stack);
}

extern "x86-interrupt" fn timer_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let interrupted_user = from_user(&stack);

    timer::tick();
    let idle = crate::arch::x86_64::cpu::account_timer_tick(interrupted_user);
    notify_end_of_interrupt(InterruptIndex::Timer.as_u8());

    // BOUCHAUD_SMP4_STALL_PROBE_V1 : volontairement AVANT le BKL.
    crate::kernel::task::stall_probe_from_timer();    let quantum = timer::ticks() % smp::SCHED_QUANTUM_TICKS == 0;
    if quantum && !smp::local_scheduler_timer_enabled() {
        // BOUCHAUD_P0_TARGETED_SCHED_IPI_V1
        //
        // PIT fallback used when local TSC-deadline scheduling is unavailable.
        // Do not broadcast every 4 ms to idle APs. Wake only secondary CPUs
        // that are currently executing a user task.
        let targets = crate::kernel::task::running_user_cpu_mask();
        let online = smp::schedulable_cpus().min(64);
        let mut cpu = 1usize;

        while cpu < online {
            if targets & (1u64 << cpu) != 0 {
                smp::reschedule_cpu(cpu);
            }
            cpu += 1;
        }
    }

    let mut preempt_now = false;
    {
        let _site = crate::kernel::task::SiteIrq::enter(60, 0);
        let Some(_kernel) = crate::kernel::smp_lock::try_enter() else {
            if quantum && crate::kernel::task::in_user_task() {
                crate::kernel::task::request_deferred_preempt();
            }
            return;
        };
        crate::kernel::task::stall_site_set(61, 0);

        if !idle {
            crate::kernel::task::echantillonne_tache_bsp();
        }
        crate::kernel::task::watchdog_from_timer();

        if quantum && crate::kernel::task::in_user_task() {
            if interrupted_user {
                // Le guard sort de ce scope AVANT le context switch IRQ.
                preempt_now = true;
            } else if !crate::kernel::task::current_is_kernel_task() {
                crate::kernel::task::request_deferred_preempt();
            }
        }
    }

    if preempt_now {
        crate::kernel::task::preempt_from_irq();
    }
}


// BOUCHAUD_SMP_NG2_TLB_HANDLER_V1
/// Shootdown TLB: aucun BKL ici. L'emetteur peut justement etre en train de
/// tenir le BKL pendant un munmap/mprotect et attend notre ACK.
extern "x86-interrupt" fn tlb_shootdown_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    smp::handle_tlb_shootdown();
}

/// IPI de quantum sur AP. S'il faudrait attendre le Big Kernel Lock, on ne
/// bloque pas le coeur dans l'IRQ : on pose seulement NEED_RESCHED, qui sera
/// consomme au prochain syscall / point sur.
extern "x86-interrupt" fn reschedule_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let interrupted_user = from_user(&stack);
    crate::kernel::task::stall_ipi_observe(
        stack.instruction_pointer.as_u64(),
        interrupted_user,
    );
    smp::eoi_local();
    if smp::local_scheduler_timer_enabled() {
        smp::arm_local_scheduler_timer();
    }

    let mut preempt_now = false;
    {
        let _site = crate::kernel::task::SiteIrq::enter(30, 0);
        let Some(_kernel) = crate::kernel::smp_lock::try_enter() else {
            crate::kernel::task::stall_ipi_bkl_result(false);
            crate::kernel::task::set_need_resched();
            return;
        };
        crate::kernel::task::stall_ipi_bkl_result(true);
        crate::kernel::task::stall_site_set(31, 0);

        crate::kernel::task::echantillonne_quantum(
            interrupted_user,
            smp::SCHED_QUANTUM_TICKS,
        );

        if crate::kernel::task::in_user_task() {
            if interrupted_user {
                preempt_now = true;
            } else if !crate::kernel::task::current_is_kernel_task() {
                crate::kernel::task::request_deferred_preempt();
            }
        }
    }

    if preempt_now {
        crate::kernel::task::preempt_from_irq();
    }
}


extern "x86-interrupt" fn keyboard_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let interrupted_user = from_user(&stack);

    {
        let _kernel = crate::kernel::smp_lock::enter();
        let status = unsafe { ports::inb(0x64) };
        let data = unsafe { ports::inb(0x60) };
        if status & 0x20 == 0 {
            keyboard::push_scancode(data);
        } else {
            mouse::handle_byte(data);
        }
        notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }

    // Pas de context switch dans IRQ clavier. PIT/IPI preempte sous 4 ms.
    if interrupted_user && crate::kernel::task::in_user_task() {
        crate::kernel::task::request_deferred_preempt();
    }
}


extern "x86-interrupt" fn ata_primary_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    let _ = unsafe { ports::inb(0x1F7) };
    notify_end_of_interrupt(InterruptIndex::AtaPrimary.as_u8());
}

extern "x86-interrupt" fn ata_secondary_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    let _ = unsafe { ports::inb(0x177) };
    notify_end_of_interrupt(InterruptIndex::AtaSecondary.as_u8());
}

extern "x86-interrupt" fn mouse_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    let status = unsafe { ports::inb(0x64) };
    let byte = unsafe { ports::inb(0x60) };
    if status & 0x20 != 0 { mouse::handle_byte(byte); } else { keyboard::push_scancode(byte); }
    notify_end_of_interrupt(InterruptIndex::Mouse.as_u8());
}
