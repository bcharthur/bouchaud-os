//! Interface graphique de Bouchaud OS — gestionnaire de fenetres.
//!
//! Structure (couches) :
//!   - `framebuffer` : primitives de dessin (au-dessus du pilote d'affichage) ;
//!   - `event` / `mouse` : entrees clavier / souris ;
//!   - `window` : fenetres et types partages ;
//!   - `widgets` : rendu (fenetres, barre des taches, menu, curseur, icones) ;
//!   - `window_manager` : boucle d'evenements (focus, z-order, drag, resize) ;
//!   - `desktop` : point d'entree ;
//!   - `apps/` : applications natives (terminal, fichiers, moniteur,
//!     calculatrice, editeur).
//!
//! Le navigateur ne fait plus partie du noyau : il vit en ring 3
//! (`tools/userland/navigateur/`) et s'affiche par Qt sur `/dev/fb0`.

pub mod apps;
pub mod client;
pub mod desktop;
pub mod event;
pub mod framebuffer;
pub mod mouse;
pub mod widgets;
pub mod window;
pub mod window_manager;

pub mod font;


pub use desktop::run;
