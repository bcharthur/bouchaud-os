//! Façade d'abstraction de l'architecture CPU.
//!
//! Le code générique doit progressivement utiliser cette couche et ne plus
//! importer directement `arch::x86_64` ou `arch::aarch64`.

pub mod api;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "x86_64")]
pub use x86_64 as current;

#[cfg(target_arch = "aarch64")]
pub use aarch64 as current;
