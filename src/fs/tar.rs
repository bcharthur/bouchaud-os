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

use crate::drivers::ata::{self, Drive, SECTOR_SIZE};
use crate::fs::ramfs;

/// Taille d'un en-tete `tar` (et unite de bloc de l'archive).
const BLOCK: usize = 512;

/// Ladybird est lie statiquement : WebContent pese plusieurs centaines de Mio.
/// Cette limite ne s'applique qu'aux fichiers de l'archive de boot, qui est une
/// image de confiance fabriquee par notre CI. Les ecritures ordinaires du RAMFS
/// restent protegees par `ramfs::MAX_FILE_SIZE` (64 Mio).
const MAX_BOOT_FILE_SIZE: usize = 512 * 1024 * 1024;

/// Jusqu'ou l'indexation du second disque a le droit d'aller.
///
/// Cette borne a ete ecrite pour l'ancien chemin **glouton**, qui allouait un
/// `Vec` de la taille du disque entier avant de l'analyser : la limiter, c'etait
/// limiter une allocation noyau. Ce chemin n'existe plus. `index_data_disk` lit
/// un secteur d'en-tete par entree TAR, saute par-dessus les donnees, et ne
/// copie en RAM que les fichiers de moins de `INLINE_BOOT_FILE_SIZE`. Son cout
/// est proportionnel au NOMBRE d'entrees, pas a la taille du disque.
///
/// Le plafond reel est ailleurs : `drivers::ata::read` n'implemente que LBA28
/// et s'arrete a 2^28 secteurs, soit 128 Gio. La borne ci-dessous reste donc un
/// garde-fou contre un disque aberrant, pas une contrainte de ressources.
///
/// 768 Mio ne suffisaient plus : depouillees de leur DWARF, les sept runtimes
/// Ladybird pesent encore 1038 Mio (run 32420097987), parce que WebContent,
/// WebWorker, Compositor, WebDriver et le BrowserHost embarquent chacun tout
/// LibWeb/LibJS/Skia par edition de liens statique. 2 Gio couvre cette charge
/// avec de la marge sans rien couter au demarrage.
const MAX_ARCHIVE_DISK_SIZE: usize = 2048 * 1024 * 1024;

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
    let mut fs = ramfs::fs();
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

    let mut fs = ramfs::fs();
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

                    let mut fs = ramfs::fs();
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
