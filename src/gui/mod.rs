//! Interface graphique de Bouchaud OS — gestionnaire de fenetres.
//!
//! Structure (couches) :
//!   - `framebuffer` : primitives de dessin (au-dessus du pilote d'affichage) ;
//!   - `event` / `mouse` : entrees clavier / souris ;
//!   - `window` : fenetres et types partages ;
//!   - `widgets` : rendu (fenetres, barre des taches, menu, curseur, icones) ;
//!   - `window_manager` : boucle d'evenements (focus, z-order, drag, resize) ;
//!   - `desktop` : point d'entree ;
//!   - `engine/` : moteur de plateforme web (web, js, image) ;
//!   - `apps/` : applications natives (terminal, fichiers, moniteur, navigateur,
//!     calculatrice).

pub mod apps;
pub mod desktop;
pub mod engine;
pub mod event;
pub mod framebuffer;
pub mod mouse;
pub mod widgets;
pub mod window;
pub mod window_manager;

// Le moteur web/JS vit dans `engine/` ; chemins historiques conserves.
pub use engine::{web, js, image};

pub use desktop::run;
