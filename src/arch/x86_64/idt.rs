//! Interrupt Descriptor Table (IDT) : exceptions CPU + IRQ materielles.
//!
//! On enregistre les exceptions essentielles (breakpoint, double faute, faute de
//! page, faute de protection generale) et les deux IRQ utiles : le timer (IRQ0)
//! qui incremente l'horloge noyau, et le clavier (IRQ1) qui empile les scancodes
//! pour l'editeur de ligne.

use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use crate::arch::x86_64::{gdt, ports};
use crate::arch::x86_64::interrupts::{notify_end_of_interrupt, InterruptIndex};
use crate::drivers::keyboard;
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
        IDT[InterruptIndex::Timer.as_usize()].set_handler_fn(timer_interrupt_handler);
        IDT[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);
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

extern "x86-interrupt" fn general_protection_handler(stack: InterruptStackFrame, code: u64) {
    serial_println!("[cpu] general protection fault, code {}", code);
    panic!("EXCEPTION: general protection fault (code {})\n{:#?}", code, stack);
}

extern "x86-interrupt" fn page_fault_handler(stack: InterruptStackFrame, code: PageFaultErrorCode) {
    let addr = x86_64::registers::control::Cr2::read();
    serial_println!("[cpu] page fault @ {:?} code {:?}", addr, code);
    panic!("EXCEPTION: page fault @ {:?}\ncode: {:?}\n{:#?}", addr, code, stack);
}

// --- IRQ materielles --------------------------------------------------------

extern "x86-interrupt" fn timer_interrupt_handler(_stack: InterruptStackFrame) {
    timer::tick();
    notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack: InterruptStackFrame) {
    let scancode = unsafe { ports::inb(0x60) };
    keyboard::push_scancode(scancode);
    notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
}
