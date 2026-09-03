//! Ce que le noyau sait du navigateur, en tant que GROUPE de processus.
//!
//! Ladybird n'est pas un processus : c'est un courtier, un serveur de
//! requetes, un decodeur d'images, et un moteur de rendu par contexte de
//! rendu. Le noyau les suivait comme un seul client graphique -- il n'en
//! connaissait qu'un --, ce qui suffisait tant qu'il n'y en avait qu'un.

pub mod supervision;
