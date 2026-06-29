//! Moteur de rendu web de Nautile — cœur du navigateur, intégré nativement
//! dans Bouchaud OS et suivi par le système de version (`build.rs` trace
//! `src/browser`). C'est ici que vit toute la logique de rendu :
//!
//!   - `web`      : HTML → DOM → CSS (cascade, sélecteurs descendants) → layout
//!                  (flux blocs/inline, flexbox, CSS grid, box model) → liste
//!                  d'affichage truecolor ;
//!   - `js`       : interpréteur JavaScript (DOM, événements, timers, Promise,
//!                  WebAssembly via wasmi) ;
//!   - `image`    : décodage + downscale d'images (PNG, JPEG baseline, data:URI) ;
//!   - `font_ttf` : rasterizer de police vectorielle TrueType (antialiasé).
//!
//! Pipeline inspiré des moteurs modernes (Gecko/Firefox, WebKit/Blink) :
//!   HTML → tokenizer → DOM → style (cascade CSS) → layout → paint.

pub mod web;
pub mod style;
pub mod css_values;
pub mod css_parser;
pub mod display_list;
pub mod paint;
pub mod js;
pub mod image;
pub mod svg;
pub mod font_ttf;
