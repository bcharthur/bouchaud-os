//! Small per-CPU cache of clean physical pages for demand-paging hot paths.
//!
//! It deliberately sits in front of the proven VMM allocator instead of
//! replacing it. The global allocator remains the source of truth; each CPU can
//! retain a bounded number of pages to avoid global frame-lock traffic during
//! bursty Ladybird page-cache activity.

use crate::arch::x86_64::smp;
use crate::kernel::sync::SpinLockIrq;
use core::sync::atomic::{AtomicU64, Ordering};

pub use crate::kernel::vmm::PAGE_SIZE;
const LOCAL_CAPACITY: usize = 32;

struct LocalCache {
    pages: [u64; LOCAL_CAPACITY],
    len: usize,
}

impl LocalCache {
    const fn new() -> Self { Self { pages: [0; LOCAL_CAPACITY], len: 0 } }
    fn pop(&mut self) -> Option<u64> {
        if self.len == 0 { None } else {
            self.len -= 1;
            Some(self.pages[self.len])
        }
    }
    fn push(&mut self, page: u64) -> bool {
        if self.len == LOCAL_CAPACITY { return false; }
        self.pages[self.len] = page;
        self.len += 1;
        true
    }
}

static LOCAL: [SpinLockIrq<LocalCache>; smp::MAX_CPUS] =
    [const { SpinLockIrq::new(LocalCache::new()) }; smp::MAX_CPUS];
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static RETURNS: AtomicU64 = AtomicU64::new(0);
static DRAINS: AtomicU64 = AtomicU64::new(0);
static CACHED: AtomicU64 = AtomicU64::new(0);

fn cpu() -> usize { smp::cpu_index().min(smp::MAX_CPUS - 1) }

pub fn alloc_frame() -> Option<u64> {
    if let Some(page) = LOCAL[cpu()].lock().pop() {
        CACHED.fetch_sub(1, Ordering::Relaxed);
        HITS.fetch_add(1, Ordering::Relaxed);
        unsafe {
            core::ptr::write_bytes(
                crate::kernel::memory::phys_to_virt(page), 0, PAGE_SIZE as usize
            );
        }
        return Some(page);
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    crate::kernel::vmm::alloc_frame()
}

pub fn free_frame(page: u64) {
    if LOCAL[cpu()].lock().push(page & !(PAGE_SIZE - 1)) {
        CACHED.fetch_add(1, Ordering::Relaxed);
        RETURNS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    crate::kernel::vmm::free_frame(page);
}

/// Bypass local caches when reclaim must make RAM globally available.
pub fn free_frame_global(page: u64) {
    crate::kernel::vmm::free_frame(page);
}

/// Return cached pages to the global allocator under memory pressure.
pub fn drain(limit: usize) -> usize {
    let mut drained = 0usize;
    for c in 0..smp::MAX_CPUS {
        loop {
            if drained >= limit { break; }
            let page = LOCAL[c].lock().pop();
            match page {
                Some(page) => {
                    CACHED.fetch_sub(1, Ordering::Relaxed);
                    crate::kernel::vmm::free_frame(page);
                    drained += 1;
                }
                None => break,
            }
        }
        if drained >= limit { break; }
    }
    if drained != 0 { DRAINS.fetch_add(drained as u64, Ordering::Relaxed); }
    drained
}

pub fn cached_pages() -> usize {
    CACHED.load(Ordering::Relaxed) as usize
}

pub fn log_stats() {
    crate::serial_println!(
        "[MEM-NG-FRAMECACHE] cached={} hits={} misses={} returns={} drained={}",
        cached_pages(), HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed),
        RETURNS.load(Ordering::Relaxed), DRAINS.load(Ordering::Relaxed)
    );
}
