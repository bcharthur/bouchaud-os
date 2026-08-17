//! Couche bloc generique.
//!
//! Le noyau ne doit pas faire connaitre ATA au VFS ni aux futurs systemes de
//! fichiers. Cette abstraction est volontairement petite : un backend expose
//! des blocs fixes, le cache/pages/filesystem se construisent au-dessus.
//!
//! Pour l'instant `AtaBlockDevice` est l'unique backend. `virtio-blk` pourra
//! implementer le meme contrat sans modifier le code de fichier/memoire.

use crate::drivers::ata::{self, Drive, SECTOR_SIZE};

pub const BLOCK_SIZE: usize = SECTOR_SIZE;

pub trait BlockDevice {
    fn block_size(&self) -> usize;
    fn block_count(&self) -> u64;
    fn read_blocks(&self, lba: u64, count: usize, out: &mut [u8]) -> usize;
    fn write_blocks(&self, lba: u64, count: usize, data: &[u8]) -> usize;
}

#[derive(Clone, Copy)]
pub struct AtaBlockDevice {
    drive: Drive,
}

impl AtaBlockDevice {
    pub const fn new(drive: Drive) -> Self {
        Self { drive }
    }

    pub const fn drive(&self) -> Drive {
        self.drive
    }
}

impl BlockDevice for AtaBlockDevice {
    fn block_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn block_count(&self) -> u64 {
        let (master, slave) = ata::capacities();
        match self.drive {
            Drive::Master => master,
            Drive::Slave => slave,
        }
    }

    fn read_blocks(&self, lba: u64, count: usize, out: &mut [u8]) -> usize {
        ata::read(self.drive, lba, count, out)
    }

    fn write_blocks(&self, lba: u64, count: usize, data: &[u8]) -> usize {
        ata::write(self.drive, lba, count, data)
    }
}

pub fn read_blocks(drive: Drive, lba: u64, count: usize, out: &mut [u8]) -> usize {
    AtaBlockDevice::new(drive).read_blocks(lba, count, out)
}

pub fn write_blocks(drive: Drive, lba: u64, count: usize, data: &[u8]) -> usize {
    AtaBlockDevice::new(drive).write_blocks(lba, count, data)
}
