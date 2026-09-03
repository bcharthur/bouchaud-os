//! Gestion memoire de haut niveau.
//!
//! - Tas statique (voir `kernel::heap`).
//! - Acces a la memoire physique (offset fourni par le bootloader via la
//!   feature `map_physical_memory`) et petit allocateur de frames DMA pour les
//!   pilotes (e1000). A terme : frames physiques generiques + pagination.

use bootloader::bootinfo::MemoryRegionType;
use bootloader::BootInfo;
use crate::kernel::heap;
use crate::kernel::arene_dma::AreneDma;
use x86_64::instructions::interrupts;

static mut PHYS_OFFSET: u64 = 0;
static mut USER_START: u64 = 0;
static mut USER_END: u64 = 0;

// BOUCHAUD_C3_ARENE_DMA_V1
//
// L'arene etait trois `static mut` et un pointeur qui monte. Elle ne rendait
// RIEN -- un anneau reseau reinitialise perdait sa memoire jusqu'au
// redemarrage -- et c'etait une COURSE : deux pilotes s'initialisant en
// parallele pouvaient recevoir la meme adresse physique et se marcher dessus
// dans un tampon que le materiel lit.
//
// `memory::arene_dma` porte maintenant la frontiere ET une liste de regions
// rendues, fusionnante et bornee, sous son propre verrou.
static ARENE: AreneDma = AreneDma::neuve();

/// Reserve en fin de la plus grande region pour l'arene DMA (pilotes).
const DMA_RESERVE: u64 = 32 * 1024 * 1024;

/// Reserve pour les frames physiques distribuees aux processus utilisateur
/// (tables de pages, segments ELF, piles, `mmap`). Prelevee sur la plus grande
/// region, juste avant l'arene DMA : sans elle, le tas noyau avalerait toute la
/// RAM et `vmm::alloc_frame` n'aurait plus rien a donner au ring 3.
///
/// 512 Mio : une pile graphique statique (Qt + son moteur de rendu) mappe
/// facilement 100 a 200 Mio entre son image, ses tampons de dessin et le tas de
/// ses threads. On ne les prend que si la RAM le permet (cf. `init`).
const USER_RESERVE_DEFAULT: u64 = 512 * 1024 * 1024;
/// Repli lorsque la machine est trop juste pour la reserve complete.
const USER_RESERVE_MIN: u64 = 192 * 1024 * 1024;
/// Plafond de la reserve prelevee sur la plus grande region.
/// Les autres regions `Usable` sont de toute facon ajoutees au VMM ensuite.
const USER_RESERVE_MAX: u64 = 4 * 1024 * 1024 * 1024;

/// Initialise l'acces memoire physique, etend le tas sur la plus grande region
/// de RAM libre, et reserve une arene DMA. La memoire physique est entierement
/// mappee a `PHYS_OFFSET` (feature `map_physical_memory` du bootloader).
pub fn init(boot: &'static BootInfo) {
    unsafe { PHYS_OFFSET = boot.physical_memory_offset; }

    // Choisit la plus grande region RAM libre (>= 1 MiB).
    let mut best_start = 0u64;
    let mut best_len = 0u64;
    for region in boot.memory_map.iter() {
        if region.region_type == MemoryRegionType::Usable {
            let start = region.range.start_addr();
            let end = region.range.end_addr();
            if end > start && start >= 0x100000 && (end - start) > best_len {
                best_len = end - start;
                best_start = start;
            }
        }
    }

    // Decoupe de la plus grande region :
    //   [debut .. u) -> tas noyau
    //   [u .. d)     -> frames physiques utilisateur (vmm)
    //   [d .. fin)   -> arene DMA (pilotes)
    // On exige une region assez grande, sinon on garde le tas bootstrap statique.
    let heap_start = (best_start + 0xFFF) & !0xFFF;
    let region_end = best_start + best_len;
    // Le tas noyau garde au moins la moitie de la region : le moteur de rendu
    // maison en depend autant que le ring 3 depend de ses frames.
    // Dimensionnement dynamique : avec 8+ Gio donnes a QEMU, ne pas rester
    // artificiellement bloque a la reserve historique de 512 Mio. On conserve
    // toutefois au moins la moitie de la plus grande region pour le tas noyau,
    // dont le rendu CPU Ladybird depend encore fortement.
    let max_user_while_preserving_heap =
        (best_len / 2).saturating_sub(DMA_RESERVE);
    let user_reserve = if max_user_while_preserving_heap >= USER_RESERVE_DEFAULT {
        max_user_while_preserving_heap.min(USER_RESERVE_MAX)
    } else {
        USER_RESERVE_MIN.min(max_user_while_preserving_heap)
    };
    if best_len > DMA_RESERVE + user_reserve + 16 * 1024 * 1024 {
        let dma_start = (region_end - DMA_RESERVE) & !0xFFF;
        let user_start = (dma_start - user_reserve) & !0xFFF;
        let heap_size = (user_start - heap_start) as usize;
        unsafe {
            // Bascule le tas sur la grande arene physique (avant toute
            // allocation persistante : seul le bootstrap statique a servi).
            heap::switch_arena(phys_to_virt(heap_start), heap_size);
            USER_START = user_start;
            USER_END = dma_start;
        }
        ARENE.configure(dma_start, region_end);
    } else if best_len > DMA_RESERVE + 16 * 1024 * 1024 {
        let dma_start = (region_end - DMA_RESERVE) & !0xFFF;
        let heap_size = (dma_start - heap_start) as usize;
        unsafe {
            heap::switch_arena(phys_to_virt(heap_start), heap_size);
        }
        ARENE.configure(dma_start, region_end);
    } else {
        // Region trop petite : DMA seule, tas bootstrap conserve.
        ARENE.configure(heap_start, region_end);
    }
    crate::kernel::dmesg::log("memory: acces physique + tas etendu + arene DMA prets");

    // Alimente l'allocateur de frames : l'arene utilisateur reservee ci-dessus,
    // plus toutes les autres regions RAM utilisables laissees de cote.
    unsafe {
        if USER_END > USER_START {
            crate::kernel::vmm::add_region(USER_START, USER_END);
        }
    }
    for region in boot.memory_map.iter() {
        if region.region_type != MemoryRegionType::Usable {
            continue;
        }
        let start = region.range.start_addr();
        let end = region.range.end_addr();
        // La grande region est deja repartie (tas / user / DMA).
        if start == best_start {
            continue;
        }
        if start >= 0x100000 && end > start {
            crate::kernel::vmm::add_region(start, end);
        }
    }
}

/// Offset de la memoire physique mappee (virtuel = offset + physique).
pub fn phys_offset() -> u64 {
    unsafe { PHYS_OFFSET }
}

/// Pointeur virtuel pour acceder a une adresse physique donnee.
pub fn phys_to_virt(phys: u64) -> *mut u8 {
    (unsafe { PHYS_OFFSET } + phys) as *mut u8
}

/// Alloue un bloc DMA (aligne page, mis a zero). Renvoie (adresse physique,
/// pointeur virtuel). `None` si l'arene est epuisee.
pub fn alloc_dma(size: usize) -> Option<(u64, *mut u8)> {
    // Interruptions masquees : l'arene est atteignable depuis l'initialisation
    // d'un pilote comme depuis un gestionnaire, et son verrou est un verrou
    // tournant simple.
    let base = interrupts::without_interrupts(|| ARENE.alloue(size))?;
    let virt = phys_to_virt(base);
    // La remise a zero reste HORS du verrou : elle peut porter sur plusieurs
    // centaines de kilooctets, et la tenir sous le verrou de l'arene
    // serialiserait l'initialisation de tous les pilotes derriere le plus gros
    // tampon.
    unsafe { core::ptr::write_bytes(virt, 0, size) };
    Some((base, virt))
}

/// Rend un bloc DMA a l'arene.
///
/// `base` et `size` doivent etre ceux rendus par [`alloc_dma`]. Rendre une
/// region hors arene est compte (`debordements`) et ignore : l'ajouter a la
/// liste corromprait les allocations suivantes.
pub fn free_dma(base: u64, size: usize) {
    interrupts::without_interrupts(|| ARENE.libere(base, size));
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DmaStats {
    pub used: u64,
    pub free: u64,
    pub total: u64,
    pub allocations: u64,
    pub failures: u64,
}

pub fn dma_stats() -> DmaStats {
    let etat = ARENE.etat();
    DmaStats {
        used: etat.utilise,
        free: etat.libre,
        total: etat.total,
        allocations: etat.allocations,
        failures: etat.echecs,
    }
}

/// L'etat complet de l'arene, pour le releve periodique.
pub fn dma_etat() -> crate::kernel::arene_dma::EtatDma {
    ARENE.etat()
}

pub fn log_dma_stats() {
    let e = ARENE.etat();
    crate::serial_println!(
        "[MEM-NG-DMA] total={} utilise={} rendu={} regions={} pic={} allocations={} liberations={} reutilisations={} fusions={} debordements={} echecs={}",
        e.total, e.utilise, e.rendu, e.regions, e.pic, e.allocations,
        e.liberations, e.reutilisations, e.fusions, e.debordements, e.echecs
    );
}

/// Octets de tas utilises.
pub fn used() -> usize { heap::stats().0 }
/// Octets de tas libres.
pub fn free() -> usize { heap::stats().1 }
/// Taille totale du tas.
pub fn total() -> usize { heap::stats().2 }

/// Affiche un resume memoire (commande `free`).
pub fn print_info() {
    let (u, f, t) = heap::stats();
    crate::println!("Memoire (tas noyau):");
    crate::println!("  total : {} o", t);
    crate::println!("  utilise: {} o", u);
    crate::println!("  libre : {} o", f);
    crate::println!("offset physique: {:#x}", phys_offset());
    let (fu, ff, ft) = crate::kernel::vmm::frame_stats();
    crate::println!("Frames physiques (pages utilisateur 4 KiB):");
    crate::println!("  total : {} ({} MiB)", ft, ft * 4096 / (1024 * 1024));
    crate::println!("  utilisees: {}", fu);
    crate::println!("  libres: {}", ff);
}

