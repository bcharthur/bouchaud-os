//! Interrupt Descriptor Table (IDT) — STUB V0.6.
//!
//! La table d'interruptions n'est pas encore installee. En V0.7 ce module
//! enregistrera les handlers d'exceptions CPU (breakpoint, double fault, page
//! fault...) puis les IRQ materielles (timer PIT, clavier PS/2).

use crate::kernel::dmesg;

/// Etat courant de l'IDT, expose aux commandes systeme.
pub fn state() -> &'static str {
    "stub (aucune IDT chargee, handlers planifies V0.7)"
}

/// Point d'initialisation appele au boot. Aucun handler n'est encore enregistre.
pub fn init() {
    dmesg::log("idt: stub initialise (aucun handler enregistre)");
}
