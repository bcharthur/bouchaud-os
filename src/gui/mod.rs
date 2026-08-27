//! Interface graphique de Bouchaud OS — gestionnaire de fenetres.
//!
//! Structure (couches) :
//!   - `framebuffer` : primitives de dessin (au-dessus du pilote d'affichage) ;
//!   - `event` / `mouse` : entrees clavier / souris ;
//!   - `window` : fenetres et types partages ;
//!   - `widgets` : rendu (fenetres, barre des taches, menu, curseur, icones) ;
//!   - `politique` : quand composer, quand dormir -- arithmetique pure ;
//!   - `reveil` : comptabilite du reveil evenementiel du compositeur ;
//!   - `scene` : quels calques dessiner pour un rectangle -- geometrie pure ;
//!   - `silence` : ce client annonce-t-il ses trames, ou faut-il deviner ;
//!   - `window_manager` : boucle d'evenements (focus, z-order, drag, resize) ;
//!   - `desktop` : point d'entree ;
//!   - `apps/` : applications natives (terminal, fichiers, moniteur,
//!     calculatrice, editeur).
//!
//! Le navigateur ne fait plus partie du noyau : il vit en ring 3
//! (`tools/userland/navigateur/`). Il n'ecrit plus pour autant a l'ecran : son
//! `/dev/fb0` est redirige vers une surface partagee que le gestionnaire de
//! fenetres compose dans une fenetre ordinaire.
//!
//!   - `protocole` : format de fil du protocole GUI userland ;
//!   - `surface` : memoire de pixels partagee avec un client ring 3 ;
//!   - `client` : session (processus, surface, canal) d'un client.

pub mod apps;
pub mod client;
pub mod degats;
pub mod protocole;
pub mod surface;
pub mod desktop;
pub mod event;
pub mod framebuffer;
pub mod mouse;
pub mod polices;
pub mod politique;
pub mod reveil;
pub mod scene;
pub mod silence;
pub mod widgets;
pub mod window;
pub mod window_manager;

pub mod font;


pub use desktop::run;
