//! Contrat de boot indépendant du chargeur.
//!
//! Le chemin x86 actuel utilise encore `bootloader::BootInfo`. La prochaine
//! étape convertira ces données en `BootInfo` Bouchaud avant l'entrée générique.

pub mod info;
pub use info::*;
