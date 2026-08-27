//! Abstraction de la machine autour du CPU.
//!
//! `arch` décrit l'ISA ; `platform` décrit ACPI/Device Tree, topologie, routage
//! d'interruptions et assemblage de la machine.

pub mod pc;
pub mod qemu_virt;
pub mod raspberry_pi;
