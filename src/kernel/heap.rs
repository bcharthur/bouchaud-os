//! Allocateur de tas du noyau (active `alloc` : Vec, String, BTreeMap...).
//!
//! On utilise un tas **statique** de taille fixe place dans le `.bss` (donc
//! deja mappe par le bootloader, sans manipuler la pagination) confie a
//! `linked_list_allocator`. Simple et robuste : suffisant pour passer aux
//! structures dynamiques sans risquer une faute de page au boot.

use linked_list_allocator::LockedHeap;

/// Taille du tas noyau (16 MiB) : marge pour le framebuffer HD 1280x720x32
/// (~3,7 Mo de double-buffer) + GUI + tampons reseau/TLS.
pub const HEAP_SIZE: usize = 16 * 1024 * 1024;

static mut HEAP_SPACE: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Initialise l'allocateur. A appeler une seule fois, tres tot au boot.
pub fn init() {
    unsafe {
        ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP_SPACE) as *mut u8, HEAP_SIZE);
    }
    crate::kernel::dmesg::log("heap: allocateur initialise (16 MiB)");
}

/// Renvoie (octets utilises, octets libres, taille totale) du tas.
pub fn stats() -> (usize, usize, usize) {
    let heap = ALLOCATOR.lock();
    (heap.used(), heap.free(), HEAP_SIZE)
}
