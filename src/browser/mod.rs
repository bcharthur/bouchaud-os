//! Nautile — navigateur web souverain de Bouchaud OS.
//!
//! Module autonome : état, chargement, moteur et chrome visuel sont séparés
//! selon le modèle des navigateurs modernes (Firefox / Chrome) :
//!
//!  ┌──────────────┬──────────────────────────────────────────┐
//!  │ state        │ Onglets, historique, session JS           │
//!  │ loader       │ Pipeline réseau → HTML → Session + Page  │
//!  │ pages        │ Pages internes (about:*, calc, wasm…)    │
//!  │ engine/      │ Moteur de rendu (web, js, image, font)   │
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

mod version {
    include!(concat!(env!("OUT_DIR"), "/nautile_version.rs"));
}

pub use state::BrowserState;
pub use version::{
    NAUTILE_MERGE_DATE, NAUTILE_MERGE_SHORT, NAUTILE_MERGE_SUBJECT, NAUTILE_SOURCE_DATE,
    NAUTILE_SOURCE_SHORT,
};
