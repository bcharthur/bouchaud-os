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

/// Initialise l'acces memoire physique et l'arene DMA depuis la memory map.
pub fn init(boot: &'static BootInfo) {
    unsafe { PHYS_OFFSET = boot.physical_memory_offset; }

    // Choisit la plus grande region RAM libre (>= 1 MiB) comme arene DMA.
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
    unsafe {
        DMA_NEXT = (best_start + 0xFFF) & !0xFFF;
        DMA_END = best_start + best_len;
    }
    crate::kernel::dmesg::log("memory: acces physique + arene DMA prets");
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
    crate::println!("pagination par processus: planifiee (memoire virtuelle a venir)");
}

