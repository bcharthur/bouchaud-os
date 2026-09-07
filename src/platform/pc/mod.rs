//! Plateforme PC/QEMU-PC.
//!
//! Backend actuellement exécuté. ACPI, APIC et la découverte de plateforme y
//! seront extraits progressivement du code x86_64.

pub mod bringup;
pub mod stage2;
pub mod reference_gop;
pub mod reference_metrics;
pub mod acpi_probe;
pub mod hardware_probe;
pub mod hardware_facts;
pub mod physical_diag;
