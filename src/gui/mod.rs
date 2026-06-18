//! Interface graphique de Bouchaud OS — gestionnaire de fenetres.
//!
//! Structure :
//!   - `framebuffer` : primitives de dessin (au-dessus du pilote d'affichage) ;
//!   - `event` / `mouse` : entrees clavier / souris ;
//!   - `window` : fenetres et types partages ;
//!   - `widgets` : rendu (fenetres, barre des taches, menu, curseur) ;
//!   - `window_manager` : boucle d'evenements (focus, z-order, drag, resize) ;
//!   - `desktop` : point d'entree ;
//!   - `apps/` : applications natives (terminal, fichiers, moniteur, navigateur).

pub mod apps;
pub mod desktop;
pub mod event;
pub mod framebuffer;
pub mod mouse;
pub mod web;
pub mod widgets;
pub mod window;
pub mod window_manager;

pub use desktop::run;
