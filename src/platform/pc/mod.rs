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
pub mod installation;
// H10 : l'outillage de preuve de la machine de reference.
pub mod trigkey;
pub mod physical_diag;
pub mod ecran_faute;
/// Banc de charge d'entree-sortie, derriere son drapeau : voir `banc_io.rs`.
#[cfg(feature = "banc-io")]
pub mod banc_io;
/// Temoin du noyau fige (#283), derriere son drapeau : voir `banc_souffrance.rs`.
#[cfg(feature = "banc-souffrance-verrou")]
pub mod banc_souffrance;
/// BOUCHAUD_CLE_USB_TEST_V1 : sonde d'ecriture du banc, QEMU uniquement.
#[cfg(feature = "banc-ecriture-interne")]
pub mod sonde_ecriture_interne;
