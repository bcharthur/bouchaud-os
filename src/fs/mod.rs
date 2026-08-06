//! Systeme de fichiers de Bouchaud OS.
//!
//! Le systeme vivant est `ramfs`, en memoire. Ce qui doit survivre a
//! l'extinction passe par `persistance`, qui deplie une zone du disque de
//! donnees dans `/persist` au demarrage et l'y reecrit sur `sync`.
//!
//! La suite — pilote de bloc generique, virtio-blk, systeme de fichiers a
//! blocs — est documentee dans `docs/ROADMAP.md`.

pub mod persistance;
pub mod ramfs;
pub mod tar;
