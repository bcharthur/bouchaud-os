//! Couche d'abstraction architecture.
//!
//! Bouchaud OS cible aujourd'hui uniquement x86_64 sous QEMU, mais tout le code
//! dependant du materiel est isole ici pour preparer d'eventuels portages.

pub mod x86_64;
