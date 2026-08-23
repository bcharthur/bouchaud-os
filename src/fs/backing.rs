//! Backing de fichiers : contenu resident ou etendue immutable sur disque.
//!
//! Etape de migration entre le RAMFS historique et un VFS complet.
//!
//! Le namespace reste encore celui du RAMFS, mais un gros fichier provenant de
//! l'archive de boot n'est plus copie dans `Node::content`. Le node porte son
//! nom, ses permissions et son identite ; ce registre indique ou lire ses
//! octets sur le disque. Les lecteurs utilisent `read_at`, donc ils ne savent
//! plus si les donnees sont residentes ou file-backed.

use crate::drivers::ata::{Drive, SECTOR_SIZE};
use crate::drivers::block;
use alloc::vec::Vec;

#[derive(Clone, Copy)]
struct DiskExtent {
    node: usize,
    drive: Drive,
    data_lba: u64,
    size: usize,
}

static mut EXTENTS: Option<Vec<DiskExtent>> = None;
static mut DISK_READ_OPS: u64 = 0;
static mut DISK_READ_BYTES: u64 = 0;
static mut CACHE_HITS: u64 = 0;
static mut READAHEAD_HITS: u64 = 0;
const READAHEAD_BYTES: usize = 16 * 1024;
const CACHE_ENTRIES_MAX: usize = 256;

struct ReadCacheEntry {
    node: usize,
    base: usize,
    valid: usize,
    data: Vec<u8>,
    prefetched_from: usize,
}
static mut READ_CACHE: Option<Vec<ReadCacheEntry>> = None;

fn read_cache() -> &'static mut Vec<ReadCacheEntry> {
    unsafe { (&mut *core::ptr::addr_of_mut!(READ_CACHE)).get_or_insert_with(Vec::new) }
}

fn extents() -> &'static mut Vec<DiskExtent> {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(EXTENTS);
        slot.get_or_insert_with(Vec::new)
    }
}

pub fn reset() {
    unsafe {
        EXTENTS = Some(Vec::new());
        DISK_READ_OPS = 0;
        DISK_READ_BYTES = 0;
        CACHE_HITS = 0;
        READAHEAD_HITS = 0;
        READ_CACHE = Some(Vec::new());
    }
}

pub fn register_disk(node: usize, drive: Drive, data_lba: u64, size: usize) {
    unregister(node);
    extents().push(DiskExtent {
        node,
        drive,
        data_lba,
        size,
    });
}

pub fn unregister(node: usize) {
    extents().retain(|extent| extent.node != node);
    read_cache().retain(|entry| entry.node != node);
}

pub fn is_disk_backed(node: usize) -> bool {
    disk_len(node).is_some()
}

pub fn disk_len(node: usize) -> Option<usize> {
    extents()
        .iter()
        .find(|extent| extent.node == node)
        .map(|extent| extent.size)
}

pub fn logical_len(node: usize) -> usize {
    if let Some(size) = disk_len(node) {
        return size;
    }
    crate::fs::ramfs::fs().nodes[node].content.len()
}

/// Lit une plage sans materialiser le fichier complet.
fn read_at_uncached(node: usize, offset: usize, out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }

    let extent = match extents().iter().find(|extent| extent.node == node).copied() {
        Some(extent) => extent,
        None => {
            let fs = crate::fs::ramfs::fs();
            let content = &fs.nodes[node].content;
            if offset >= content.len() {
                return 0;
            }
            let len = core::cmp::min(out.len(), content.len() - offset);
            out[..len].copy_from_slice(&content[offset..offset + len]);
            return len;
        }
    };

    if offset >= extent.size {
        return 0;
    }

    let wanted = core::cmp::min(out.len(), extent.size - offset);
    let mut done = 0usize;
    let mut absolute = offset;

    let intra = absolute % SECTOR_SIZE;
    if intra != 0 && done < wanted {
        let mut sector = [0u8; SECTOR_SIZE];
        let lba = extent.data_lba + (absolute / SECTOR_SIZE) as u64;
        if block::read_blocks(extent.drive, lba, 1, &mut sector) != 1 {
            return done;
        }
        let take = core::cmp::min(SECTOR_SIZE - intra, wanted - done);
        out[done..done + take].copy_from_slice(&sector[intra..intra + take]);
        done += take;
        absolute += take;
    }

    let full_sectors = (wanted - done) / SECTOR_SIZE;
    if full_sectors > 0 {
        let bytes = full_sectors * SECTOR_SIZE;
        let lba = extent.data_lba + (absolute / SECTOR_SIZE) as u64;
        let read = block::read_blocks(
            extent.drive,
            lba,
            full_sectors,
            &mut out[done..done + bytes],
        );
        let got = read * SECTOR_SIZE;
        done += got;
        absolute += got;
        if read != full_sectors {
            unsafe {
                DISK_READ_OPS = DISK_READ_OPS.saturating_add(1);
                DISK_READ_BYTES = DISK_READ_BYTES.saturating_add(done as u64);
            }
            return done;
        }
    }

    if done < wanted {
        let mut sector = [0u8; SECTOR_SIZE];
        let lba = extent.data_lba + (absolute / SECTOR_SIZE) as u64;
        if block::read_blocks(extent.drive, lba, 1, &mut sector) == 1 {
            let take = wanted - done;
            out[done..done + take].copy_from_slice(&sector[..take]);
            done += take;
        }
    }

    unsafe {
        DISK_READ_OPS = DISK_READ_OPS.saturating_add(1);
        DISK_READ_BYTES = DISK_READ_BYTES.saturating_add(done as u64);
    }
    done
}

/// Lit via une fenêtre read-ahead partagée entre processus.
///
/// Les faults ELF sont typiquement des lectures de 4 KiB consécutives. Une
/// fenêtre alignée de 16 KiB transforme quatre faults en une commande backing,
/// sans précharger un binaire entier. Le cache est borné à 4 MiB et partagé par
/// identité de nœud; les processus Ladybird relisant les mêmes pages propres
/// réutilisent donc les octets déjà lus.
pub fn read_at(node: usize, offset: usize, out: &mut [u8]) -> usize {
    if out.is_empty() || !is_disk_backed(node) || out.len() > READAHEAD_BYTES {
        return read_at_uncached(node, offset, out);
    }
    if let Some(entry) = read_cache().iter().find(|entry| {
        entry.node == node && offset >= entry.base
            && offset.saturating_add(out.len()) <= entry.base.saturating_add(entry.valid)
    }) {
        let start = offset - entry.base;
        out.copy_from_slice(&entry.data[start..start + out.len()]);
        unsafe {
            CACHE_HITS = CACHE_HITS.saturating_add(1);
            if offset >= entry.prefetched_from { READAHEAD_HITS = READAHEAD_HITS.saturating_add(1); }
        }
        return out.len();
    }

    let base = offset & !(READAHEAD_BYTES - 1);
    let mut data = alloc::vec![0u8; READAHEAD_BYTES];
    let valid = read_at_uncached(node, base, &mut data);
    if valid == 0 || offset < base || offset - base >= valid {
        return 0;
    }
    let start = offset - base;
    let copied = core::cmp::min(out.len(), valid - start);
    out[..copied].copy_from_slice(&data[start..start + copied]);
    let cache = read_cache();
    if cache.len() >= CACHE_ENTRIES_MAX { cache.remove(0); }
    cache.push(ReadCacheEntry {
        node,
        base,
        valid,
        data,
        prefetched_from: offset.saturating_add(copied),
    });
    copied
}

/// (hits cache, hits read-ahead).
pub fn cache_stats() -> (u64, u64) {
    unsafe { (CACHE_HITS, READAHEAD_HITS) }
}

/// (fichiers paresseux, octets logiques, operations disque, octets lus).
pub fn stats() -> (usize, u64, u64, u64) {
    let files = extents().len();
    let logical = extents().iter().map(|extent| extent.size as u64).sum();
    unsafe { (files, logical, DISK_READ_OPS, DISK_READ_BYTES) }
}
