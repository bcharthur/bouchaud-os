//! Interface historique de diagnostic interne (QUATRE commandes).
//!
//! CE MODULE N'EST PAS L'ABI RING3 EN PRODUCTION. La vraie transition
//! ring0/ring3 (syscall/sysret) est dans arch/x86_64/usermode.rs ;
//! les appels natifs versions sont dans kernel/native/abi/ et la
//! compatibilite Linux/POSIX dans kernel/abi/. Ne pas deduire l'absence
//! d'isolation de la seule taille de ce fichier.

use crate::kernel::{scheduler, timer};

/// Numeros d'appels systeme.
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Sys {
    GetPid = 1,
    Uptime = 2,
    Ticks = 3,
    WriteSerial = 4,
}

/// Point d'entree unique. `arg` est l'argument generique (selon l'appel).
/// Renvoie une valeur entiere (resultat ou code d'erreur negatif).
pub fn dispatch(call: Sys, arg: usize) -> isize {
    match call {
        Sys::GetPid => scheduler::current() as isize,
        Sys::Uptime => timer::seconds() as isize,
        Sys::Ticks => timer::ticks() as isize,
        Sys::WriteSerial => {
            // arg = octet a ecrire sur la sortie serie de debug.
            crate::serial_print!("{}", (arg as u8) as char);
            0
        }
    }
}

/// Liste les appels systeme disponibles (commande `syscalls`).
pub fn print_table() {
    crate::println!("Appels historiques de diagnostic interne (pas ABI ring3):");
    crate::println!("  1 getpid       -> pid de la tache courante");
    crate::println!("  2 uptime       -> secondes depuis le boot");
    crate::println!("  3 ticks        -> ticks timer");
    crate::println!("  4 write_serial -> ecrit un octet sur COM1");
    crate::println!("ABI native ring3: kernel/native/abi/ ; route syscall: arch/x86_64/usermode.rs");
}
