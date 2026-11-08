//! Systeme de fichiers de Bouchaud OS.
//!
//! Le systeme vivant est `ramfs`, en memoire. Ce qui doit survivre a
//! l'extinction passe par `persistance`, qui deplie une zone du disque de
//! donnees dans `/persist` au demarrage et l'y reecrit sur `sync`, sur `fsync`
//! d'un fichier persistant, et -- pour tout le reste -- par l'ecriture differee
//! (`ecriture_differee`, au plus quelques secondes apres la modification).
//! Les dossiers personnels (`dossiers`) vivent sous `/persist/home`.
//!
//! La suite — pilote de bloc generique, virtio-blk, systeme de fichiers a
//! blocs — est documentee dans `docs/ROADMAP.md`.

pub mod backing;
pub mod backing_attrib;
pub mod fat32;
pub mod cache_jetable;
pub mod dossiers;
pub mod dossiers_politique;
pub mod ecriture_differee;
pub mod ecriture_differee_politique;
pub mod gpt;
pub mod persistance;
pub mod ramfs;
pub mod renommage;
pub mod vfs_chemins;
pub mod tar;
