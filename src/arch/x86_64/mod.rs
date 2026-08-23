//! Implementation x86_64 : ports d'E/S, CPU, et briques GDT/IDT/interruptions.
//!
//! Les modules `gdt`, `idt` et `interrupts` sont pour l'instant des stubs propres.
//! Ils sont appeles au boot et exposent leur etat aux commandes systeme afin de
//! preparer l'activation reelle des interruptions en V0.7.

pub mod ports;
pub mod cpu;
pub mod cpu_local; // BOUCHAUD_SMP_NG1_CPU_FOUNDATION
pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod pci;
pub mod rtc;
pub mod smp;
pub mod usermode;

/// Initialise les briques bas niveau de l'architecture au boot.
///
/// L'ordre compte : la GDT doit etre chargee avant que `usermode` ne programme
/// les MSR de `syscall`, qui referencent ses selecteurs.
pub fn init() {
    gdt::init();
    idt::init();
    interrupts::init();
    usermode::init();
    pci::init();
}
