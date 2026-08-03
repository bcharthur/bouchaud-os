//! Memoire virtuelle : allocateur de frames physiques et gestion des pages.
//!
//! Premiere couche du chemin vers l'execution de binaires natifs. Tant que le
//! noyau ne sait pas creer d'espaces d'adressage, il ne peut ni charger un
//! ELF, ni isoler un processus, ni offrir `mmap` — donc ni libc, ni editeur de
//! liens dynamique, ni serveur graphique.
//!
//! Le bootloader nous laisse dans un etat confortable : la memoire physique
//! est entierement mappee a `memory::phys_offset()` (feature
//! `map_physical_memory`), et CR3 pointe sur une table de pages valide. On
//! reprend cette table plutot que d'en construire une nouvelle : c'est ce qui
//! permet de mapper des pages sans se couper l'herbe sous le pied.
//!
//! Deux ressources distinctes :
//!
//!   * les **frames physiques**, servies par `FrameArena` depuis les regions
//!     que la carte memoire du bootloader declare libres, moins ce que le tas
//!     et l'arene DMA ont deja pris ;
//!   * les **pages virtuelles**, mappees vers ces frames via les tables x86_64
//!     a quatre niveaux.

use alloc::vec::Vec;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::mapper::{MapToError, TranslateResult, UnmapError};
use x86_64::structures::paging::{
    FrameAllocator, FrameDeallocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags,
    PhysFrame, Size4KiB, Translate,
};
use x86_64::{PhysAddr, VirtAddr};

use crate::kernel::memory;

/// Taille d'une frame (4 KiB, le seul granule qu'on sert).
pub const FRAME_SIZE: u64 = 4096;

/// Etat global de l'allocateur de frames.
static mut ARENA: Option<FrameArena> = None;

/// Une plage de frames libres, prise dans la carte memoire du bootloader.
struct Region {
    /// Prochaine frame jamais servie de cette region.
    next: u64,
    /// Fin exclusive.
    end: u64,
}

/// Allocateur de frames physiques.
///
/// Deux sources, consultees dans cet ordre : les frames rendues (`freed`),
/// puis les regions jamais entamees. Rendre une frame la remet donc en tete,
/// ce qui garde chaude la memoire recemment utilisee.
pub struct FrameArena {
    regions: Vec<Region>,
    freed: Vec<u64>,
    /// Compteurs de service, pour `vm stats`.
    allocated: usize,
    released: usize,
}

impl FrameArena {
    fn new() -> FrameArena {
        FrameArena { regions: Vec::new(), freed: Vec::new(), allocated: 0, released: 0 }
    }

    /// Ajoute `[start, end)` aux frames disponibles, aligne sur la page.
    fn add_region(&mut self, start: u64, end: u64) {
        let start = (start + FRAME_SIZE - 1) & !(FRAME_SIZE - 1);
        let end = end & !(FRAME_SIZE - 1);
        if end > start {
            self.regions.push(Region { next: start, end });
        }
    }

    /// Nombre de frames encore servables.
    pub fn free_frames(&self) -> usize {
        let mut total = self.freed.len();
        for region in &self.regions {
            total += ((region.end - region.next) / FRAME_SIZE) as usize;
        }
        total
    }

    pub fn allocated(&self) -> usize {
        self.allocated
    }

    pub fn released(&self) -> usize {
        self.released
    }

    /// Sert une frame physique, mise a zero.
    pub fn allocate(&mut self) -> Option<PhysAddr> {
        let address = if let Some(recycled) = self.freed.pop() {
            recycled
        } else {
            let mut found = None;
            for region in self.regions.iter_mut() {
                if region.next < region.end {
                    found = Some(region.next);
                    region.next += FRAME_SIZE;
                    break;
                }
            }
            found?
        };

        // Une frame servie doit etre vierge : sans ca, un espace d'adressage
        // utilisateur heriterait des restes du precedent.
        unsafe {
            core::ptr::write_bytes(memory::phys_to_virt(address), 0, FRAME_SIZE as usize);
        }
        self.allocated += 1;
        Some(PhysAddr::new(address))
    }

    /// Rend une frame au pool.
    pub fn release(&mut self, address: PhysAddr) {
        self.freed.push(address.as_u64());
        self.released += 1;
    }
}

// `x86_64` pilote lui-meme l'allocation des tables intermediaires : on lui
// expose l'arene par ces deux traits.
unsafe impl FrameAllocator<Size4KiB> for FrameArena {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.allocate().map(|address| PhysFrame::containing_address(address))
    }
}

impl FrameDeallocator<Size4KiB> for FrameArena {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        self.release(frame.start_address());
    }
}

/// Construit l'allocateur de frames. Doit etre appele apres `memory::init`,
/// qui reserve la plage dediee.
pub fn init(boot: &'static bootloader::BootInfo) {
    use bootloader::bootinfo::MemoryRegionType;

    let mut arena = FrameArena::new();

    // Source principale : la plage que `memory::init` a mise de cote.
    let (frame_start, frame_end) = memory::frame_range();
    if frame_end > frame_start {
        arena.add_region(frame_start, frame_end);
    }

    // Complement : les autres regions libres de la carte memoire, hors de ce
    // que le tas et le DMA occupent. Elles sont petites, mais gratuites.
    let (reserved_start, reserved_end) = memory::reserved_range();
    for region in boot.memory_map.iter() {
        if region.region_type != MemoryRegionType::Usable {
            continue;
        }
        // Le premier mega-octet contient le BIOS et les tables du bootloader.
        let start = region.range.start_addr().max(0x100000);
        let end = region.range.end_addr();
        if end <= start {
            continue;
        }
        // Servir une frame du tas ou du DMA ecraserait le noyau : panique
        // immediate, et difficile a relier a sa cause.
        if start < reserved_end && end > reserved_start {
            if start < reserved_start {
                arena.add_region(start, reserved_start);
            }
            if end > reserved_end {
                arena.add_region(reserved_end, end);
            }
        } else {
            arena.add_region(start, end);
        }
    }

    let frames = arena.free_frames();
    unsafe {
        ARENA = Some(arena);
    }
    crate::kernel::dmesg::log(&alloc::format!(
        "vm: {} frames libres ({} Mio), pagination active",
        frames,
        frames as u64 * FRAME_SIZE / (1024 * 1024)
    ));
}

/// Acces a l'arene. `None` avant `init`.
fn arena() -> Option<&'static mut FrameArena> {
    unsafe { ARENA.as_mut() }
}

pub fn ready() -> bool {
    unsafe { ARENA.is_some() }
}

/// Table de pages active, vue au travers de l'offset physique.
///
/// # Securite
/// L'appelant doit garantir qu'une seule vue mutable existe a la fois : deux
/// `OffsetPageTable` simultanes sur la meme table permettraient des ecritures
/// concurrentes sur les entrees.
unsafe fn active_table() -> OffsetPageTable<'static> {
    let (level4, _) = Cr3::read();
    let virt = memory::phys_offset() + level4.start_address().as_u64();
    let table: *mut PageTable = virt as *mut PageTable;
    OffsetPageTable::new(&mut *table, VirtAddr::new(memory::phys_offset()))
}

/// Drapeaux d'une page noyau ordinaire : presente, accessible en ecriture.
pub fn kernel_flags() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::WRITABLE
}

/// Drapeaux d'une page utilisateur (ring 3).
pub fn user_flags() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE
}

/// Mappe `page` sur `frame`. Les tables intermediaires manquantes sont creees.
pub fn map_to(page: Page<Size4KiB>, frame: PhysFrame<Size4KiB>, flags: PageTableFlags)
    -> Result<(), MapToError<Size4KiB>>
{
    let allocator = match arena() {
        Some(a) => a,
        None => return Err(MapToError::FrameAllocationFailed),
    };
    unsafe {
        let mut table = active_table();
        table.map_to(page, frame, flags, allocator)?.flush();
    }
    Ok(())
}

/// Alloue une frame et la mappe sur `page`. Renvoie l'adresse physique servie.
pub fn map_alloc(page: Page<Size4KiB>, flags: PageTableFlags)
    -> Result<PhysAddr, MapToError<Size4KiB>>
{
    let allocator = match arena() {
        Some(a) => a,
        None => return Err(MapToError::FrameAllocationFailed),
    };
    let address = match allocator.allocate() {
        Some(a) => a,
        None => return Err(MapToError::FrameAllocationFailed),
    };
    let frame = PhysFrame::containing_address(address);
    unsafe {
        let mut table = active_table();
        match table.map_to(page, frame, flags, allocator) {
            Ok(flush) => flush.flush(),
            Err(error) => {
                // Le mappage a echoue : la frame ne doit pas fuir.
                allocator.release(address);
                return Err(error);
            }
        }
    }
    Ok(address)
}

/// Demappe `page` et rend sa frame au pool.
pub fn unmap(page: Page<Size4KiB>) -> Result<PhysAddr, UnmapError> {
    let (frame, flush) = unsafe {
        let mut table = active_table();
        table.unmap(page)?
    };
    flush.flush();
    let address = frame.start_address();
    if let Some(allocator) = arena() {
        allocator.release(address);
    }
    Ok(address)
}

/// Traduit une adresse virtuelle en physique, ou `None` si non mappee.
pub fn translate(address: VirtAddr) -> Option<PhysAddr> {
    unsafe {
        let table = active_table();
        match table.translate(address) {
            TranslateResult::Mapped { frame, offset, .. } => {
                Some(frame.start_address() + offset)
            }
            _ => None,
        }
    }
}

/// (frames libres, frames servies, frames rendues).
pub fn stats() -> (usize, usize, usize) {
    match arena() {
        Some(a) => (a.free_frames(), a.allocated(), a.released()),
        None => (0, 0, 0),
    }
}

/// Alloue `count` frames et les mappe consecutivement a partir de `start`.
/// Toute erreur en cours de route defait ce qui a deja ete fait.
pub fn map_range(start: VirtAddr, count: usize, flags: PageTableFlags)
    -> Result<(), MapToError<Size4KiB>>
{
    let first: Page<Size4KiB> = Page::containing_address(start);
    for index in 0..count {
        let page = first + index as u64;
        if let Err(error) = map_alloc(page, flags) {
            for done in 0..index {
                let _ = unmap(first + done as u64);
            }
            return Err(error);
        }
    }
    Ok(())
}

/// Demappe `count` pages a partir de `start`.
pub fn unmap_range(start: VirtAddr, count: usize) {
    let first: Page<Size4KiB> = Page::containing_address(start);
    for index in 0..count {
        let _ = unmap(first + index as u64);
    }
}
