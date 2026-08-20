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

/// Ladybird est lie statiquement : WebContent pese plusieurs centaines de Mio.
/// Cette limite ne s'applique qu'aux fichiers de l'archive de boot, qui est une
/// image de confiance fabriquee par notre CI. Les ecritures ordinaires du RAMFS
/// restent protegees par `ramfs::MAX_FILE_SIZE` (64 Mio).
const MAX_BOOT_FILE_SIZE: usize = 512 * 1024 * 1024;

/// Garde-fou sur le second disque lu integralement au boot. Le run Ladybird
/// #31966498412 produit une image d'environ 271 Mio ; l'ancienne limite de
/// 192 Mio tronquait donc WebContent avant meme que le TAR atteigne le petit
/// `webcontent-bootstrap` place juste apres. 768 Mio laisse une marge importante
/// tout en bornant clairement l'allocation noyau.
const MAX_ARCHIVE_DISK_SIZE: usize = 768 * 1024 * 1024;

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

/// Depose un fichier de l'archive de boot dans le RAMFS, en creant
/// l'arborescence au besoin.
///
/// On n'utilise volontairement pas `write_node_bytes`: sa limite de 64 Mio
/// protege les ecritures ordinaires et ne doit pas etre relevee uniquement pour
/// Ladybird. L'archive de boot a son propre plafond, plus haut mais toujours
/// borne, et provient exclusivement de notre image ustar de confiance.
fn write_file(path: &str, content: &[u8], mode: u16) -> bool {
    if content.len() > MAX_BOOT_FILE_SIZE {
        return false;
    }

    let (parent_path, name) = match path.rfind('/') {
        Some(index) => (&path[..index], &path[index + 1..]),
        None => ("", path),
    };
    if name.is_empty() {
        return false;
    }
    let parent = if parent_path.is_empty() {
        0
    } else {
        mkdir_path(parent_path)
    };

    let fs = ramfs::fs();
    let node = match fs.find_child(parent, name) {
        Some(existing) => existing,
        None => match fs.touch_at(parent, name) {
            Ok(created) => created,
            Err(_) => return false,
        },
    };
    fs.nodes[node].content = content.to_vec();
    fs.nodes[node].mode = mode;
    true
}

/// Resultat d'un depliage.
pub struct Unpacked {
    pub files: usize,
    pub directories: usize,
    pub bytes: usize,
    /// Entrees refusees (trop grosses, table d'inodes pleine ou type inconnu).
    pub skipped: usize,
    /// Archive tronquee ou incoherente : on s'est arrete avant de lire des
    /// donnees partielles comme si elles formaient un fichier valide.
    pub truncated: bool,
}

/// Deplie une archive `tar` deja en memoire.
pub fn unpack(archive: &[u8]) -> Unpacked {
    let mut result = Unpacked {
        files: 0,
        directories: 0,
        bytes: 0,
        skipped: 0,
        truncated: false,
    };
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

        // Ne jamais donner un fichier partiel au RAMFS. C'etait exactement ce
        // que faisait l'ancien `min(offset + size, archive.len())` quand hdb
        // etait coupe a 192 Mio : WebContent devenait un fragment puis le
        // parseur sautait hors du tampon et n'atteignait plus le bootstrap.
        let end = match offset.checked_add(size) {
            Some(end) if end <= archive.len() => end,
            _ => {
                result.skipped += 1;
                result.truncated = true;
                break;
            }
        };

        match kind {
            TYPE_DIR => {
                if !path.is_empty() && mkdir_path(path) != 0 {
                    result.directories += 1;
                }
            }
            TYPE_FILE | TYPE_FILE_ALT => {
                if !path.is_empty() {
                    if write_file(
                        path,
                        &archive[offset..end],
                        if mode == 0 { 0o644 } else { mode },
                    ) {
                        result.files += 1;
                        result.bytes += size;
                    } else {
                        result.skipped += 1;
                    }
                }
            }
            // Liens, peripheriques, entetes longues GNU : ignores en silence,
            // ils n'ont pas d'equivalent dans le RAMFS.
            _ => result.skipped += 1,
        }

        // Le contenu est complete jusqu'au bloc suivant.
        let padded = match size.checked_add(BLOCK - 1) {
            Some(value) => value / BLOCK * BLOCK,
            None => {
                result.truncated = true;
                break;
            }
        };
        offset = match offset.checked_add(padded) {
            Some(next) => next,
            None => {
                result.truncated = true;
                break;
            }
        };
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
    // lecture complete : inutile d'aspirer des centaines de mega-octets d'un
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
    let max_sectors = (MAX_ARCHIVE_DISK_SIZE / SECTOR_SIZE) as u64;
    if sectors > max_sectors {
        crate::kernel::dmesg::log_fmt(format_args!(
            "tar: hdb trop grand ({} Mio), lecture bornee a {} Mio",
            sectors * SECTOR_SIZE as u64 / (1024 * 1024),
            MAX_ARCHIVE_DISK_SIZE / (1024 * 1024)
        ));
    }
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

/// Seuil sous lequel un fichier de boot reste resident dans le RAMFS.
const INLINE_BOOT_FILE_SIZE: usize = 4 * 1024 * 1024;

/// Indexe directement l'USTAR sur hdb, secteur par secteur.
///
/// Aucun `Vec` de la taille du disque n'est alloue. Les donnees d'un gros
/// fichier ne sont meme pas lues au boot : seule son etendue est enregistree.
fn index_data_disk() -> Option<Unpacked> {
    if !ata::present(Drive::Slave) {
        return None;
    }

    let (_, slave_sectors) = ata::capacities();
    let max_sectors = (MAX_ARCHIVE_DISK_SIZE / SECTOR_SIZE) as u64;
    let disk_sectors = core::cmp::min(slave_sectors, max_sectors);
    if disk_sectors == 0 {
        return None;
    }

    // Un disque plus grand que le plafond ne provoque pas d'erreur ici : la
    // zone persistante occupe justement la fin de hdb, bien au-dela de
    // l'archive. Mais la borne de lecture doit s'annoncer avec ses chiffres
    // exacts, sinon une archive trop lourde perd ses dernieres entrees en
    // silence et le defaut se decouvre a l'execution, sous la forme d'un
    // fichier « absent » que l'image contient pourtant.
    if slave_sectors > max_sectors {
        crate::kernel::dmesg::log_fmt(format_args!(
            "tar: hdb = {} secteurs ({} Mio), indexation bornee a {} secteurs ({} Mio)",
            slave_sectors,
            slave_sectors * SECTOR_SIZE as u64 / (1024 * 1024),
            max_sectors,
            MAX_ARCHIVE_DISK_SIZE / (1024 * 1024)
        ));
    }

    crate::fs::backing::reset();

    let mut result = Unpacked {
        files: 0,
        directories: 0,
        bytes: 0,
        skipped: 0,
        truncated: false,
    };

    let mut sector = 0u64;
    while sector < disk_sectors {
        let mut header = [0u8; SECTOR_SIZE];
        if ata::read(Drive::Slave, sector, 1, &mut header) != 1 {
            result.truncated = true;
            break;
        }

        if header.iter().all(|&byte| byte == 0) {
            break;
        }
        if !is_ustar(&header) {
            if sector == 0 {
                return None;
            }
            break;
        }

        let prefix = field(&header, PREFIX, 155);
        let name = field(&header, NAME, 100);
        let path = if prefix.is_empty() {
            String::from(name)
        } else {
            alloc::format!("{}/{}", prefix, name)
        };
        let path = path.trim_start_matches("./").trim_start_matches('/');

        let size = octal(&header, SIZE, 12) as usize;
        let mode = octal(&header, MODE, 8) as u16 & 0o7777;
        let kind = header[TYPEFLAG];
        let data_lba = sector + 1;
        let data_sectors = ((size + SECTOR_SIZE - 1) / SECTOR_SIZE) as u64;

        if data_lba.saturating_add(data_sectors) > disk_sectors {
            crate::kernel::dmesg::log_fmt(format_args!(
                "tar: ARCHIVE TRONQUEE sur '{}' ({} octets a partir du secteur {}), au-dela du secteur {}",
                path, size, data_lba, disk_sectors
            ));
            result.skipped += 1;
            result.truncated = true;
            break;
        }

        match kind {
            TYPE_DIR => {
                if !path.is_empty() && mkdir_path(path) != 0 {
                    result.directories += 1;
                }
            }
            TYPE_FILE | TYPE_FILE_ALT => {
                if !path.is_empty() {
                    let (parent_path, file_name) = match path.rfind('/') {
                        Some(index) => (&path[..index], &path[index + 1..]),
                        None => ("", path),
                    };
                    let parent = if parent_path.is_empty() {
                        0
                    } else {
                        mkdir_path(parent_path)
                    };

                    let fs = ramfs::fs();
                    let node = match fs.find_child(parent, file_name) {
                        Some(existing) => existing,
                        None => match fs.touch_at(parent, file_name) {
                            Ok(created) => created,
                            Err(_) => {
                                result.skipped += 1;
                                sector = data_lba + data_sectors;
                                continue;
                            }
                        },
                    };
                    fs.nodes[node].mode = if mode == 0 { 0o644 } else { mode };

                    if size <= INLINE_BOOT_FILE_SIZE {
                        let sectors = data_sectors as usize;
                        let mut content = alloc::vec![0u8; sectors * SECTOR_SIZE];
                        if sectors > 0
                            && ata::read(Drive::Slave, data_lba, sectors, &mut content) != sectors
                        {
                            result.skipped += 1;
                            result.truncated = true;
                            break;
                        }
                        content.truncate(size);
                        crate::fs::backing::unregister(node);
                        fs.nodes[node].content = content;
                    } else {
                        fs.nodes[node].content.clear();
                        crate::fs::backing::register_disk(node, Drive::Slave, data_lba, size);
                    }

                    result.files += 1;
                    result.bytes += size;
                }
            }
            _ => result.skipped += 1,
        }

        sector = data_lba + data_sectors;
    }

    Some(result)
}

/// Cherche une archive sur le disque de donnees et la deplie dans le RAMFS.
///
/// Sans archive, le systeme demarre normalement : c'est un enrichissement, pas
/// une dependance.
pub fn mount_data_disk() {
    let result = match index_data_disk() {
        Some(result) => result,
        None => {
            crate::kernel::dmesg::log("tar: aucune archive sur hdb (demarrage sans userland)");
            return;
        }
    };

    unsafe {
        MOUNTED = Some((result.files, result.directories, result.bytes));
    }
    let (lazy_files, lazy_bytes, _, _) = crate::fs::backing::stats();
    crate::kernel::dmesg::log_fmt(format_args!(
        "tar: hdb deplie -> {} fichiers, {} repertoires, {} Kio{}{} ({} fichiers paresseux, {} Kio non copies)",
        result.files,
        result.directories,
        result.bytes / 1024,
        if result.skipped > 0 {
            alloc::format!(" ({} entrees ignorees)", result.skipped)
        } else {
            String::new()
        },
        if result.truncated { " [ARCHIVE TRONQUEE]" } else { "" },
        lazy_files,
        lazy_bytes / 1024
    ));
}
