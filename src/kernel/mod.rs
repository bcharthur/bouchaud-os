//! Coeur du noyau Bouchaud OS : journalisation, temps, et gestion des paniques.
//!
//! - `dmesg` : tampon circulaire des evenements noyau (commande `dmesg`) ;
//! - `timer` : compteur de ticks et mesure de temps (`uptime`, `ticks`) ;
//! - `panic` : handler de panique noyau.

pub mod dmesg;
pub mod timer;
pub mod panic;
