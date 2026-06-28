//! Allocateur de tas du noyau (active `alloc` : Vec, String, BTreeMap...).
//!
//! On utilise un tas **statique** de taille fixe place dans le `.bss` (donc
//! deja mappe par le bootloader, sans manipuler la pagination) confie a
//! `linked_list_allocator`. Simple et robuste : suffisant pour passer aux
//! structures dynamiques sans risquer une faute de page au boot.

use linked_list_allocator::LockedHeap;

/// Taille du tas noyau (128 MiB) : framebuffer HD (~3,7 Mo) + GUI + tampons
/// reseau/TLS + rendu de pages web modernes (DOM + CSS + JS + images).
///
/// Les pages comme Google chargent des bundles JS dont le lexing/parsing cree
/// temporairement des structures volumineuses (tokens + AST). Avec 48 MiB, une
/// seule croissance de Vec a 20 MiB pouvait echouer apres TLS/cache/DOM. Le tas
/// reste statique en .bss, mais il doit refleter le budget reel d'un navigateur.
pub const HEAP_SIZE: usize = 128 * 1024 * 1024;

static mut HEAP_SPACE: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Initialise l'allocateur. A appeler une seule fois, tres tot au boot.
pub fn init() {
    unsafe {
        ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP_SPACE) as *mut u8, HEAP_SIZE);
    }
    crate::kernel::dmesg::log("heap: allocateur initialise (128 MiB)");
}

/// Renvoie (octets utilises, octets libres, taille totale) du tas.
pub fn stats() -> (usize, usize, usize) {
    let heap = ALLOCATOR.lock();
    (heap.used(), heap.free(), HEAP_SIZE)
}
