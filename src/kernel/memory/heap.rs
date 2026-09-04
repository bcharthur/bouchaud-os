//! P0-NG1 kernel heap: global backing arena + bounded per-CPU size caches.
//!
//! The old `LockedHeap` remains the proven backing allocator, but the hot path
//! for small kernel objects no longer takes its global lock on every alloc/free.
//! Six fixed size classes keep at most a small, bounded amount of memory per CPU.
//!
//! # Chantier 3 : le depot de magasins
//!
//! Les caches per-CPU supprimaient le verrou global AU MILIEU, pas AUX BORDS.
//! Liste vide : une allocation backing par objet. Liste pleine : une liberation
//! backing par objet. Un fil qui oscille autour du plafond -- un compositeur
//! qui alloue et rend un tampon par trame, un analyseur qui construit et jette
//! des noeuds -- payait donc le verrou global a CHAQUE operation.
//!
//! `memory::magasin` intercale un DEPOT par classe entre le cache et le
//! backing : une pile de magasins de `LOT` blocs deja decoupes. Une liste vide
//! se remplit d'un coup, une liste pleine se vide d'un coup, et le backing
//! n'est atteint que lorsque le depot est vide (premiere chauffe) ou plein
//! (memoire reellement rendue).
//!
//! `[MEM-NG-HEAP] backing_allocs=` face a `[MEM-NG-DEPOT] servis=` mesure
//! exactement ce que cela retire au verrou global.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use linked_list_allocator::LockedHeap;
use crate::arch::x86_64::smp;
use crate::kernel::magasin::{self, Depot, Magasin, LOT};
use x86_64::instructions::interrupts;

pub const BOOTSTRAP_SIZE: usize = 8 * 1024 * 1024;
const CLASS_SIZES: [usize; 6] = [32, 64, 128, 256, 512, 1024];
const CLASS_COUNT: usize = CLASS_SIZES.len();
const MAX_CACHED_PER_CLASS: usize = 64;

static mut HEAP_SPACE: [u8; BOOTSTRAP_SIZE] = [0; BOOTSTRAP_SIZE];
static HEAP_TOTAL: AtomicUsize = AtomicUsize::new(BOOTSTRAP_SIZE);
static CACHE_READY: AtomicBool = AtomicBool::new(false);

struct CacheClass {
    head: AtomicUsize,
    count: AtomicUsize,
}
impl CacheClass {
    const fn new() -> Self {
        Self { head: AtomicUsize::new(0), count: AtomicUsize::new(0) }
    }
}
struct CpuCache { classes: [CacheClass; CLASS_COUNT] }
impl CpuCache {
    const fn new() -> Self {
        Self { classes: [const { CacheClass::new() }; CLASS_COUNT] }
    }
}
static CACHES: [CpuCache; smp::MAX_CPUS] =
    [const { CpuCache::new() }; smp::MAX_CPUS];

/// Un depot par classe de taille. Partage par tous les CPU, pris pour la duree
/// d'un transfert de magasin -- donc O(1), interruptions masquees.
static DEPOTS: [Depot; CLASS_COUNT] = [const { Depot::neuf() }; CLASS_COUNT];

/// Allocations demandees alors que ce CPU etait dans un gestionnaire
/// d'interruption.
///
/// Ce n'est pas une faute en soi -- le reveil d'une file d'attente depuis
/// l'IRQ 8042 en fait --, mais c'est le chemin ou une descente dans le backing
/// global coute le plus cher, et il n'etait mesure par rien.
static ALLOCS_EN_IRQ: AtomicU64 = AtomicU64::new(0);
static BACKING_FREES: AtomicU64 = AtomicU64::new(0);

static CACHE_BYTES: AtomicUsize = AtomicUsize::new(0);
static DEPOT_HITS: AtomicU64 = AtomicU64::new(0);
static DEPOT_SPILLS: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static CACHE_RETURNS: AtomicU64 = AtomicU64::new(0);
static BACKING_ALLOCS: AtomicU64 = AtomicU64::new(0);

struct NgHeap { inner: LockedHeap }
impl NgHeap { const fn empty() -> Self { Self { inner: LockedHeap::empty() } } }

#[global_allocator]
static ALLOCATOR: NgHeap = NgHeap::empty();

fn class_for(layout: Layout) -> Option<(usize, usize)> {
    let need = layout.size().max(layout.align()).max(core::mem::size_of::<usize>());
    CLASS_SIZES.iter().copied().enumerate().find(|(_, size)| *size >= need)
}

fn cpu_index() -> usize { smp::cpu_index().min(smp::MAX_CPUS - 1) }

/// Compte les allocations demandees depuis un gestionnaire d'interruption.
///
/// Le noyau en fait -- le reveil d'une file d'attente depuis l'IRQ 8042 en
/// est une --, et les interdire serait une regle qu'on ne pourrait pas tenir.
/// Les MESURER est ce qui manquait : c'est le chemin ou une descente dans le
/// backing global coute le plus cher, et un compteur dit s'il est rare ou s'il
/// est le regime normal.
#[inline]
fn note_contexte_alloc() {
    let index = smp::cpu_index();
    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(index) {
        if crate::arch::x86_64::cpu_local::local(id).irq_depth() != 0 {
            ALLOCS_EN_IRQ.fetch_add(1, Ordering::Relaxed);
        }
    }
}

unsafe fn cache_pop(index: usize, size: usize) -> *mut u8 {
    interrupts::without_interrupts(|| {
        let cache = &CACHES[cpu_index()].classes[index];
        let head = cache.head.load(Ordering::Acquire);
        if head == 0 { return core::ptr::null_mut(); }
        let next = *(head as *const usize);
        cache.head.store(next, Ordering::Release);
        cache.count.fetch_sub(1, Ordering::Relaxed);
        CACHE_BYTES.fetch_sub(size, Ordering::Relaxed);
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        head as *mut u8
    })
}

unsafe fn cache_push(index: usize, size: usize, ptr: *mut u8) -> bool {
    interrupts::without_interrupts(|| {
        let cache = &CACHES[cpu_index()].classes[index];
        if cache.count.load(Ordering::Relaxed) >= MAX_CACHED_PER_CLASS {
            return false;
        }
        let head = cache.head.load(Ordering::Acquire);
        *(ptr as *mut usize) = head;
        cache.head.store(ptr as usize, Ordering::Release);
        cache.count.fetch_add(1, Ordering::Relaxed);
        CACHE_BYTES.fetch_add(size, Ordering::Relaxed);
        CACHE_RETURNS.fetch_add(1, Ordering::Relaxed);
        true
    })
}

/// Remplit la liste per-CPU vide avec un magasin entier du depot.
///
/// Rend un bloc pret a servir, ou nul si le depot n'avait rien : c'est
/// seulement alors qu'on descend dans le backing global.
unsafe fn recharge_depuis_depot(index: usize, size: usize) -> *mut u8 {
    interrupts::without_interrupts(|| {
        let Some(magasin) = DEPOTS[index].retire() else {
            return core::ptr::null_mut();
        };
        let cache = &CACHES[cpu_index()].classes[index];
        let servi = magasin.tete;
        let reste = magasin::lien_lit(servi);
        let restants = magasin.compte - 1;

        // La liste etait vide quand `cache_pop` a echoue -- mais elle a pu se
        // remplir depuis. `cache_pop` ne masque les interruptions QUE pour sa
        // propre section critique : entre son echec et ce rechargement, une
        // liberation depuis un gestionnaire d'interruption de ce CPU a pu
        // pousser des blocs ici. Ecraser la tete les perdrait -- une fuite
        // silencieuse, proportionnelle au trafic d'interruption.
        //
        // Le magasin se RACCORDE donc a ce qui est la. La marche jusqu'a sa
        // queue est bornee par `LOT`, sur des blocs qui viennent d'etre lus.
        let ancienne = cache.head.load(Ordering::Acquire);
        if ancienne != 0 && restants != 0 {
            let mut queue = reste;
            for _ in 1..restants {
                let suivant = magasin::lien_lit(queue);
                if suivant == 0 { break; }
                queue = suivant;
            }
            magasin::lien_ecrit(queue, ancienne);
            cache.head.store(reste, Ordering::Release);
            cache.count.fetch_add(restants, Ordering::Relaxed);
        } else if restants != 0 {
            cache.head.store(reste, Ordering::Release);
            cache.count.fetch_add(restants, Ordering::Relaxed);
        }
        CACHE_BYTES.fetch_add(size.saturating_mul(restants), Ordering::Relaxed);
        DEPOT_HITS.fetch_add(1, Ordering::Relaxed);
        servi as *mut u8
    })
}

/// Vide `LOT` blocs de la liste per-CPU vers le depot.
///
/// Rend `false` si le depot est plein : l'appelant rend alors les blocs au
/// backing, un par un et avec exactement la disposition qui les a alloues.
unsafe fn deverse_vers_depot(index: usize, size: usize) -> Option<Magasin> {
    interrupts::without_interrupts(|| {
        let cache = &CACHES[cpu_index()].classes[index];
        let tete = cache.head.load(Ordering::Acquire);
        if tete == 0 {
            return None;
        }
        let (magasin, reste) = magasin::detache(tete, LOT);
        if magasin.compte == 0 {
            return None;
        }
        cache.head.store(reste, Ordering::Release);
        cache
            .count
            .store(cache.count.load(Ordering::Relaxed).saturating_sub(magasin.compte), Ordering::Relaxed);
        CACHE_BYTES.fetch_sub(size.saturating_mul(magasin.compte), Ordering::Relaxed);
        if DEPOTS[index].depose(magasin) {
            DEPOT_SPILLS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        // Depot plein : ces blocs reviennent a l'appelant, qui les rendra au
        // backing. Ne jamais les perdre serait tentant a oublier : un magasin
        // detache et non replace est une fuite silencieuse.
        Some(magasin)
    })
}

unsafe impl GlobalAlloc for NgHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_contexte_alloc();
        if CACHE_READY.load(Ordering::Acquire) {
            if let Some((index, size)) = class_for(layout) {
                let cached = cache_pop(index, size);
                if !cached.is_null() { return cached; }
                CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
                // Le depot AVANT le backing : un magasin remplit la liste d'un
                // coup, et le verrou global n'est pris qu'a la premiere chauffe.
                let recharge = recharge_depuis_depot(index, size);
                if !recharge.is_null() { return recharge; }
                let backing = Layout::from_size_align_unchecked(size, size);
                BACKING_ALLOCS.fetch_add(1, Ordering::Relaxed);
                return GlobalAlloc::alloc(&self.inner, backing);
            }
        }
        BACKING_ALLOCS.fetch_add(1, Ordering::Relaxed);
        GlobalAlloc::alloc(&self.inner, layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if CACHE_READY.load(Ordering::Acquire) {
            if let Some((index, size)) = class_for(layout) {
                if cache_push(index, size, ptr) { return; }
                let backing = Layout::from_size_align_unchecked(size, size);
                // La liste deborde. On la vide d'un LOT vers le depot ; c'est
                // le seul chemin qui rend au cache la place d'accepter ce bloc
                // sans descendre dans le backing.
                let refus = deverse_vers_depot(index, size);
                if let Some(magasin) = refus {
                    // Depot plein : la memoire est reellement rendue. Chaque
                    // bloc repart avec EXACTEMENT la disposition qui l'a
                    // alloue -- c'est ce qui garde le contrat du backing
                    // intact.
                    let mut courant = magasin.tete;
                    while courant != 0 {
                        let suivant = magasin::lien_lit(courant);
                        BACKING_FREES.fetch_add(1, Ordering::Relaxed);
                        GlobalAlloc::dealloc(&self.inner, courant as *mut u8, backing);
                        courant = suivant;
                    }
                }
                if cache_push(index, size, ptr) { return; }
                BACKING_FREES.fetch_add(1, Ordering::Relaxed);
                GlobalAlloc::dealloc(&self.inner, ptr, backing);
                return;
            }
        }
        BACKING_FREES.fetch_add(1, Ordering::Relaxed);
        GlobalAlloc::dealloc(&self.inner, ptr, layout);
    }
}

pub fn init() {
    unsafe {
        ALLOCATOR.inner.lock().init(
            core::ptr::addr_of_mut!(HEAP_SPACE) as *mut u8,
            BOOTSTRAP_SIZE,
        );
    }
    crate::kernel::dmesg::log("heap-ng: bootstrap 8 MiB initialise");
}

/// Switch to the large physical arena. Must retain the historical boot invariant:
/// no persistent bootstrap allocation may exist when this is called.
pub unsafe fn switch_arena(start: *mut u8, size: usize) {
    CACHE_READY.store(false, Ordering::Release);
    ALLOCATOR.inner.lock().init(start, size);
    HEAP_TOTAL.store(size, Ordering::Release);
    CACHE_READY.store(true, Ordering::Release);
    crate::kernel::dmesg::log("heap-ng: arene physique active + caches per-CPU");
}

/// (used, free, total). Cached free blocks are reported as free, not as live use.
pub fn stats() -> (usize, usize, usize) {
    let heap = ALLOCATOR.inner.lock();
    let cached = CACHE_BYTES.load(Ordering::Relaxed);
    let total = HEAP_TOTAL.load(Ordering::Acquire);
    (heap.used().saturating_sub(cached), heap.free().saturating_add(cached), total)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NgStats {
    pub cached_bytes: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_returns: u64,
    pub backing_allocs: u64,
    pub backing_frees: u64,
    pub depot_hits: u64,
    pub depot_spills: u64,
    pub allocs_en_irq: u64,
}

pub fn ng_stats() -> NgStats {
    NgStats {
        cached_bytes: CACHE_BYTES.load(Ordering::Relaxed),
        cache_hits: CACHE_HITS.load(Ordering::Relaxed),
        cache_misses: CACHE_MISSES.load(Ordering::Relaxed),
        cache_returns: CACHE_RETURNS.load(Ordering::Relaxed),
        backing_allocs: BACKING_ALLOCS.load(Ordering::Relaxed),
        backing_frees: BACKING_FREES.load(Ordering::Relaxed),
        depot_hits: DEPOT_HITS.load(Ordering::Relaxed),
        depot_spills: DEPOT_SPILLS.load(Ordering::Relaxed),
        allocs_en_irq: ALLOCS_EN_IRQ.load(Ordering::Relaxed),
    }
}

pub fn log_ng_stats() {
    let s = ng_stats();
    crate::serial_println!(
        "[MEM-NG-HEAP] cached_bytes={} hits={} misses={} returns={} backing_allocs={} backing_frees={} depot_hits={} depot_spills={} allocs_en_irq={}",
        s.cached_bytes, s.cache_hits, s.cache_misses, s.cache_returns,
        s.backing_allocs, s.backing_frees, s.depot_hits, s.depot_spills,
        s.allocs_en_irq
    );
    for (index, taille) in CLASS_SIZES.iter().enumerate() {
        let d = DEPOTS[index].compteurs();
        // Une classe jamais sollicitee n'a rien a dire : la taire garde la
        // trace lisible sur un port serie.
        if d.servis == 0 && d.deposes == 0 && d.vides == 0 {
            continue;
        }
        crate::serial_println!(
            "[MEM-NG-DEPOT] classe={} magasins={} pic={} servis={} deposes={} vides={} pleins={}",
            taille, d.magasins, d.pic, d.servis, d.deposes, d.vides, d.pleins
        );
    }
}
