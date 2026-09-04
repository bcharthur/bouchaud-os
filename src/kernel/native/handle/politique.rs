//! Les decisions de securite de la table de handles, isolees pour etre
//! prouvables.
//!
//! # Pourquoi les sortir de la table
//!
//! `table.rs` melange deux choses de natures tres differentes : la gestion des
//! emplacements -- un `Vec`, un verrou, des generations -- et les REGLES qui
//! decident si un acces est permis. Les premieres ne se testent qu'avec un
//! noyau ; les secondes sont de l'arithmetique de bits, et ce sont elles qui
//! font la difference entre une isolation et une decoration.
//!
//! Separees, elles se mettent a l'epreuve sur l'hote, une par une, avec les cas
//! NEGATIFS -- ce qui doit etre refuse -- au meme rang que les cas positifs.
//!
//! # Ce que ce module a corrige en naissant
//!
//! `HandleTable::export` portait ceci :
//!
//!     entry.rights = entry.rights.intersection(entry.rights);
//!
//! avec, au-dessus, un commentaire expliquant qu'aucun droit n'est jamais gagne
//! par IPC. L'intersection d'un ensemble avec lui-meme est cet ensemble : la
//! ligne ne faisait RIEN. Le commentaire decrivait une intention, pas le code.
//!
//! Ce n'etait pas seulement inutile : il n'existait aucun moyen d'ATTENUER les
//! droits d'un handle en le transferant. Un courtier qui possede une region
//! partagee en lecture-ecriture ne pouvait pas en donner une vue en lecture
//! seule a un moteur de rendu -- il donnait tout, ou rien. La regle porte
//! desormais un masque, et le test le verifie dans les deux sens.
//!
//! # L'invariant
//!
//! Aucune de ces regles ne peut AJOUTER un droit. Toutes sont des
//! intersections. C'est ce qui rend la delegation sure : un handle derive n'est
//! jamais plus puissant que celui dont il vient, quelle que soit la profondeur
//! de la chaine.

use super::super::abi::types::{Error, ObjectKind, Result, Rights};

include!("politique_corps.rs");
