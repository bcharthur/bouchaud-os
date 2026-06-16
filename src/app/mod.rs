//! Systeme d'applications natives Bouchaud OS.
//!
//! Une application est decrite par un manifeste `.bapp` (dans `/apps`). Le
//! `launcher` les enumere et les lance ; `runtime` definit les types d'apps
//! natives connues. Socle du futur format applicatif (apps en espace
//! utilisateur, paquets, permissions).

pub mod manifest;
pub mod runtime;
pub mod launcher;
