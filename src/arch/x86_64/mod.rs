//! Implementation x86_64 : ports d'E/S, CPU, et briques GDT/IDT/interruptions.
//!
//! Les modules `gdt`, `idt` et `interrupts` sont pour l'instant des stubs propres.
//! Ils sont appeles au boot et exposent leur etat aux commandes systeme afin de
//! preparer l'activation reelle des interruptions en V0.7.

pub mod ports;
pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod interrupts;

/// Initialise les briques bas niveau de l'architecture au boot.
///
/// Pour l'instant ces appels ne font que journaliser leur etat : aucune table
/// n'est encore reellement chargee, mais le point d'entree est en place.
pub fn init() {
    gdt::init();
    idt::init();
    interrupts::init();
}
