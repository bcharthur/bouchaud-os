//! Gestion memoire de haut niveau.
//!
//! - Tas statique (voir `kernel::heap`).
//! - Acces a la memoire physique (offset fourni par le bootloader via la
//!   feature `map_physical_memory`) et petit allocateur de frames DMA pour les
//!   pilotes (e1000). A terme : frames physiques generiques + pagination.

use bootloader::bootinfo::MemoryRegionType;
use bootloader::BootInfo;
use crate::kernel::heap;

static mut PHYS_OFFSET: u64 = 0;
static mut DMA_NEXT: u64 = 0;
static mut DMA_END: u64 = 0;
static mut USER_START: u64 = 0;
static mut USER_END: u64 = 0;

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
const USER_RESERVE: u64 = 512 * 1024 * 1024;
/// Repli lorsque la machine est trop juste pour la reserve complete.
const USER_RESERVE_MIN: u64 = 192 * 1024 * 1024;

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
    let user_reserve = if best_len / 2 >= USER_RESERVE + DMA_RESERVE {
        USER_RESERVE
    } else {
        USER_RESERVE_MIN
    };
    if best_len > DMA_RESERVE + user_reserve + 16 * 1024 * 1024 {
        let dma_start = (region_end - DMA_RESERVE) & !0xFFF;
        let user_start = (dma_start - user_reserve) & !0xFFF;
        let heap_size = (user_start - heap_start) as usize;
        unsafe {
            // Bascule le tas sur la grande arene physique (avant toute
            // allocation persistante : seul le bootstrap statique a servi).
            heap::switch_arena(phys_to_virt(heap_start), heap_size);
            DMA_NEXT = dma_start;
            DMA_END = region_end;
            USER_START = user_start;
            USER_END = dma_start;
        }
    } else if best_len > DMA_RESERVE + 16 * 1024 * 1024 {
        let dma_start = (region_end - DMA_RESERVE) & !0xFFF;
        let heap_size = (dma_start - heap_start) as usize;
        unsafe {
            heap::switch_arena(phys_to_virt(heap_start), heap_size);
            DMA_NEXT = dma_start;
            DMA_END = region_end;
        }
    } else {
        // Region trop petite : DMA seule, tas bootstrap conserve.
        unsafe {
            DMA_NEXT = heap_start;
            DMA_END = region_end;
        }
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
    unsafe {
        let base = (DMA_NEXT + 0xFFF) & !0xFFF;
        let end = base + (((size as u64) + 0xFFF) & !0xFFF);
        if DMA_END == 0 || end > DMA_END { return None; }
        DMA_NEXT = end;
        let virt = phys_to_virt(base);
        core::ptr::write_bytes(virt, 0, size);
        Some((base, virt))
    }
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

