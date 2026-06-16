//! Types d'evenements d'entree du bureau.
//!
//! Reexporte la touche logique du clavier. La lecture non bloquante se fait via
//! `gui::mouse` (souris) et `drivers::keyboard::try_key` (clavier).

pub use crate::drivers::keyboard::Key;
