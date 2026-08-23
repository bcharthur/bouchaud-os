//! PIC 8259 sur le BSP + activation locale des interruptions sur les AP.

use pic8259::ChainedPics;
use crate::arch::x86_64::ports::{inb, outb};
use crate::kernel::dmesg;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

static mut PICS: ChainedPics = unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) };
static mut ENABLED: bool = false;

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
    Mouse = PIC_1_OFFSET + 12,
    AtaPrimary = PIC_1_OFFSET + 14,
    AtaSecondary = PIC_1_OFFSET + 15,
}

impl InterruptIndex {
    pub fn as_u8(self) -> u8 { self as u8 }
    pub fn as_usize(self) -> usize { self as usize }
}

pub fn enabled() -> bool { unsafe { ENABLED } }

pub fn state() -> &'static str {
    if enabled() {
        "enabled (PIC BSP + interruptions locales AP)"
    } else {
        "disabled"
    }
}

pub fn unmask_irq(irq: u8) {
    unsafe {
        match irq {
            0..=7 => {
                let mask = inb(0x21);
                outb(0x21, mask & !1u8.wrapping_shl(irq as u32));
            }
            8..=15 => {
                let master = inb(0x21);
                outb(0x21, master & !1u8.wrapping_shl(2));
                let slave_irq = irq - 8;
                let slave = inb(0xA1);
                outb(0xA1, slave & !1u8.wrapping_shl(slave_irq as u32));
            }
            _ => {}
        }
    }
}

pub fn mask_snapshot() -> (u8, u8) {
    unsafe { (inb(0x21), inb(0xA1)) }
}

pub fn notify_end_of_interrupt(irq: u8) {
    unsafe { PICS.notify_end_of_interrupt(irq); }
}

/// BSP : remappe le PIC et demasque le PIT.
pub fn init() {
    unsafe {
        PICS.initialize();
        unmask_irq(0);
        x86_64::instructions::interrupts::enable();
        ENABLED = true;
    }
    dmesg::log("interrupts: PIC BSP remappe (32..47), sti actif");
}

/// AP : pas de PIC a reprogrammer. L'IDT et le LAPIC sont deja locaux au CPU.
pub fn enable_ap() {
    x86_64::instructions::interrupts::enable();
}
