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
//!     calculatrice, rustpad).
//!
//! Le rasterizer de police (`gui::font`) est un service du systeme : le pilote
//! graphique s'en sert pour le texte proportionnel.

pub mod apps;
pub mod desktop;
pub mod event;
pub mod framebuffer;
pub mod mouse;
pub mod widgets;
pub mod window;
pub mod window_manager;

// Rasterizer de police : service du systeme, utilise par le pilote graphique
// pour le texte proportionnel. C'est la seule piece de l'ancien moteur de rendu
// que l'OS conserve — le reste vivait dans le navigateur, desormais dans son
// propre depot.
pub mod font;

pub use desktop::run;
