//! Peripheriques bloc et occupation des systemes de fichiers.
//!
//! Le stockage se limite pour l'instant a la **lecture** : le pilote ATA
//! (`drivers::ata`) sert a charger une archive userland au demarrage
//! (`fs::tar`). Il n'y a pas encore d'ecriture persistante — un vrai systeme de
//! fichiers (BFS) reste a la feuille de route.

use crate::drivers::ata::{self, Drive, SECTOR_SIZE};

/// Un disque de donnees est-il present ?
pub fn present() -> bool {
    ata::present(Drive::Slave)
}

/// Affiche l'occupation des systemes de fichiers (commande `df`).
pub fn print_df() {
    use crate::fs::{ramfs, tar};
    let fs = ramfs::fs();
    let (master, slave) = ata::capacities();

    crate::println!("Sys. fichiers   Type     Inodes       Etat");
    crate::println!(
        "ramfs /         RAMFS    {:>4}/{:<5}  monte (volatil)",
        fs.used_nodes(),
        ramfs::MAX_NODES
    );

    crate::println!("");
    crate::println!("Peripheriques bloc:");
    for (drive, sectors) in [(Drive::Master, master), (Drive::Slave, slave)] {
        if sectors == 0 {
            crate::println!("  {:<22} absent", drive.label());
        } else {
            crate::println!(
                "  {:<22} {} secteurs ({} Mio)",
                drive.label(),
                sectors,
                sectors * SECTOR_SIZE as u64 / (1024 * 1024)
            );
        }
    }

    match tar::mounted() {
        Some((files, dirs, bytes)) => {
            crate::println!("");
            crate::println!(
                "Archive userland indexee depuis hdb : {} fichiers, {} repertoires, {} Kio logiques",
                files,
                dirs,
                bytes / 1024
            );
            let (lazy_files, lazy_bytes, read_ops, read_bytes) = crate::fs::backing::stats();
            crate::println!(
                "  backing paresseux : {} fichiers, {} Kio ; lectures={} ({} Kio)",
                lazy_files,
                lazy_bytes / 1024,
                read_ops,
                read_bytes / 1024
            );
        }
        None => {
            crate::println!("");
            crate::println!("Aucune archive userland (voir tools/userland/mkdisk.sh)");
        }
    }
    crate::println!("ecriture persistante : a venir (BFS)");
}
