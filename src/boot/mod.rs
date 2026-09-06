//! Contrat de boot indépendant du chargeur.
//!
//! Le chemin x86 historique utilise encore `bootloader` 0.9, mais cette
//! dépendance s'arrête à l'adaptateur `legacy_x86`. Le noyau reçoit toujours
//! un `BootInfo` Bouchaud ; le futur chargeur UEFI produira le même contrat.

pub mod info;
pub use info::*;

#[cfg(target_arch = "x86_64")]
mod legacy_x86;

#[cfg(target_arch = "x86_64")]
pub use legacy_x86::from_bootloader_09;
