//! Sous-système HTML de Nautile — un fichier par préoccupation.
//!
//!   - `tags` : taxonomie des balises (void, bloc, inline, sectioning, métadonnée,
//!              tableau, formulaire, média, interactif) + valeurs par défaut.
//!
//! Le tokenizer et la construction du DOM restent dans `web.rs` (couplés au
//! pipeline) ; ce module centralise la connaissance « par balise » pour que
//! le rendu puisse traiter chaque famille correctement, façon HTML5.

pub mod tags;
