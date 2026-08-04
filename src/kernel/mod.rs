//! Coeur du noyau Bouchaud OS : journalisation, temps, et gestion des paniques.
//!
//! - `dmesg` : tampon circulaire des evenements noyau (commande `dmesg`) ;
//! - `timer` : compteur de ticks et mesure de temps (`uptime`, `ticks`) ;
//! - `panic` : handler de panique noyau.
//!
//! S'y ajoute la chaine qui mene a l'execution d'un programme Linux natif :
//! `vmm` (frames physiques et espaces d'adressage), `elf` (chargeur ELF64),
//! `fd` (descripteurs et peripheriques), `task` (threads et futex), `abi`
//! (appels systeme POSIX) et `exec` (assemblage de tout cela).

pub mod abi;
pub mod dmesg;
pub mod elf;
pub mod exec;
pub mod fd;
pub mod heap;
pub mod memory;
pub mod handle;
pub mod process;
pub mod scheduler;
pub mod syscall;
pub mod task;
pub mod timer;
pub mod vmm;
pub mod panic;
