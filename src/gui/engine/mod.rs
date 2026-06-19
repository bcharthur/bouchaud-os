//! Moteur de plateforme web de Bouchaud OS (couche rendu/execution), distinct
//! du gestionnaire de fenetres :
//!   - `web`   : HTML -> DOM -> CSS (subset) -> layout -> liste d'affichage ;
//!   - `js`    : interpreteur JavaScript (DOM, evenements, eval d'expressions) ;
//!   - `image` : decodage et downscale d'images (PNG, data:URI).
pub mod web;
pub mod js;
pub mod image;
