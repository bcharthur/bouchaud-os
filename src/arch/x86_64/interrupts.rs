//! Controleur d'interruptions PIC 8259 et activation des IRQ materielles.
//!
//! On remappe les deux PIC sur les vecteurs 32..47 (les 0..31 sont reserves aux
//! exceptions CPU), puis on active les interruptions (`sti`). A partir de la, le
//! timer (IRQ0) et le clavier (IRQ1) fonctionnent en mode interruption : le
//! clavier n'est plus interroge en polling.

use pic8259::ChainedPics;
use crate::arch::x86_64::ports::{inb, outb};
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
    Mouse = PIC_1_OFFSET + 12, // IRQ12 (souris PS/2), sur le PIC esclave
    /// IRQ14/15 : controleurs ATA primaire et secondaire.
    ///
    /// Le pilote disque fonctionne par interrogation et coupe ces lignes
    /// (`nIEN`), mais un vecteur non installe transforme la moindre IRQ egaree
    /// en faute de protection generale, puis en double faute. Les enregistrer
    /// pour les acquitter coute deux lignes et evite un arret complet.
    AtaPrimary = PIC_1_OFFSET + 14,
    AtaSecondary = PIC_1_OFFSET + 15,
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


/// Demasque explicitement une IRQ sur les PIC 8259.
///
/// `ChainedPics::initialize()` restaure les masques qu'il a trouves avant le
/// remappage. Cela convient a une machine deja configuree, mais pas comme
/// contrat de pilote : l'IRQ1 peut donc rester masquee meme si le 8042 est
/// parfaitement configure. Les pilotes appellent cette fonction quand leur
/// peripherique est pret a produire des interruptions.
pub fn unmask_irq(irq: u8) {
    unsafe {
        match irq {
            0..=7 => {
                let mask = inb(0x21);
                outb(0x21, mask & !1u8.wrapping_shl(irq as u32));
            }
            8..=15 => {
                // Une IRQ du PIC esclave a aussi besoin de la cascade IRQ2.
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

/// Masques materiels courants (PIC maitre, PIC esclave), pour le diagnostic.
pub fn mask_snapshot() -> (u8, u8) {
    unsafe { (inb(0x21), inb(0xA1)) }
}

/// Signale la fin de traitement d'une IRQ au PIC.
pub fn notify_end_of_interrupt(irq: u8) {
    unsafe { PICS.notify_end_of_interrupt(irq); }
}

/// Initialise et remappe les PIC, puis active les interruptions.
pub fn init() {
    unsafe {
        PICS.initialize();
        // Le timer est une dependance du scheduler : ne pas dependre du masque BIOS.
        unmask_irq(0);
        x86_64::instructions::interrupts::enable();
        ENABLED = true;
    }
    dmesg::log("interrupts: PIC 8259 remappe (32..47), sti actif");
}
