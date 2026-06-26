//! Interface graphique de Bouchaud OS — gestionnaire de fenetres.
//!
//! Structure (couches) :
//!   - `framebuffer` : primitives de dessin (au-dessus du pilote d'affichage) ;
//!   - `event` / `mouse` : entrees clavier / souris ;
//!   - `window` : fenetres et types partages ;
//!   - `widgets` : rendu (fenetres, barre des taches, menu, curseur, icones) ;
//!   - `window_manager` : boucle d'evenements (focus, z-order, drag, resize) ;
//!   - `desktop` : point d'entree ;
//!   - `apps/` : applications natives (terminal, fichiers, moniteur, navigateur,
//!     calculatrice).
//!
//! Le moteur de rendu web vit desormais dans `crate::browser::engine` (code du
//! navigateur Nautile). Il reste accessible ici via des re-exports historiques.

pub mod apps;
pub mod desktop;
pub mod event;
pub mod framebuffer;
pub mod mouse;
pub mod widgets;
pub mod window;
pub mod window_manager;

// Le moteur web/JS vit desormais dans `crate::browser::engine` (code du
// navigateur Nautile, suivi par le systeme de version `build.rs`). On conserve
// les chemins historiques `crate::gui::engine` / `gui::web` / `gui::js` /
// `gui::image` via des re-exports pour ne pas toucher les nombreux appelants.
pub use crate::browser::engine::{self as engine, image, js, web};

pub use desktop::run;
