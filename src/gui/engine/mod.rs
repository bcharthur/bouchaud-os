//! Moteur de plateforme web de Bouchaud OS (couche rendu/execution), distinct
//! du gestionnaire de fenetres :
//!   - `web`   : HTML -> DOM -> CSS (subset) -> layout -> liste d'affichage ;
//!   - `js`    : interpreteur JavaScript proche d'un navigateur moderne : DOM,
//!               modele d'evenements reel (addEventListener + dispatch au clic),
//!               boucle d'evenements (setTimeout/setInterval, Promise/microtaches,
//!               queueMicrotask), Date (RTC), styles live (`el.style.*` -> layout),
//!               et API `WebAssembly` (instantiate/validate, branche sur le
//!               runtime wasmi via `crate::wasm`) ;
//!   - `image` : decodage et downscale d'images (PNG, data:URI).
pub mod web;
pub mod js;
pub mod image;
