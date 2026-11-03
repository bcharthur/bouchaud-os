//! Atelier -- le design system de Bouchaud OS Experience 2.0.
//!
//! # Ce que ce module est
//!
//! La SOURCE UNIQUE de l'apparence du bureau : couleurs, matieres, rayons,
//! espacements, typographie, elevation et mouvement. Les peintres
//! (`widgets.rs`) demandent « une matiere de verre de niveau 1 », pas
//! « 0xffffff a 72 % » : la meme valeur ne peut donc pas diverger entre la
//! barre du haut, le dock et les fenetres.
//!
//! # Comment il est coupe
//!
//!   * [`jetons`]    valeurs pures (couleurs, rayons, espaces, corps, duree) ;
//!   * [`mouvement`] courbes d'acceleration et interpolation, pures ;
//!   * [`dock`]      geometrie pure du dock flottant ;
//!   * [`fond`]      fond d'ecran calcule une fois, puis recopie ;
//!   * [`matiere`]   peintres de matieres (verre, ombre douce, fenetre).
//!
//! Les trois premiers ne connaissent ni le framebuffer ni l'allocateur : les
//! tests d'hote les incluent tels quels (`tools/gui/test_atelier.rs`).
//!
//! # Ce qu'il ne fait pas
//!
//! Il ne touche ni a l'ordonnanceur, ni a la mise a jour A/B, ni a la
//! securite, ni a la cadence du compositeur : un bureau plus beau ne doit pas
//! couter un cycle de plus quand rien ne bouge. Le fond est calcule une fois ;
//! une trame ou seule l'horloge change ne recompose que la barre du haut.

pub mod jetons;
pub mod mouvement;
pub mod dock;
pub mod fond;
pub mod matiere;
