//! Moteur de rendu web de Nautile.
//!
//! Pont vers `gui::engine` — la logique réside dans les sous-modules du moteur
//! (`web`, `js`, `image`). Ce module expose une API stable côté browser sans
//! dépendre directement de la hiérarchie `gui::`.
//!
//! Architecture inspirée du pipeline Gecko (Firefox) :
//!   HTML → tokenizer → DOM → style (cascade CSS) → layout → paint
//! et de Blink (Chrome) pour la séparation contenu / rendu.

// Re-exports stables de l'API moteur (publics pour les consommateurs du crate).
#[allow(unused_imports)]
pub use crate::gui::engine::web;
#[allow(unused_imports)]
pub use crate::gui::engine::js;
#[allow(unused_imports)]
pub use crate::gui::engine::image;
