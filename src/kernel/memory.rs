//! Gestion memoire de haut niveau.
//!
//! Aujourd'hui : tas statique (voir `kernel::heap`). A terme : lecture de la
//! memory map du bootloader, allocateur de frames physiques, pagination par
//! processus et memoire virtuelle.

use crate::kernel::heap;

/// Octets de tas utilises.
pub fn used() -> usize { heap::stats().0 }
/// Octets de tas libres.
pub fn free() -> usize { heap::stats().1 }
/// Taille totale du tas.
pub fn total() -> usize { heap::stats().2 }

/// Affiche un resume memoire (commande `free`).
pub fn print_info() {
    let (u, f, t) = heap::stats();
    crate::println!("Memoire (tas noyau):");
    crate::println!("  total : {} o", t);
    crate::println!("  utilise: {} o", u);
    crate::println!("  libre : {} o", f);
    crate::println!("pagination par processus: planifiee (memoire virtuelle a venir)");
}
