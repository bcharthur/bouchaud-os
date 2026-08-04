//! Extinction de la machine.
//!
//! Un noyau qui ne sait pas s'arreter ne peut pas etre teste automatiquement :
//! sans extinction, l'emulateur tourne jusqu'a ce qu'on le tue, et il n'y a
//! aucun code de retour a interpreter. C'est la brique qui manque a
//! [`crate::kernel::autorun`].
//!
//! Aucun des ports utilises ici n'est standard PC ; ce sont des conventions
//! d'emulateurs, essayees dans l'ordre du plus specifique au plus general. Une
//! vraie extinction ACPI demanderait de lire les tables ACPI, d'y trouver le
//! bloc PM1a et la valeur SLP_TYP du systeme — beaucoup de code pour un noyau
//! qui, aujourd'hui, ne tourne que sous emulation.

use crate::arch::x86_64::ports::{outb, outw};

/// Code passe au peripherique `isa-debug-exit` quand tout s'est bien passe.
///
/// QEMU sort avec `(code << 1) | 1`, soit 33 ici : il lui est impossible de
/// rendre 0, puisque 0 signifierait « QEMU s'est arrete tout seul ».
pub const EXIT_OK: u8 = 0x10;
/// Code passe a `isa-debug-exit` en cas d'echec (QEMU sortira avec 35).
pub const EXIT_FAIL: u8 = 0x11;

/// Port du peripherique de test `isa-debug-exit` de QEMU.
const QEMU_DEBUG_EXIT: u16 = 0xF4;
/// Port d'extinction ACPI de QEMU (>= 2.0).
const QEMU_ACPI_SHUTDOWN: u16 = 0x604;
/// Meme role sur les anciennes versions de QEMU et sur Bochs.
const BOCHS_SHUTDOWN: u16 = 0xB004;
/// Meme role sous VirtualBox.
const VBOX_SHUTDOWN: u16 = 0x4004;

/// Arrete la machine, en signalant `code` a l'hote quand c'est possible.
///
/// `code` n'est lu que par le peripherique de test de QEMU : c'est par lui que
/// le lanceur de sondes apprend si le scenario a reussi. Les autres voies
/// eteignent sans rien rapporter.
pub fn shutdown(code: u8) -> ! {
    crate::serial_println!("[kernel] extinction demandee (code {})", code);
    unsafe {
        // Ne repond que si QEMU a ete lance avec `-device isa-debug-exit` ;
        // sinon l'ecriture part dans le vide, ce qui est sans consequence.
        outb(QEMU_DEBUG_EXIT, code);
        outw(QEMU_ACPI_SHUTDOWN, 0x2000);
        outw(BOCHS_SHUTDOWN, 0x2000);
        outw(VBOX_SHUTDOWN, 0x3400);
    }
    halt()
}

/// Arrete le processeur pour de bon.
///
/// Interruptions masquees avant le `hlt` : sans cela, la moindre IRQ reveillerait
/// le processeur et la boucle repartirait, consommant un cœur pour rien.
pub fn halt() -> ! {
    loop {
        x86_64::instructions::interrupts::disable();
        x86_64::instructions::hlt();
    }
}
