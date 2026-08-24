//! Physical page cache for immutable disk-backed, read-only mappings.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::{SpinLock, WaitQueue};
use crate::kernel::vmm::{self, PAGE_SIZE};

/// Maximum number of reclaimable (zero-mapping) pages. Live mapped entries are
/// not evictable and may exceed this number by design.
const MAX_RECLAIMABLE_PAGES: usize = 2048;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub node: usize,
    pub offset: u64,
    pub generation: u64,
}

#[derive(Clone, Copy)]
enum State { Loading, Present { frame: u64, mappings: usize }, Failed }

struct Entry { key: Key, state: SpinLock<State>, waiters: WaitQueue }
static CACHE: SpinLock<Vec<Arc<Entry>>> = SpinLock::new(Vec::new());
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static WAITS: AtomicU64 = AtomicU64::new(0);
static SHARED_MAPS: AtomicU64 = AtomicU64::new(0);

/// Acquire one mapping reference. Only immutable disk extents are eligible, so
/// their backing generation is the registered extent itself.
pub fn acquire(key: Key) -> Option<u64> {
    if crate::fs::backing::generation(key.node) != Some(key.generation)
        || key.offset % PAGE_SIZE != 0 {
        return None;
    }
    let (entry, loader, evicted) = {
        let mut cache = CACHE.lock();
        if let Some(entry) = cache.iter().find(|entry| entry.key == key) {
            let mut state = entry.state.lock();
            match *state {
                State::Present { frame, mappings } => {
                    *state = State::Present { frame, mappings: mappings.checked_add(1).expect("clean cache ref overflow") };
                    HITS.fetch_add(1, Ordering::Relaxed);
                    if mappings != 0 { SHARED_MAPS.fetch_add(1, Ordering::Relaxed); }
                    return Some(frame);
                }
                State::Failed => return None,
                State::Loading => (Arc::clone(entry), false, None),
            }
        } else {
            let evicted = if cache.len() >= MAX_RECLAIMABLE_PAGES {
                cache.iter().position(|entry| matches!(
                    *entry.state.lock(), State::Present { mappings: 0, .. } | State::Failed
                )).map(|index| cache.swap_remove(index))
            } else { None };
            let entry = Arc::new(Entry {
                key,
                state: SpinLock::new(State::Loading),
                waiters: WaitQueue::new(),
            });
            cache.push(Arc::clone(&entry));
            (entry, true, evicted)
        }
    };
    if let Some(old) = evicted {
        let frame = {
            let mut state = old.state.lock();
            match *state {
                State::Present { frame, mappings: 0 } => {
                    *state = State::Failed;
                    Some(frame)
                }
                _ => None,
            }
        };
        if let Some(frame) = frame {
            vmm::free_frame(frame);
        }
    }

    if loader {
        MISSES.fetch_add(1, Ordering::Relaxed);
        let result = vmm::alloc_frame().and_then(|frame| {
            let dst = crate::kernel::memory::phys_to_virt(frame);
            let bytes = unsafe { core::slice::from_raw_parts_mut(dst, PAGE_SIZE as usize) };
            let got = crate::fs::backing::read_at(key.node, key.offset as usize, bytes);
            if got == PAGE_SIZE as usize
                && crate::fs::backing::generation(key.node) == Some(key.generation) {
                Some(frame)
            } else {
                vmm::free_frame(frame);
                None
            }
        });
        *entry.state.lock() = match result {
            Some(frame) => State::Present { frame, mappings: 1 },
            None => State::Failed,
        };
        entry.waiters.wake_all();
        return result;
    }

    loop {
        let ticket = entry.waiters.ticket();
        let mut state = entry.state.lock();
        match *state {
            State::Present { frame, mappings } => {
                *state = State::Present { frame, mappings: mappings.checked_add(1).expect("clean cache ref overflow") };
                HITS.fetch_add(1, Ordering::Relaxed);
                if mappings != 0 { SHARED_MAPS.fetch_add(1, Ordering::Relaxed); }
                return Some(frame);
            }
            State::Failed => return None,
            State::Loading => {
                drop(state);
                WAITS.fetch_add(1, Ordering::Relaxed);
                entry.waiters.wait(ticket);
            }
        }
    }
}

pub fn retain(key: Key) -> bool {
    let cache = CACHE.lock();
    let Some(entry) = cache.iter().find(|entry| entry.key == key) else { return false; };
    let mut state = entry.state.lock();
    if let State::Present { frame, mappings } = *state {
        *state = State::Present { frame, mappings: mappings.checked_add(1).expect("clean cache ref overflow") };
        true
    } else { false }
}

pub fn release(key: Key) {
    let entry = CACHE.lock().iter().find(|entry| entry.key == key).cloned();
    let Some(entry) = entry else {
        panic!("clean page cache: release of unregistered key");
    };
    let mut state = entry.state.lock();
    if let State::Present { frame, mappings } = *state {
        assert!(mappings != 0, "clean page cache: double release");
        *state = State::Present { frame, mappings: mappings - 1 };
    } else {
        panic!("clean page cache: release of non-present entry");
    }
    drop(state);
    drop(entry);
    reclaim_excess();
}

fn reclaim_excess() {
    loop {
        let frame = {
            let mut cache = CACHE.lock();
            let reclaimable = cache.iter().filter(|entry| matches!(
                *entry.state.lock(), State::Present { mappings: 0, .. } | State::Failed
            )).count();
            if reclaimable <= MAX_RECLAIMABLE_PAGES {
                return;
            }
            let Some(index) = cache.iter().position(|entry| matches!(
                *entry.state.lock(), State::Present { mappings: 0, .. } | State::Failed
            )) else { return; };
            let entry = cache.swap_remove(index);
            let mut state = entry.state.lock();
            match *state {
                State::Present { frame, mappings: 0 } => {
                    *state = State::Failed;
                    Some(frame)
                }
                State::Failed => None,
                _ => unreachable!(),
            }
        };
        if let Some(frame) = frame {
            vmm::free_frame(frame);
        }
    }
}

pub fn stats() -> (u64, u64, u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed),
     WAITS.load(Ordering::Relaxed), SHARED_MAPS.load(Ordering::Relaxed))
}
