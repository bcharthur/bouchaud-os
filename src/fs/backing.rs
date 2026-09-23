//! Backing de fichiers : contenu resident ou etendue immutable disque/RAM UEFI.
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
enum BackingSource {
    Disk { drive: Drive, data_lba: u64 },
    /// Adresse VIRTUELLE stable d'un fichier dans le ramdisk UEFI.
    Memory { address: u64 },
}

#[derive(Clone, Copy)]
struct BackingExtent {
    node: usize,
    source: BackingSource,
    size: usize,
    generation: u64,
}

static EXTENTS: SpinLock<Vec<BackingExtent>> = SpinLock::new(Vec::new());
static DISK_READ_OPS: AtomicU64 = AtomicU64::new(0);
static DISK_READ_BYTES: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_C49_COMBIEN_DE_TEMPS_LE_DISQUE
//
// `reads` et `bytes` disent COMBIEN, jamais COMBIEN DE TEMPS. Or la question
// posee est : les huit secondes de fautes fichier du premier WebWorker sont-
// elles des lectures de disque, ou de l'attente ?
//
// La duree d'une faute, telle que `Note` la mesure, inclut deja son attente --
// le commentaire de `faute_memoire.rs` le dit. Elle ne peut donc pas trancher
// seule. Ces deux compteurs-ci ne mesurent QUE le chemin disque reel : la
// branche `BackingSource::Memory` sort avant eux, parce qu'un memcpy depuis le
// ramdisk n'est pas une lecture.
static DISK_READ_NS: AtomicU64 = AtomicU64::new(0);
static DISK_READ_WORST_NS: AtomicU64 = AtomicU64::new(0);

// Le chemin MEMOIRE est compte a part. Un memcpy depuis le ramdisk UEFI n'est
// pas une lecture de disque, et les additionner rendrait le total illisible
// exactement la ou il doit trancher. Sur la Trigkey c'est CE chemin-ci que
// prennent les gros ELF.
static MEM_READ_OPS: AtomicU64 = AtomicU64::new(0);
static MEM_READ_BYTES: AtomicU64 = AtomicU64::new(0);
static MEM_READ_NS: AtomicU64 = AtomicU64::new(0);
static MEM_READ_WORST_NS: AtomicU64 = AtomicU64::new(0);
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

pub fn reset() {
    EXTENTS.lock().clear();
    READ_CACHE.lock().clear();
    READ_PATTERNS.lock().clear();
    DISK_READ_OPS.store(0, Ordering::Relaxed);
    DISK_READ_BYTES.store(0, Ordering::Relaxed);
    DISK_READ_NS.store(0, Ordering::Relaxed);
    DISK_READ_WORST_NS.store(0, Ordering::Relaxed);
    MEM_READ_OPS.store(0, Ordering::Relaxed);
    MEM_READ_BYTES.store(0, Ordering::Relaxed);
    MEM_READ_NS.store(0, Ordering::Relaxed);
    MEM_READ_WORST_NS.store(0, Ordering::Relaxed);
    CACHE_HITS.store(0, Ordering::Relaxed);
    READAHEAD_HITS.store(0, Ordering::Relaxed);
    READAHEAD_PAGES.store(0, Ordering::Relaxed);
}

pub fn register_disk(node: usize, drive: Drive, data_lba: u64, size: usize) {
    unregister(node);
    EXTENTS.lock().push(BackingExtent {
        node,
        source: BackingSource::Disk { drive, data_lba },
        size,
        generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
    });
}

/// Enregistre une etendue immutable situee dans le ramdisk UEFI deja mappe.
/// L'adresse est virtuelle et reste valide pendant toute la vie du noyau.
pub fn register_memory(node: usize, address: u64, size: usize) {
    unregister(node);
    EXTENTS.lock().push(BackingExtent {
        node,
        source: BackingSource::Memory { address },
        size,
        generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
    });
}

pub fn is_memory_backed(node: usize) -> bool {
    EXTENTS.lock().iter().any(|extent| {
        extent.node == node && matches!(extent.source, BackingSource::Memory { .. })
    })
}

pub fn unregister(node: usize) {
    EXTENTS.lock().retain(|extent| extent.node != node);
    READ_CACHE.lock().retain(|entry| entry.node != node);
    READ_PATTERNS.lock().retain(|entry| entry.node != node);
}

/// Nom historique: signifie maintenant "fichier externe immutable".
/// Les etendues ramdisk doivent suivre les memes regles RO/COW que les etendues ATA.
/// D'OU vient reellement le contenu d'un fichier.
///
/// BOUCHAUD_C50_TROIS_SOURCES_PAS_UN_BOOLEEN
///
/// `is_disk_backed` rend vrai des qu'une etendue existe -- il ne distingue pas
/// une lecture ATA d'un memcpy depuis le ramdisk UEFI. Son nom dit « disque »
/// et il signifie « fichier externe immuable ». Sur la question posee, cette
/// confusion est fatale :
///
///     tar.rs, taille <= 4 Mio   -> contenu INLINE dans le noeud, pas d'etendue
///     tar.rs, > 4 Mio, QEMU     -> register_disk(Drive::Slave)   = ATA, hdb
///     tar.rs, > 4 Mio, UEFI     -> register_memory(adresse)      = ramdisk
///
/// Les ELF de Ladybird depassent tous 4 Mio. Ils sont donc ATA sous QEMU et
/// MEMOIRE sur la Trigkey -- et une optimisation du chemin ATA qui gagnerait
/// trente secondes en CI pourrait ne rien changer sur la machine physique.
///
/// Melanger les deux dans une meme conclusion serait une erreur de mesure, pas
/// une approximation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackingKind {
    /// Aucune etendue : le contenu vit dans `fs.nodes[node].content`.
    Inline,
    /// Lecture par secteurs sur un disque ATA.
    Disk,
    /// Recopie depuis le ramdisk mappe par le chargeur UEFI.
    Memory,
}

impl BackingKind {
    /// Le nom tel qu'il parait dans les journaux. Stable : des scripts le lisent.
    pub fn etiquette(self) -> &'static str {
        match self {
            BackingKind::Inline => "inline",
            BackingKind::Disk => "ata-disk",
            BackingKind::Memory => "uefi-memory",
        }
    }
}

/// La source reelle du contenu de `node`.
pub fn kind(node: usize) -> BackingKind {
    match EXTENTS.lock().iter().find(|extent| extent.node == node) {
        None => BackingKind::Inline,
        Some(extent) => match extent.source {
            BackingSource::Disk { .. } => BackingKind::Disk,
            BackingSource::Memory { .. } => BackingKind::Memory,
        },
    }
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

    if let BackingSource::Memory { address } = extent.source {
        let debut_mem_ns = crate::kernel::timer::monotonic_ns();
        let source = match address.checked_add(offset as u64) {
            Some(value) => value,
            None => return 0,
        };
        // SAFETY: register_memory n'est appele que pour une plage entierement
        // incluse dans le ramdisk mappe par bootloader_api. `wanted` est borne
        // par extent.size ci-dessus.
        unsafe {
            core::ptr::copy_nonoverlapping(source as *const u8, out.as_mut_ptr(), wanted);
        }
        let duree = crate::kernel::timer::monotonic_ns().saturating_sub(debut_mem_ns);
        MEM_READ_OPS.fetch_add(1, Ordering::Relaxed);
        MEM_READ_BYTES.fetch_add(wanted as u64, Ordering::Relaxed);
        MEM_READ_NS.fetch_add(duree, Ordering::Relaxed);
        MEM_READ_WORST_NS.fetch_max(duree, Ordering::Relaxed);
        return wanted;
    }

    let (drive, data_lba) = match extent.source {
        BackingSource::Disk { drive, data_lba } => (drive, data_lba),
        BackingSource::Memory { .. } => unreachable!(),
    };
    let mut done = 0usize;
    let mut absolute = offset;

    let intra = absolute % SECTOR_SIZE;
    if intra != 0 && done < wanted {
        let mut sector = [0u8; SECTOR_SIZE];
        let lba = data_lba + (absolute / SECTOR_SIZE) as u64;
        if block::read_blocks(drive, lba, 1, &mut sector) != 1 {
            return done;
        }
        let take = core::cmp::min(SECTOR_SIZE - intra, wanted - done);
        out[done..done + take].copy_from_slice(&sector[intra..intra + take]);
        done += take;
        absolute += take;
    }

    let debut_io_ns = crate::kernel::timer::monotonic_ns();

    let full_sectors = (wanted - done) / SECTOR_SIZE;
    if full_sectors > 0 {
        let bytes = full_sectors * SECTOR_SIZE;
        let lba = data_lba + (absolute / SECTOR_SIZE) as u64;
        let read = block::read_blocks(
            drive,
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
            note_duree_io(debut_io_ns);
            return done;
        }
    }

    if done < wanted {
        let mut sector = [0u8; SECTOR_SIZE];
        let lba = data_lba + (absolute / SECTOR_SIZE) as u64;
        if block::read_blocks(drive, lba, 1, &mut sector) == 1 {
            let take = wanted - done;
            out[done..done + take].copy_from_slice(&sector[..take]);
            done += take;
        }
    }

    DISK_READ_OPS.fetch_add(1, Ordering::Relaxed);
    DISK_READ_BYTES.fetch_add(done as u64, Ordering::Relaxed);
    note_duree_io(debut_io_ns);
    done
}

/// Ajoute une lecture au cumul, et retient la pire.
///
/// La pire compte autant que le total : deux mille lectures a dix
/// microsecondes et vingt a une milliseconde donnent le meme cumul, mais la
/// premiere est un chargement paresseux qui se voit a peine et la seconde est
/// une saccade.
fn note_duree_io(debut_ns: u64) {
    let duree = crate::kernel::timer::monotonic_ns().saturating_sub(debut_ns);
    DISK_READ_NS.fetch_add(duree, Ordering::Relaxed);
    DISK_READ_WORST_NS.fetch_max(duree, Ordering::Relaxed);
}

/// (lectures, octets, nanosecondes cumulees, pire) du chemin ATA seul.
pub fn disk_read_timing() -> (u64, u64, u64, u64) {
    (
        DISK_READ_OPS.load(Ordering::Relaxed),
        DISK_READ_BYTES.load(Ordering::Relaxed),
        DISK_READ_NS.load(Ordering::Relaxed),
        DISK_READ_WORST_NS.load(Ordering::Relaxed),
    )
}

/// (lectures, octets, nanosecondes cumulees, pire) du chemin RAMDISK seul.
pub fn memory_read_timing() -> (u64, u64, u64, u64) {
    (
        MEM_READ_OPS.load(Ordering::Relaxed),
        MEM_READ_BYTES.load(Ordering::Relaxed),
        MEM_READ_NS.load(Ordering::Relaxed),
        MEM_READ_WORST_NS.load(Ordering::Relaxed),
    )
}

/// Lit via une fenêtre read-ahead partagée entre processus.
///
/// Les faults ELF sont typiquement des lectures de 4 KiB consécutives. Une
/// fenêtre alignée de 16 KiB transforme quatre faults en une commande backing,
/// sans précharger un binaire entier. Le cache est borné à 4 MiB et partagé par
/// identité de nœud; les processus Ladybird relisant les mêmes pages propres
/// réutilisent donc les octets déjà lus.
pub fn read_at(node: usize, offset: usize, out: &mut [u8]) -> usize {
    // Un ramdisk est deja de la memoire: inutile d'allouer un cache read-ahead
    // pour recopier une zone qui se lit directement. Le page-cache du MM prend
    // ensuite le relais pour partager les pages ELF propres entre processus.
    if out.is_empty() || !is_disk_backed(node) || is_memory_backed(node)
        || out.len() > READAHEAD_MAX
    {
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
