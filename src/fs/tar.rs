//! Archive `tar` depliee dans le RAMFS au demarrage.
//!
//! Repond a une limite tres concrete : jusqu'ici, deposer un programme sur la
//! machine imposait de l'inclure dans le noyau (`include_bytes!`) puis de tout
//! recompiler. Impossible d'y installer quoi que ce soit de volumineux —
//! l'image de boot depasse deja 20 Mio — et impossible d'iterer autrement qu'en
//! reconstruisant l'OS entier.
//!
//! Le principe est celui d'un `initramfs` : un second disque contient une
//! archive `tar`, le noyau la lit au boot et la deplie dans le RAMFS. Le flux de
//! travail devient « je fabrique l'image, je demarre, `exec /mon-programme` ».
//!
//! ## Pourquoi `tar` et pas un systeme de fichiers
//!
//! Un vrai systeme de fichiers (FAT, ext2) demanderait un allocateur de blocs,
//! une table d'inodes, une gestion d'ecriture — pour un besoin qui est ici de
//! lire une fois, en sequence, au demarrage. Le format `tar` se lit d'un bout a
//! l'autre sans index, se fabrique avec l'outil du meme nom present partout, et
//! tient en une centaine de lignes d'analyse. L'ecriture persistante viendra
//! avec un vrai systeme de fichiers ; ce n'est pas ce qui bloquait.

use alloc::string::String;
use alloc::vec::Vec;

use crate::drivers::ata::{self, Drive, SECTOR_SIZE};
use crate::fs::ramfs;

/// Taille d'un en-tete `tar` (et unite de bloc de l'archive).
const BLOCK: usize = 512;

/// Champs d'un en-tete `ustar`, en octets depuis son debut.
const NAME: usize = 0;
const MODE: usize = 100;
const SIZE: usize = 124;
const TYPEFLAG: usize = 156;
const MAGIC: usize = 257;
const PREFIX: usize = 345;

/// Types d'entree que l'on traite.
const TYPE_FILE: u8 = b'0';
const TYPE_FILE_ALT: u8 = b'\0';
const TYPE_DIR: u8 = b'5';

/// Lit un champ numerique octal termine par un espace ou un zero.
fn octal(header: &[u8], offset: usize, len: usize) -> u64 {
    let mut value = 0u64;
    for &byte in &header[offset..offset + len] {
        if byte == 0 || byte == b' ' {
            // Les champs sont completes par des espaces ou des zeros : le
            // premier rencontre termine le nombre.
            if value != 0 {
                break;
            }
            continue;
        }
        if !(b'0'..=b'7').contains(&byte) {
            break;
        }
        value = value * 8 + (byte - b'0') as u64;
    }
    value
}

/// Lit une chaine terminee par zero (ou par la fin du champ).
fn field(header: &[u8], offset: usize, len: usize) -> &str {
    let slice = &header[offset..offset + len];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    core::str::from_utf8(&slice[..end]).unwrap_or("")
}

/// L'en-tete porte-t-il la signature `ustar` ?
fn is_ustar(header: &[u8]) -> bool {
    &header[MAGIC..MAGIC + 5] == b"ustar"
}

/// Cree (ou retrouve) un repertoire par son chemin, en creant les parents.
fn mkdir_path(path: &str) -> usize {
    let fs = ramfs::fs();
    let mut current = 0usize;
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        current = match fs.find_child(current, segment) {
            Some(existing) => existing,
            None => match fs.mkdir_at(current, segment) {
                Ok(created) => created,
                Err(_) => return 0,
            },
        };
    }
    current
}

/// Depose un fichier dans le RAMFS, en creant l'arborescence au besoin.
fn write_file(path: &str, content: &[u8], mode: u16) -> bool {
    let (parent_path, name) = match path.rfind('/') {
        Some(index) => (&path[..index], &path[index + 1..]),
        None => ("", path),
    };
    if name.is_empty() {
        return false;
    }
    let parent = if parent_path.is_empty() { 0 } else { mkdir_path(parent_path) };

    let fs = ramfs::fs();
    let node = match fs.find_child(parent, name) {
        Some(existing) => existing,
        None => match fs.touch_at(parent, name) {
            Ok(created) => created,
            Err(_) => return false,
        },
    };
    if !fs.write_node_bytes(node, content) {
        return false;
    }
    fs.nodes[node].mode = mode;
    true
}

/// Resultat d'un depliage.
pub struct Unpacked {
    pub files: usize,
    pub directories: usize,
    pub bytes: usize,
    /// Entrees refusees (trop grosses, ou table d'inodes pleine).
    pub skipped: usize,
}

/// Deplie une archive `tar` deja en memoire.
pub fn unpack(archive: &[u8]) -> Unpacked {
    let mut result = Unpacked { files: 0, directories: 0, bytes: 0, skipped: 0 };
    let mut offset = 0usize;

    while offset + BLOCK <= archive.len() {
        let header = &archive[offset..offset + BLOCK];
        // Deux blocs nuls consecutifs marquent la fin ; un seul suffit a nous
        // arreter, il n'y a rien d'exploitable apres.
        if header.iter().all(|&byte| byte == 0) {
            break;
        }
        if !is_ustar(header) {
            break;
        }

        let prefix = field(header, PREFIX, 155);
        let name = field(header, NAME, 100);
        let path = if prefix.is_empty() {
            String::from(name)
        } else {
            alloc::format!("{}/{}", prefix, name)
        };
        // Les chemins d'une archive sont relatifs : on les ancre a la racine.
        let path = path.trim_start_matches("./").trim_start_matches('/');

        let size = octal(header, SIZE, 12) as usize;
        let mode = octal(header, MODE, 8) as u16 & 0o7777;
        let kind = header[TYPEFLAG];
        offset += BLOCK;

        match kind {
            TYPE_DIR => {
                if !path.is_empty() && mkdir_path(path) != 0 {
                    result.directories += 1;
                }
            }
            TYPE_FILE | TYPE_FILE_ALT => {
                let end = core::cmp::min(offset + size, archive.len());
                if end > offset && !path.is_empty() {
                    if write_file(path, &archive[offset..end], if mode == 0 { 0o644 } else { mode }) {
                        result.files += 1;
                        result.bytes += size;
                    } else {
                        result.skipped += 1;
                    }
                } else if !path.is_empty() {
                    // Fichier vide : on le cree quand meme.
                    if write_file(path, &[], if mode == 0 { 0o644 } else { mode }) {
                        result.files += 1;
                    }
                }
            }
            // Liens, peripheriques, entetes longues GNU : ignores en silence,
            // ils n'ont pas d'equivalent dans le RAMFS.
            _ => result.skipped += 1,
        }

        // Le contenu est complete jusqu'au bloc suivant.
        offset += (size + BLOCK - 1) / BLOCK * BLOCK;
    }
    result
}

/// Lit le disque de donnees et y cherche une archive.
///
/// Renvoie `None` si le disque est absent ou ne commence pas par un en-tete
/// `ustar` — cas normal quand on demarre sans image userland.
fn read_archive(drive: Drive) -> Option<Vec<u8>> {
    if !ata::present(drive) {
        return None;
    }

    // On lit d'abord un secteur pour verifier la signature avant d'engager la
    // lecture complete : inutile d'aspirer des dizaines de mega-octets d'un
    // disque qui ne contient pas ce qu'on cherche.
    let mut probe = alloc::vec![0u8; SECTOR_SIZE];
    if ata::read(drive, 0, 1, &mut probe) != 1 {
        return None;
    }
    if !is_ustar(&probe) {
        return None;
    }

    let (_, slave_sectors) = ata::capacities();
    let sectors = match drive {
        Drive::Slave => slave_sectors,
        Drive::Master => return None,
    };
    // Garde-fou : on ne lit pas plus que ce que le tas peut absorber.
    let max_sectors = (192 * 1024 * 1024 / SECTOR_SIZE) as u64;
    let to_read = core::cmp::min(sectors, max_sectors) as usize;

    let mut data = alloc::vec![0u8; to_read * SECTOR_SIZE];
    let read = ata::read(drive, 0, to_read, &mut data);
    if read == 0 {
        return None;
    }
    data.truncate(read * SECTOR_SIZE);
    Some(data)
}

/// Etat du dernier montage, expose aux commandes systeme.
static mut MOUNTED: Option<(usize, usize, usize)> = None;

/// A-t-on deplie une archive au demarrage ?
pub fn mounted() -> Option<(usize, usize, usize)> {
    unsafe { MOUNTED }
}

/// Cherche une archive sur le disque de donnees et la deplie dans le RAMFS.
///
/// Sans archive, le systeme demarre normalement : c'est un enrichissement, pas
/// une dependance.
pub fn mount_data_disk() {
    let archive = match read_archive(Drive::Slave) {
        Some(archive) => archive,
        None => {
            crate::kernel::dmesg::log("tar: aucune archive sur hdb (demarrage sans userland)");
            return;
        }
    };

    let result = unpack(&archive);
    unsafe { MOUNTED = Some((result.files, result.directories, result.bytes)) };
    crate::kernel::dmesg::log_fmt(format_args!(
        "tar: hdb deplie -> {} fichiers, {} repertoires, {} Kio{}",
        result.files,
        result.directories,
        result.bytes / 1024,
        if result.skipped > 0 {
            alloc::format!(" ({} entrees ignorees)", result.skipped)
        } else {
            String::new()
        }
    ));
}
