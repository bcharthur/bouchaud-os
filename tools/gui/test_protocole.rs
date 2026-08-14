//! Harnais de test hote pour le protocole GUI userland.
//!
//! Le noyau se compile pour `x86_64-bouchaud_os`, sans `std` et sans harnais de
//! test : `cargo test` n'a rien a executer sur cette cible. Or ce qui casse en
//! silence dans un compositeur n'est pas le noyau, c'est l'arithmetique — un
//! rectangle de degat rogne d'un pixel de trop, une coordonnee convertie du
//! mauvais cote d'une bordure. Ce sont des fonctions pures ; il suffit donc de
//! les compiler pour l'hote et de les exercer.
//!
//! `src/gui/protocole.rs` est inclus tel quel, sans copie ni adaptation : le
//! code teste est exactement celui qui tourne sur la machine. Il n'utilise que
//! `core` et `alloc`, tous deux presents dans la bibliotheque standard de
//! l'hote — d'ou le `extern crate alloc` ci-dessous, et rien de plus.
//!
//! Lance par `tools/gui/test-protocole.sh`.

// Le harnais n'exerce pas tout le module : les constantes que seul le noyau
// emploie ne sont pas mortes pour autant.
#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/gui/protocole.rs"]
mod protocole;

// Les cas de test vivent dans `protocole.rs`, au contact du code qu'ils
// decrivent. Ce fichier n'est que la porte d'entree du harnais.
