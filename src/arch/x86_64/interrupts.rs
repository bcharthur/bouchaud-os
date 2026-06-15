//! Gestion des interruptions — STUB V0.6.
//!
//! Les interruptions materielles restent masquees. Le clavier fonctionne donc
//! encore par polling du port PS/2 (voir `drivers::keyboard`). En V0.7 on
//! programmera le PIC 8259 (ou l'APIC), on activera `sti`, et le clavier ainsi
//! que le timer passeront en mode interruption.

use crate::kernel::dmesg;

/// Indique si les interruptions materielles sont activees.
pub fn enabled() -> bool {
    false
}

/// Etat courant des interruptions, expose aux commandes systeme.
pub fn state() -> &'static str {
    "disabled (stub, PIC/APIC non programme, clavier en polling)"
}

/// Point d'initialisation appele au boot.
pub fn init() {
    dmesg::log("interrupts: stub, IRQ masquees, clavier en polling PS/2");
}
