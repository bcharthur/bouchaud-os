//! Backend AArch64 de Bouchaud OS.
//!
//! Première cible d'exécution : QEMU `virt`.
//! Première carte physique : Raspberry Pi 4 / BCM2711.
//! Aucun faux backend n'est fourni : une primitive n'est ajoutée que lorsqu'un
//! test de bring-up l'accompagne.

pub mod context;
pub mod cpu;
pub mod exceptions;
pub mod interrupts;
pub mod mmu;
pub mod smp;
pub mod usermode;
