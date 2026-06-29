//! Sous-système CSS de Nautile — un fichier par préoccupation.
//!
//!   - `color`  : couleurs CSS (#hex 3/4/6/8, rgb/rgba, hsl/hsla, ~150 noms,
//!                `transparent`, `currentColor`) → `0x00RRGGBB` ;
//!   - `units`  : longueurs (px, em, rem, %, vw/vh, pt, ch, ex) et nombres.
//!
//! La cascade, le matching de sélecteurs et l'application des déclarations
//! restent pour l'instant dans `web.rs` (couplés au layout) ; ce module
//! héberge la logique réutilisable et purement « valeurs ».

pub mod color;
pub mod units;
