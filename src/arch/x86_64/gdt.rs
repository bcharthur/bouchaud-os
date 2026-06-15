//! Global Descriptor Table (GDT) — STUB V0.6.
//!
//! En long mode 64 bits le bootloader nous laisse avec une GDT minimale deja
//! fonctionnelle. Ce module prepare le terrain pour charger notre propre GDT
//! (segments noyau/utilisateur + TSS) en V0.7, indispensable au futur split
//! user/kernel et a la gestion propre des exceptions.

use crate::kernel::dmesg;

/// Etat courant de la GDT, expose aux commandes systeme.
pub fn state() -> &'static str {
    "stub (heritee du bootloader, GDT maison planifiee V0.7)"
}

/// Point d'initialisation appele au boot. Aucune table n'est encore chargee.
pub fn init() {
    dmesg::log("gdt: stub initialise (GDT du bootloader conservee)");
}
