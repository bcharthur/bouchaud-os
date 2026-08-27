//! Souris du bureau : le pilote souris PS/2, vu dans les conventions du bureau.
//!
//! Position et boutons passent tels quels. La molette, non : le pilote rend le
//! quatrieme octet du paquet IntelliMouse, qui compte a l'envers du protocole
//! GUI. La conversion est faite ici, une fois, a la frontiere — plutot que dans
//! chacun des consommateurs (navigateur, explorateur de fichiers, Rustpad), ou
//! il aurait suffi d'en oublier un.
//!
//! `kernel::input` ne passe pas par ce module : il publie de l'evdev, dont la
//! convention est encore une troisieme, et il fait sa propre conversion depuis
//! le brut du pilote.

pub use crate::drivers::mouse::{init, left_down, pos};

/// Delta de molette accumule depuis le dernier appel, **dans la convention du
/// protocole GUI** : positif vers le haut.
///
/// Voir [`crate::gui::protocole::molette_depuis_ps2`] pour le pourquoi du signe.
pub fn take_wheel() -> i32 {
    crate::gui::protocole::molette_depuis_ps2(crate::drivers::mouse::take_wheel())
}
