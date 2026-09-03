//! Une zone inscriptible sur le disque de donnees.
//!
//! Le RAMFS oublie tout a l'extinction. C'est sans consequence pour un binaire
//! deplie depuis l'archive — il y sera encore au prochain demarrage — mais cela
//! rend impossible tout ce qu'un systeme est cense retenir : les temoins de
//! connexion d'un navigateur, son cache, ses reglages, un fichier telecharge.
//!
//! ## Ou l'ecriture a lieu
//!
//! Le disque de donnees porte une archive `tar` **au debut**, lue une fois au
//! demarrage. La zone persistante occupe les derniers secteurs du meme disque :
//! les deux ne se rencontrent jamais tant que l'image est plus grande que
//! l'archive, ce dont `mkdisk.sh` se charge en la completant.
//!
//! Se passer d'un troisieme disque n'est pas qu'une economie : le noyau ne
//! choisit pas combien de disques QEMU lui presente, et une image de plus
//! serait un fichier de plus a oublier d'attacher.
//!
//! ## Le format
//!
//! ```text
//! secteur 0            en-tete : magie, version, nombre d'entrees
//! secteurs 1 a 1024    table : 2048 entrees de 256 octets (chemin, taille)
//! secteurs 1025 a ...  contenu, chaque fichier aligne sur un secteur
//! ```
//!
//! La zone entiere occupe `SECTEURS_ZONE` secteurs, soit 128 Mio.
//!
//! Aucune allocation de blocs, aucune table d'inodes : la zone est reecrite en
//! entier a chaque `sync`. C'est ce qui convient a quelques mega-octets ecrits
//! rarement, et cela supprime la moitie des facons de corrompre un systeme de
//! fichiers. Un vrai systeme de fichiers viendra quand le besoin sera d'ecrire
//! souvent et beaucoup — ce n'est pas celui du navigateur.
//!
//! ## Le format V2 : deux demi-zones, deux superblocs
//!
//! Le format V1 ci-dessus ecrivait le contenu, puis la table, puis l'en-tete.
//! L'ordre etait le bon, et il ne suffisait pas -- pour une raison qui n'a rien
//! a voir avec l'ordre. L'en-tete vit a un secteur FIXE : l'ecrire ecrase le
//! precedent. Le contenu vit aux memes secteurs d'une synchronisation a
//! l'autre : le reecrire ecrase le precedent. Une coupure pendant l'ecriture du
//! contenu laissait donc l'ANCIEN en-tete -- valide, magie correcte -- pointant
//! vers un contenu a moitie neuf, et ce melange etait monte comme coherent.
//!
//! ```text
//! secteur 0            superbloc A
//! secteur 1            superbloc B
//! secteurs 2 ..        demi-zone 0 : table puis contenu
//! secteurs 2+D ..      demi-zone 1 : table puis contenu
//! ```
//!
//! Une synchronisation ecrit dans la demi-zone INACTIVE, puis commit en
//! ecrivant un seul secteur : le superbloc de l'autre emplacement, portant une
//! generation plus haute et une somme de controle. Le contenu committe n'est
//! jamais touche par l'ecriture en cours ; le point de commit tient dans un
//! secteur ; et meme ce secteur peut etre dechire sans consequence, puisque sa
//! somme de controle le rejette et que l'autre superbloc reste valide.
//!
//! Une coupure laisse donc TOUJOURS soit l'ancien etat, soit le nouveau.
//!
//! ## Ce qui est garanti, et ce qui ne l'est pas
//!
//! Une zone dont aucune magie ne correspond est traitee comme vide : un disque
//! neuf, ou un disque dont l'archive a grandi jusqu'a mordre sur la zone, ne
//! font pas echouer le demarrage. Un disque au format V1 reste LISIBLE -- le
//! montage se replie dessus --, et la premiere synchronisation le fait passer
//! en V2. Ce qui n'est pas garanti : la duree d'une synchronisation, qui reste
//! proportionnelle a ce qui a change.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::drivers::ata::{self, Drive, SECTOR_SIZE};
use crate::fs::ramfs::{fs, NodeKind};
use crate::kernel::sync::{SleepMutex, SpinLock};
use core::sync::atomic::{AtomicU64, Ordering};

/// Reconnait une zone au format V1. Conserve pour la LIRE.
const MAGIE: &[u8; 8] = b"BOPERSI1";

/// Nombre maximal de fichiers retenus.
const ENTREES_MAX: usize = 2048;

/// Taille d'une entree de table, en octets.
const TAILLE_ENTREE: usize = 256;

/// Longueur maximale d'un chemin dans la table.
const CHEMIN_MAX: usize = TAILLE_ENTREE - 16;

/// Secteurs reserves par la table.
///
/// Le format garde 1024 secteurs reserves afin que `SECTEUR_CONTENU` reste
/// strictement compatible avec les disques deja crees. En revanche, un sync
/// n'ecrit plus les 1024 secteurs quand seules quelques entrees sont utilisees.
const SECTEURS_TABLE: u64 = (ENTREES_MAX * TAILLE_ENTREE / SECTOR_SIZE) as u64;

/// Premier secteur du contenu, relatif au debut de la zone.
const SECTEUR_CONTENU: u64 = 1 + SECTEURS_TABLE;

/// Taille de la zone, en secteurs. 128 Mio.
///
/// `tools/userland/mkdisk.sh` ajoute exactement autant de secteurs nuls a la
/// fin de l'image : les deux valeurs doivent bouger ensemble.
const SECTEURS_ZONE: u64 = 262144;

/// Racine des fichiers persistants dans le RAMFS.
pub const RACINE: &str = "/persist";


// BOUCHAUD_DEEP_FRAGMENTATION_V11A
// Façade de persistance. Les fragments sont inclus dans CE module :
// format disque, statiques privées et API publique restent identiques.
include!("persistance/superbloc.rs");
include!("persistance/format.rs");
include!("persistance/transaction.rs");
include!("persistance/arbre.rs");
include!("persistance/index.rs");
include!("persistance/montage.rs");
include!("persistance/snapshot.rs");
include!("persistance/io.rs");
include!("persistance/sync.rs");
include!("persistance/diagnostic.rs");
include!("persistance/collecte.rs");
include!("persistance/codec.rs");
