//! P0-NG1 kernel heap: global backing arena + bounded per-CPU size caches.
//!
//! The old `LockedHeap` remains the proven backing allocator, but the hot path
//! for small kernel objects no longer takes its global lock on every alloc/free.
//! Six fixed size classes keep at most a small, bounded amount of memory per CPU.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use linked_list_allocator::LockedHeap;
use crate::arch::x86_64::smp;
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

static CACHE_BYTES: AtomicUsize = AtomicUsize::new(0);
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

unsafe impl GlobalAlloc for NgHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if CACHE_READY.load(Ordering::Acquire) {
            if let Some((index, size)) = class_for(layout) {
                let cached = cache_pop(index, size);
                if !cached.is_null() { return cached; }
                CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
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
                GlobalAlloc::dealloc(&self.inner, ptr, backing);
                return;
            }
        }
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
}

pub fn ng_stats() -> NgStats {
    NgStats {
        cached_bytes: CACHE_BYTES.load(Ordering::Relaxed),
        cache_hits: CACHE_HITS.load(Ordering::Relaxed),
        cache_misses: CACHE_MISSES.load(Ordering::Relaxed),
        cache_returns: CACHE_RETURNS.load(Ordering::Relaxed),
        backing_allocs: BACKING_ALLOCS.load(Ordering::Relaxed),
    }
}

pub fn log_ng_stats() {
    let s = ng_stats();
    crate::serial_println!(
        "[MEM-NG-HEAP] cached_bytes={} hits={} misses={} returns={} backing_allocs={}",
        s.cached_bytes, s.cache_hits, s.cache_misses, s.cache_returns, s.backing_allocs
    );
}
