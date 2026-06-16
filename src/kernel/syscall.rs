//! Couche d'appels systeme (ABI) — socle.
//!
//! Definit l'interface que les futures applications user-mode utiliseront pour
//! demander des services au noyau (au lieu d'acceder directement au materiel).
//! Aujourd'hui le dispatch est appele en interne ; quand le split user/kernel
//! existera, ce point d'entree sera atteint via une interruption (`int 0x80`)
//! ou `syscall`.

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
    crate::println!("Appels systeme (ABI Bouchaud OS):");
    crate::println!("  1 getpid       -> pid de la tache courante");
    crate::println!("  2 uptime       -> secondes depuis le boot");
    crate::println!("  3 ticks        -> ticks timer");
    crate::println!("  4 write_serial -> ecrit un octet sur COM1");
    crate::println!("note: dispatch interne pour l'instant (int 0x80 a venir avec user-mode)");
}
