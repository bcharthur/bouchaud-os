//! Interrupt Descriptor Table (IDT) : exceptions CPU + IRQ materielles.
//!
//! On enregistre les exceptions essentielles (breakpoint, double faute, faute de
//! page, faute de protection generale) et les deux IRQ utiles : le timer (IRQ0)
//! qui incremente l'horloge noyau, et le clavier (IRQ1) qui empile les scancodes
//! pour l'editeur de ligne.

use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use crate::arch::x86_64::{gdt, ports};
use crate::arch::x86_64::interrupts::{notify_end_of_interrupt, InterruptIndex};
use crate::drivers::{keyboard, mouse};
use crate::kernel::{dmesg, timer};
use crate::serial_println;

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();
static mut READY: bool = false;

/// Etat courant de l'IDT, expose aux commandes systeme.
pub fn state() -> &'static str {
    if unsafe { READY } {
        "initialisee (exceptions + IRQ timer/clavier)"
    } else {
        "non chargee"
    }
}

/// Construit et charge l'IDT.
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
        IDT.load();
        READY = true;
    }
    dmesg::log("idt: IDT chargee (exceptions + IRQ)");
}

/// Declenche volontairement une exception breakpoint (commande de test).
pub fn trigger_breakpoint() {
    x86_64::instructions::interrupts::int3();
}

// --- Exceptions CPU ---------------------------------------------------------

extern "x86-interrupt" fn breakpoint_handler(stack: InterruptStackFrame) {
    println!("exception: breakpoint (int3) capturee, on continue");
    serial_println!("[cpu] breakpoint at {:?}", stack.instruction_pointer);
}

extern "x86-interrupt" fn double_fault_handler(stack: InterruptStackFrame, _code: u64) -> ! {
    serial_println!("[cpu] DOUBLE FAULT\n{:#?}", stack);
    panic!("EXCEPTION: double faute\n{:#?}", stack);
}

/// La faute vient-elle du ring 3 ? Dans ce cas c'est le programme utilisateur
/// qu'il faut tuer, surtout pas le noyau.
fn from_user(stack: &InterruptStackFrame) -> bool {
    stack.code_segment & 3 == 3
}

/// Garde qui retablit l'invariant GS pendant un gestionnaire.
///
/// Une interruption venue du ring 3 arrive avec `GS_BASE` cote utilisateur ;
/// le noyau, lui, exige que `GS_BASE` designe la structure par-CPU (voir
/// `arch::x86_64::usermode`). On echange donc a l'entree et on retablit a la
/// sortie — sauf si le gestionnaire ne revient jamais (tache tuee), auquel cas
/// le noyau garde l'etat ring 0 qui lui convient.
struct GsGuard {
    swapped: bool,
}

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

/// Termine la tache utilisateur fautive et rend la main au shell.
///
/// Sans ce filet, la moindre erreur d'un programme (dereferencement nul,
/// instruction illegale) ferait paniquer le noyau : le ring 3 n'aurait alors
/// aucun interet.
fn kill_faulting_task(reason: &str, stack: &InterruptStackFrame) -> ! {
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
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("faute de protection generale", &stack);
    }
    serial_println!("[cpu] general protection fault, code {}", code);
    panic!("EXCEPTION: general protection fault (code {})\n{:#?}", code, stack);
}

extern "x86-interrupt" fn invalid_opcode_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("instruction illegale", &stack);
    }
    panic!("EXCEPTION: instruction illegale\n{:#?}", stack);
}

extern "x86-interrupt" fn divide_error_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("division par zero", &stack);
    }
    panic!("EXCEPTION: division par zero\n{:#?}", stack);
}

extern "x86-interrupt" fn stack_segment_handler(stack: InterruptStackFrame, code: u64) {
    let _gs = GsGuard::enter(&stack);
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("faute de pile", &stack);
    }
    panic!("EXCEPTION: faute de segment de pile (code {})\n{:#?}", code, stack);
}

extern "x86-interrupt" fn page_fault_handler(stack: InterruptStackFrame, code: PageFaultErrorCode) {
    let _gs = GsGuard::enter(&stack);
    let addr = x86_64::registers::control::Cr2::read();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        // Pagination a la demande : la page a-t-elle ete promise ? Si oui, on
        // l'alloue et l'instruction reprend comme si de rien n'etait. C'est ce
        // qui rend `MAP_NORESERVE` utilisable — voir
        // `kernel::task::peuple_a_la_demande`.
        if crate::kernel::task::peuple_a_la_demande(addr.as_u64()) {
            return;
        }
        crate::println!("faute de page utilisateur @ {:#x} ({:?})", addr.as_u64(), code);
        kill_faulting_task("faute de page", &stack);
    }
    serial_println!("[cpu] page fault @ {:?} code {:?}", addr, code);
    panic!("EXCEPTION: page fault @ {:?}\ncode: {:?}\n{:#?}", addr, code, stack);
}

// --- IRQ materielles --------------------------------------------------------


extern "x86-interrupt" fn timer_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    timer::tick();
    crate::kernel::task::echantillonne();
    notify_end_of_interrupt(InterruptIndex::Timer.as_u8());

    crate::kernel::task::watchdog_from_timer();

    if crate::kernel::task::in_user_task() {
        if from_user(&stack) {
            crate::kernel::task::preempt_from_irq();
        } else if !crate::kernel::task::current_is_kernel_task() {
            // IRQ0 a surpris un syscall : preemption differee a sa sortie.
            crate::kernel::task::request_deferred_preempt();
        }
    }
}


extern "x86-interrupt" fn keyboard_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let status = unsafe { ports::inb(0x64) };
    let data = unsafe { ports::inb(0x60) };
    if status & 0x20 == 0 {
        keyboard::push_scancode(data);
    } else {
        mouse::handle_byte(data);
    }
    notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());

    if from_user(&stack) && crate::kernel::task::in_user_task() {
        crate::kernel::task::preempt_from_irq();
    }
}

/// Acquitte une IRQ du controleur ATA.
///
/// Le pilote disque lit par interrogation et coupe ces lignes : ce
/// gestionnaire ne sert qu'a absorber une interruption egaree — le controleur
/// en emet parfois une au tout premier acces, avant que `nIEN` soit pose.
extern "x86-interrupt" fn ata_primary_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _ = unsafe { ports::inb(0x1F7) }; // lire le statut acquitte le controleur
    notify_end_of_interrupt(InterruptIndex::AtaPrimary.as_u8());
}

extern "x86-interrupt" fn ata_secondary_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _ = unsafe { ports::inb(0x177) };
    notify_end_of_interrupt(InterruptIndex::AtaSecondary.as_u8());
}

extern "x86-interrupt" fn mouse_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let status = unsafe { ports::inb(0x64) };
    let byte = unsafe { ports::inb(0x60) };
    if status & 0x20 != 0 {
        mouse::handle_byte(byte);
    } else {
        keyboard::push_scancode(byte);
    }
    notify_end_of_interrupt(InterruptIndex::Mouse.as_u8());
}
