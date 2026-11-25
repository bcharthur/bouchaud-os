//! BOUCHAUD_VFS_MONTAGES_V1 -- premiere tranche du VFS generaliste.
//!
//! # Ce qu'elle remplace
//!
//! `/dev` et `/proc` n'etaient pas des systemes de fichiers : une suite de
//! comparaisons de chaines (`fd::device_for_path`) melee au RAMFS, ou `/proc`
//! etait par ailleurs un INSTANTANE ecrit au demarrage (`/proc/meminfo`
//! annoncait la memoire libre du boot pour toute la session).
//!
//! # Ce qu'elle installe
//!
//! * une interface commune, [`SystemeFichiers`] : ouvrir un chemin relatif a
//!   son point de montage (la liste des repertoires montes viendra avec
//!   `getdents` sur les montages) ;
//! * une TABLE DE MONTAGES, et une resolution generique : le point de montage
//!   le plus long qui prefixe le chemin canonique l'emporte ;
//! * une identite stable des fichiers servis, [`Vnode`] : (systeme, inode) ;
//! * deux systemes montes au demarrage, `devfs` sur `/dev` et `procfs` sur
//!   `/proc` (`kernel::fd`), et le RAMFS sous `/` (son adaptateur complet --
//!   vnodes, descripteurs independants des indices -- est la tranche
//!   suivante ; il reste servi par le chemin historique).
//!
//! # Union, pas masquage
//!
//! Un systeme monte qui ne connait pas un nom rend `None` : la resolution
//! retombe alors sur le RAMFS sous-jacent. Les fichiers que l'instantane de
//! demarrage ecrivait sous `/proc` (`cpuinfo`, `sys/...`) restent donc
//! lisibles, et ceux que `procfs` sert en VIVANT les remplacent. Aucune
//! application existante ne perd un fichier.
//!
//! Module sans dependance a `kernel::fd` : l'ouverture rend un `Ouverture`
//! generique que l'appelant convertit.

use alloc::vec::Vec;

use crate::kernel::sync::SpinLock;

#[path = "chemins.rs"]
mod chemins;
pub use chemins::{inode_de, sous_point};

/// Identite stable d'un fichier servi par un systeme monte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vnode {
    pub systeme: u16,
    pub inode: u64,
}

/// Ce qu'un systeme rend pour un fichier ouvert. `Objet` porte un objet du
/// noyau (peripherique, flux) que la couche descripteurs sait servir ;
/// `Contenu` un instantane genere A L'OUVERTURE (semantique `seq_file`).
pub enum Ouverture<O> {
    Objet(O),
    Contenu(Vec<u8>),
}

/// Un systeme de fichiers montable.
///
/// `relatif` est le chemin sous le point de montage, sans `/` initial ;
/// `""` designe sa racine. `O` est le type d'objet que la couche descripteurs
/// sait ouvrir (`kernel::fd::FdKind`).
pub trait SystemeFichiers<O>: Send + Sync {
    fn nom(&self) -> &'static str;
    /// Ouvre `relatif`, ou `None` si ce systeme ne le connait pas.
    fn ouvre(&self, relatif: &str) -> Option<Ouverture<O>>;
}

/// Un montage : point (canonique, sans `/` final sauf la racine) et systeme.
pub struct Montage<O: 'static> {
    pub point: &'static str,
    pub systeme: &'static dyn SystemeFichiers<O>,
    pub identifiant: u16,
}

/// La table des montages.
pub struct Table<O: 'static> {
    montages: SpinLock<Vec<Montage<O>>>,
}

impl<O: 'static> Table<O> {
    pub const fn vide() -> Self {
        Self { montages: SpinLock::new(Vec::new()) }
    }

    /// Monte `systeme` sur `point`. Un point deja monte est remplace.
    pub fn monte(&self, point: &'static str, systeme: &'static dyn SystemeFichiers<O>) {
        let mut table = self.montages.lock();
        table.retain(|m| m.point != point);
        let identifiant = table.len() as u16 + 1;
        table.push(Montage { point, systeme, identifiant });
    }

    /// Le montage le plus specifique pour `chemin` (canonique) et le chemin
    /// relatif sous lui.
    pub fn resout<'a>(&self, chemin: &'a str) -> Option<(&'static dyn SystemeFichiers<O>, u16, &'a str)> {
        let table = self.montages.lock();
        table
            .iter()
            .filter_map(|m| sous_point(m.point, chemin).map(|reste| (m, reste)))
            .max_by_key(|(m, _)| m.point.len())
            .map(|(m, reste)| (m.systeme, m.identifiant, reste))
    }

    /// Ouvre `chemin` par le systeme monte qui le couvre, et son identite.
    pub fn ouvre(&self, chemin: &str) -> Option<(Ouverture<O>, Vnode)> {
        let (systeme, id, relatif) = self.resout(chemin)?;
        let ouverture = systeme.ouvre(relatif)?;
        Some((ouverture, Vnode { systeme: id, inode: inode_de(relatif) }))
    }

    /// (point, nom du systeme), dans l'ordre de montage : `/proc/mounts`.
    pub fn montages(&self) -> Vec<(&'static str, &'static str)> {
        self.montages.lock().iter().map(|m| (m.point, m.systeme.nom())).collect()
    }
}
