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
/// Plage physique deja attribuee au tas et a l'arene DMA. L'allocateur de
/// frames (`kernel::vm`) doit l'exclure : servir ces frames ecraserait le tas
/// du noyau, avec une panique immediate et difficile a relier a sa cause.
static mut RESERVED_START: u64 = 0;
static mut RESERVED_END: u64 = 0;
/// Plage physique dediee aux frames servies par `kernel::vm`.
static mut FRAME_START: u64 = 0;
static mut FRAME_END: u64 = 0;

/// Reserve en fin de la plus grande region pour l'arene DMA (pilotes).
const DMA_RESERVE: u64 = 32 * 1024 * 1024;

/// Reserve pour l'allocateur de frames physiques (`kernel::vm`).
///
/// Sans ce decoupage explicite, le tas prend toute la grande region et il ne
/// reste litteralement aucune frame a servir : les autres regions declarees
/// libres par le BIOS sont minuscules ou sous la barre du premier mega-octet.
const FRAME_RESERVE: u64 = 96 * 1024 * 1024;

/// Taille minimale laissee au tas noyau apres les deux reserves.
const HEAP_MINIMUM: u64 = 16 * 1024 * 1024;

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

    // Decoupe de la grande region, du debut vers la fin :
    //   [tas][frames physiques][DMA]
    // Le tas passe en premier car il doit basculer avant toute allocation
    // persistante ; les deux reserves sont prises sur sa queue.
    let heap_start = (best_start + 0xFFF) & !0xFFF;
    let region_end = best_start + best_len;

    if best_len > DMA_RESERVE + FRAME_RESERVE + HEAP_MINIMUM {
        let dma_start = (region_end - DMA_RESERVE) & !0xFFF;
        let frame_start = (dma_start - FRAME_RESERVE) & !0xFFF;
        let heap_size = (frame_start - heap_start) as usize;
        unsafe {
            // Bascule le tas sur la grande arene physique (avant toute
            // allocation persistante : seul le bootstrap statique a servi).
            heap::switch_arena(phys_to_virt(heap_start), heap_size);
            DMA_NEXT = dma_start;
            DMA_END = region_end;
            FRAME_START = frame_start;
            FRAME_END = dma_start;
            RESERVED_START = heap_start;
            RESERVED_END = region_end;
        }
    } else if best_len > DMA_RESERVE + HEAP_MINIMUM {
        // Region etroite : on sacrifie l'arene de frames, pas le tas.
        let dma_start = (region_end - DMA_RESERVE) & !0xFFF;
        let heap_size = (dma_start - heap_start) as usize;
        unsafe {
            heap::switch_arena(phys_to_virt(heap_start), heap_size);
            DMA_NEXT = dma_start;
            DMA_END = region_end;
            RESERVED_START = heap_start;
            RESERVED_END = region_end;
        }
    } else {
        // Region trop petite : DMA seule, tas bootstrap conserve.
        unsafe {
            DMA_NEXT = heap_start;
            DMA_END = region_end;
            RESERVED_START = heap_start;
            RESERVED_END = region_end;
        }
    }

    crate::kernel::dmesg::log("memory: acces physique + tas etendu + arene DMA prets");
}

/// Plage physique `[debut, fin)` deja prise par le tas et le DMA.
pub fn reserved_range() -> (u64, u64) {
    unsafe { (RESERVED_START, RESERVED_END) }
}

/// Plage physique `[debut, fin)` dediee aux frames de `kernel::vm`.
/// `(0, 0)` si la RAM disponible n'a pas permis de la reserver.
pub fn frame_range() -> (u64, u64) {
    unsafe { (FRAME_START, FRAME_END) }
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
    let (reserved_start, reserved_end) = reserved_range();
    crate::println!(
        "reserve tas+DMA: {:#x}..{:#x} ({} Mio)",
        reserved_start,
        reserved_end,
        (reserved_end - reserved_start) / (1024 * 1024)
    );
    if crate::kernel::vm::ready() {
        let (free_frames, allocated, released) = crate::kernel::vm::stats();
        crate::println!(
            "frames physiques: {} libres ({} Mio), {} servies, {} rendues",
            free_frames,
            free_frames as u64 * crate::kernel::vm::FRAME_SIZE / (1024 * 1024),
            allocated,
            released
        );
    } else {
        crate::println!("frames physiques: allocateur non initialise");
    }
}

