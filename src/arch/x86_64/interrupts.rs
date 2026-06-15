//! Controleur d'interruptions PIC 8259 et activation des IRQ materielles.
//!
//! On remappe les deux PIC sur les vecteurs 32..47 (les 0..31 sont reserves aux
//! exceptions CPU), puis on active les interruptions (`sti`). A partir de la, le
//! timer (IRQ0) et le clavier (IRQ1) fonctionnent en mode interruption : le
//! clavier n'est plus interroge en polling.

use pic8259::ChainedPics;
use crate::kernel::dmesg;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

static mut PICS: ChainedPics = unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) };
static mut ENABLED: bool = false;

/// Vecteurs d'interruption materielle utilises.
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl InterruptIndex {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn as_usize(self) -> usize {
        self as usize
    }
}

/// Indique si les interruptions materielles sont activees.
pub fn enabled() -> bool {
    unsafe { ENABLED }
}

/// Etat courant des interruptions, expose aux commandes systeme.
pub fn state() -> &'static str {
    if enabled() {
        "enabled (PIC remap 32..47, IRQ timer + clavier actives)"
    } else {
        "disabled"
    }
}

/// Signale la fin de traitement d'une IRQ au PIC.
pub fn notify_end_of_interrupt(irq: u8) {
    unsafe { PICS.notify_end_of_interrupt(irq); }
}

/// Initialise et remappe les PIC, puis active les interruptions.
pub fn init() {
    unsafe {
        PICS.initialize();
        x86_64::instructions::interrupts::enable();
        ENABLED = true;
    }
    dmesg::log("interrupts: PIC 8259 remappe (32..47), sti actif");
}
