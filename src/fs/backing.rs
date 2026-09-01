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
use crate::kernel::sync::SpinLock;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
struct DiskExtent {
    node: usize,
    drive: Drive,
    data_lba: u64,
    size: usize,
    generation: u64,
}

static EXTENTS: SpinLock<Vec<DiskExtent>> = SpinLock::new(Vec::new());
static DISK_READ_OPS: AtomicU64 = AtomicU64::new(0);
static DISK_READ_BYTES: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static READAHEAD_HITS: AtomicU64 = AtomicU64::new(0);
// V14: amortise ATA/TCG overhead aggressively but keep a hard memory bound.
const READAHEAD_MIN: usize = 64 * 1024;
const READAHEAD_MID: usize = 128 * 1024;
const READAHEAD_MAX: usize = 256 * 1024;
const CACHE_ENTRIES_MAX: usize = 512;

struct ReadCacheEntry {
    node: usize,
    base: usize,
    valid: usize,
    data: Vec<u8>,
    prefetched_from: usize,
}
static READ_CACHE: SpinLock<Vec<ReadCacheEntry>> = SpinLock::new(Vec::new());
struct ReadPattern { node: usize, last_end: usize, sequential: u8 }
static READ_PATTERNS: SpinLock<Vec<ReadPattern>> = SpinLock::new(Vec::new());
static READAHEAD_PAGES: AtomicU64 = AtomicU64::new(0);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static RAMFS_BKL_ENTERS: AtomicU64 = AtomicU64::new(0);

pub fn reset() {
    EXTENTS.lock().clear();
    READ_CACHE.lock().clear();
    READ_PATTERNS.lock().clear();
    DISK_READ_OPS.store(0, Ordering::Relaxed);
    DISK_READ_BYTES.store(0, Ordering::Relaxed);
    CACHE_HITS.store(0, Ordering::Relaxed);
    READAHEAD_HITS.store(0, Ordering::Relaxed);
    READAHEAD_PAGES.store(0, Ordering::Relaxed);
}

pub fn register_disk(node: usize, drive: Drive, data_lba: u64, size: usize) {
    unregister(node);
    EXTENTS.lock().push(DiskExtent {
        node,
        drive,
        data_lba,
        size,
        generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
    });
}

pub fn unregister(node: usize) {
    EXTENTS.lock().retain(|extent| extent.node != node);
    READ_CACHE.lock().retain(|entry| entry.node != node);
    READ_PATTERNS.lock().retain(|entry| entry.node != node);
}

pub fn is_disk_backed(node: usize) -> bool {
    disk_len(node).is_some()
}

pub fn disk_len(node: usize) -> Option<usize> {
    EXTENTS.lock()
        .iter()
        .find(|extent| extent.node == node)
        .map(|extent| extent.size)
}

pub fn generation(node: usize) -> Option<u64> {
    EXTENTS.lock()
        .iter()
        .find(|extent| extent.node == node)
        .map(|extent| extent.generation)
}

pub fn logical_len(node: usize) -> usize {
    if let Some(size) = disk_len(node) {
        return size;
    }
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Fs);
    let _kernel = crate::kernel::smp_lock::enter();
    RAMFS_BKL_ENTERS.fetch_add(1, Ordering::Relaxed);
    crate::fs::ramfs::fs().nodes[node].content.len()
}

/// Lit une plage sans materialiser le fichier complet.
fn read_at_uncached(node: usize, offset: usize, out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }

    // Copier la metadonnee sous le verrou, puis le rendre avant toute decision.
    //
    // Ce `let` compte : ecrit en `match EXTENTS.lock()...`, le garde temporaire
    // du sujet du match vivait jusqu'a la fin de la construction — donc aussi
    // dans la branche « pas d'etendue », qui prend le BKL. `EXTENTS` etait
    // alors tenu pendant une attente du BKL, tandis qu'un autre cœur tenant
    // deja le BKL demandait `EXTENTS` par `disk_len`. Les deux s'attendaient
    // pour toujours. En `let`, le garde tombe au point-virgule, avant meme que
    // l'on sache s'il y a une etendue.
    let extent = EXTENTS.lock().iter().find(|extent| extent.node == node).copied();

    let Some(extent) = extent else {
        let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Fs);
        let _kernel = crate::kernel::smp_lock::enter();
        RAMFS_BKL_ENTERS.fetch_add(1, Ordering::Relaxed);
        let fs = crate::fs::ramfs::fs();
        let content = &fs.nodes[node].content;
        if offset >= content.len() {
            return 0;
        }
        let len = core::cmp::min(out.len(), content.len() - offset);
        out[..len].copy_from_slice(&content[offset..offset + len]);
        return len;
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
            DISK_READ_OPS.fetch_add(1, Ordering::Relaxed);
            DISK_READ_BYTES.fetch_add(done as u64, Ordering::Relaxed);
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

    DISK_READ_OPS.fetch_add(1, Ordering::Relaxed);
    DISK_READ_BYTES.fetch_add(done as u64, Ordering::Relaxed);
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
    if out.is_empty() || !is_disk_backed(node) || out.len() > READAHEAD_MAX {
        return read_at_uncached(node, offset, out);
    }
    {
        let cache = READ_CACHE.lock();
        if let Some(entry) = cache.iter().find(|entry| {
            entry.node == node && offset >= entry.base
                && offset.saturating_add(out.len()) <= entry.base.saturating_add(entry.valid)
        }) {
            let start = offset - entry.base;
            out.copy_from_slice(&entry.data[start..start + out.len()]);
            CACHE_HITS.fetch_add(1, Ordering::Relaxed);
            if offset >= entry.prefetched_from {
                READAHEAD_HITS.fetch_add(1, Ordering::Relaxed);
            }
            return out.len();
        }
    }

    let (window, sequentiel) = {
        let mut patterns = READ_PATTERNS.lock();
        let pattern = if let Some(pattern) = patterns.iter_mut().find(|p| p.node == node) {
            pattern
        } else {
            patterns.push(ReadPattern { node, last_end: 0, sequential: 0 });
            patterns.last_mut().unwrap()
        };
        pattern.sequential = if offset == pattern.last_end {
            pattern.sequential.saturating_add(1)
        } else { 0 };
        pattern.last_end = offset.saturating_add(out.len());
        let fenetre = match pattern.sequential {
            0 | 1 => READAHEAD_MIN,
            2 | 3 => READAHEAD_MID,
            _ => READAHEAD_MAX,
        };
        (fenetre, pattern.sequential >= 2)
    };
    // BOUCHAUD_DISQUE_ANTICIPATION_AVANT_V1
    //
    // La fenetre etait TOUJOURS alignee vers le bas. Sur un flux de fautes
    // sequentiel -- ce que produit le chargement d'un ELF ou d'une bibliotheque
    // --, cela relit jusqu'a `window - 4096` octets DERRIERE la demande : des
    // octets que le lecteur vient de depasser et ne redemandera pas.
    //
    // Des que le motif est reconnu sequentiel, la fenetre part donc de la
    // demande elle-meme. Le cache reste correct : ses entrees sont trouvees par
    // CONTENANCE d'intervalle, pas par alignement.
    //
    // L'alignement est garde pour un acces isole : sans motif, on ne sait pas
    // de quel cote le suivant tombera, et l'alignement est le pari neutre.
    let base = if sequentiel { offset } else { offset & !(window - 1) };
    let mut data = alloc::vec![0u8; window];
    let valid = read_at_uncached(node, base, &mut data);
    if valid == 0 || offset < base || offset - base >= valid {
        return 0;
    }
    let start = offset - base;
    let copied = core::cmp::min(out.len(), valid - start);
    out[..copied].copy_from_slice(&data[start..start + copied]);
    READAHEAD_PAGES.fetch_add(
        valid.saturating_sub(copied).div_ceil(crate::kernel::vmm::PAGE_SIZE as usize) as u64,
        Ordering::Relaxed,
    );
    // Publication is short; the global cache lock is never held during I/O.
    let mut cache = READ_CACHE.lock();
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
    (
        CACHE_HITS.load(Ordering::Relaxed),
        READAHEAD_HITS.load(Ordering::Relaxed),
    )
}

pub fn readahead_pages() -> u64 {
    READAHEAD_PAGES.load(Ordering::Relaxed)
}

pub fn ramfs_bkl_enters() -> u64 {
    RAMFS_BKL_ENTERS.load(Ordering::Relaxed)
}

/// (fichiers paresseux, octets logiques, operations disque, octets lus).
pub fn stats() -> (usize, u64, u64, u64) {
    let extents = EXTENTS.lock();
    let files = extents.len();
    let logical = extents.iter().map(|extent| extent.size as u64).sum();
    (
        files,
        logical,
        DISK_READ_OPS.load(Ordering::Relaxed),
        DISK_READ_BYTES.load(Ordering::Relaxed),
    )
}
