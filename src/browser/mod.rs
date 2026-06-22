//! Nautile — navigateur web souverain de Bouchaud OS.
//!
//! Module autonome : état, chargement, moteur et chrome visuel sont séparés
//! selon le modèle des navigateurs modernes (Firefox / Chrome) :
//!
//!  ┌──────────────┬──────────────────────────────────────────┐
//!  │ state        │ Onglets, historique, session JS           │
//!  │ loader       │ Pipeline réseau → HTML → Session + Page  │
//!  │ pages        │ Pages internes (about:*, calc, wasm…)    │
//!  │ engine/      │ Pont vers gui::engine (web, js, image)   │
//!  │ ui/theme     │ Palette et constantes visuelles          │
//!  │ ui/chrome    │ Dessin du chrome + gestion événements    │
//!  └──────────────┴──────────────────────────────────────────┘
//!
//! Point d'entrée pour `gui::apps::mod` et `gui::window`.

pub mod engine;
pub mod loader;
pub mod pages;
pub mod state;
pub mod ui;

pub use state::BrowserState;
