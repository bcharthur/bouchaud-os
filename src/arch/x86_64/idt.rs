// BOUCHAUD_SMP4_DEADLOCK_FIX
//! IDT partagee en contenu, chargee separement sur chaque CPU.

use x86_64::structures::idt::{
    InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
};
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

pub fn load_ap() {
    unsafe { IDT.load(); }
}

pub fn trigger_breakpoint() {
    x86_64::instructions::interrupts::int3();
}


// BOUCHAUD_DEEP_FRAGMENTATION_V11A
// Exceptions, fautes et TLB restent dans le même module Rust `idt`,
// mais vivent désormais dans un fragment dédié.
include!("idt/exceptions.rs");

// BOUCHAUD_PREEMPT_IRQ_V8
//
// Hard-IRQ scheduling is physically separated from the rest of the IDT while
// remaining in this exact Rust module. No public path changes.
include!("idt/preemption.rs");
include!("idt/timer.rs");
include!("idt/reschedule.rs");

// IRQ périphériques fragmentées par responsabilité.
include!("idt/keyboard.rs");
include!("idt/storage.rs");
include!("idt/mouse.rs");
