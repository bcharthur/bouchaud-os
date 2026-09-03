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
pub mod chaine;
pub mod degats;
pub mod disposition;
pub mod transition;
pub mod protocole;

// BOUCHAUD_C12_ECHELLE_V1
//
// L'ECHELLE D'AFFICHAGE, ET POURQUOI ELLE EST UNE VARIABLE
//
// Le compositeur noyau presente aujourd'hui un pixel logique pour un pixel
// physique, et il n'y a pas d'ecran dense a servir. L'echelle est donc l'unite.
//
// Ce qui change, c'est qu'elle est desormais NOMMEE et TRANSMISE. Un client ne
// peut pas distinguer « echelle unite » de « le compositeur ne dit rien » si le
// compositeur ne dit rien : les deux se lisent comme un champ absent. En
// l'annoncant explicitement, le jour ou elle vaudra 180 ne demandera qu'un
// changement de valeur -- et les clients qui la lisent deja continueront de
// fonctionner.
static ECHELLE_AFFICHAGE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(protocole::ECHELLE_UNITE);

/// L'echelle annoncee aux clients, en cent-vingtiemes (120 = 1,0).
pub fn echelle_affichage() -> u32 {
    protocole::echelle_valide(ECHELLE_AFFICHAGE.load(core::sync::atomic::Ordering::Relaxed))
}

/// Change l'echelle d'affichage. Rend celle qui etait en vigueur.
///
/// Une valeur hors bornes est repliee sur l'unite plutot que refusee : un
/// compositeur qui recevrait une echelle absurde doit continuer a afficher.
pub fn pose_echelle_affichage(echelle: u32) -> u32 {
    let ancienne = echelle_affichage();
    ECHELLE_AFFICHAGE.store(
        protocole::echelle_valide(echelle),
        core::sync::atomic::Ordering::Relaxed,
    );
    ancienne
}
pub mod surface;
pub mod desktop;
pub mod desktop_bkl;
pub mod event;

// V15: FPS utile + frame-gap, atomique et sans verrou.
pub mod frame_clock;

// V9: wrappers de compatibilite. Les anciens fichiers restent intacts dans le
// depot et sont inclus comme "legacy" par les wrappers.
#[path = "framebuffer_v9.rs"]
pub mod framebuffer;

pub mod mouse;
pub mod polices;
pub mod politique;

#[path = "reveil_v9.rs"]
pub mod reveil;

pub mod scene;
pub mod silence;

// V15 superpose les FPS au rendu historique sans remplacer le gros widgets.rs.
#[path = "widgets_v15.rs"]
pub mod widgets;

pub mod window;
pub mod window_manager;
pub mod windowing;
pub mod theme;
pub mod graphics;

pub mod texte;
pub mod png;
pub mod icones;
pub mod font;


pub use desktop::run;
