//! Systeme de fichiers de Bouchaud OS.
//!
//! Pour l'instant un unique backend : `ramfs`, un FS en memoire a inodes fixes
//! (aucune allocation dynamique). La feuille de route disque (block device,
//! virtio-blk, BFS persistant) est documentee dans `docs/ROADMAP.md`.

pub mod ramfs;
