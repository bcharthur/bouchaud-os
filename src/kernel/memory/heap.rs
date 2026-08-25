//! Allocateur de tas du noyau (active `alloc` : Vec, String, BTreeMap...).
//!
//! Deux temps :
//!   1. **Bootstrap** : un petit tas statique en `.bss` (8 MiB), deja mappe par
//!      le bootloader, suffisant pour les rares allocations du tout debut du
//!      boot. On le garde minuscule car un `.bss` trop gros fait paniquer le
//!      bootloader 0.9 (`too many memory regions in memory map`).
//!   2. **Extension** : des que `kernel::memory` a lu la carte memoire, on
//!      bascule l'allocateur sur une grande region de RAM physique mappee
//!      (cf. `switch_arena`). C'est ainsi qu'on obtient un tas de plusieurs
//!      centaines de Mio (rendu de pages web, gros bundles JS) sans grossir le
//!      `.bss` du noyau.

use linked_list_allocator::LockedHeap;

/// Taille du tas statique de bootstrap (8 MiB). Volontairement petit : voir
/// l'explication ci-dessus sur la limite du bootloader.
pub const BOOTSTRAP_SIZE: usize = 8 * 1024 * 1024;

static mut HEAP_SPACE: [u8; BOOTSTRAP_SIZE] = [0; BOOTSTRAP_SIZE];

/// Taille reelle du tas actif (mise a jour apres `switch_arena`).
static mut HEAP_TOTAL: usize = BOOTSTRAP_SIZE;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Initialise l'allocateur sur le tas statique de bootstrap. A appeler une
/// seule fois, tres tot au boot (avant toute allocation).
pub fn init() {
    unsafe {
        ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP_SPACE) as *mut u8, BOOTSTRAP_SIZE);
    }
    crate::kernel::dmesg::log("heap: bootstrap 8 MiB initialise");
}

/// Bascule l'allocateur sur une grande arene de RAM physique mappee.
///
/// # Securite
/// `start` doit pointer sur `size` octets de memoire valide, mappee pour la
/// duree de vie du noyau, et inutilisee. Ne doit etre appele qu'avant toute
/// allocation persistante (sinon les anciens pointeurs deviendraient invalides).
pub unsafe fn switch_arena(start: *mut u8, size: usize) {
    ALLOCATOR.lock().init(start, size);
    HEAP_TOTAL = size;
    crate::kernel::dmesg::log("heap: arene physique active (RAM etendue)");
}

/// Renvoie (octets utilises, octets libres, taille totale) du tas.
pub fn stats() -> (usize, usize, usize) {
    let heap = ALLOCATOR.lock();
    (heap.used(), heap.free(), unsafe { HEAP_TOTAL })
}
