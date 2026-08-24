//! Physical page cache for immutable disk-backed, read-only mappings.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::{SpinLock, WaitQueue};
use crate::kernel::vmm::{self, PAGE_SIZE};

const MAX_PAGES: usize = 2048;

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
                    *state = State::Present { frame, mappings: mappings + 1 };
                    HITS.fetch_add(1, Ordering::Relaxed);
                    if mappings != 0 { SHARED_MAPS.fetch_add(1, Ordering::Relaxed); }
                    return Some(frame);
                }
                State::Failed => return None,
                State::Loading => (Arc::clone(entry), false, None),
            }
        } else {
            let evicted = if cache.len() >= MAX_PAGES {
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
        if let State::Present { frame, mappings: 0 } = *old.state.lock() {
            vmm::free_frame(frame);
        }
    }

    if loader {
        MISSES.fetch_add(1, Ordering::Relaxed);
        let result = vmm::alloc_frame().and_then(|frame| {
            let dst = crate::kernel::memory::phys_to_virt(frame);
            let bytes = unsafe { core::slice::from_raw_parts_mut(dst, PAGE_SIZE as usize) };
            let got = crate::fs::backing::read_at(key.node, key.offset as usize, bytes);
            if got == 0 { vmm::free_frame(frame); None } else { Some(frame) }
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
                *state = State::Present { frame, mappings: mappings + 1 };
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
        *state = State::Present { frame, mappings: mappings + 1 };
        true
    } else { false }
}

pub fn release(key: Key) {
    let entry = CACHE.lock().iter().find(|entry| entry.key == key).cloned();
    let Some(entry) = entry else { return; };
    let mut state = entry.state.lock();
    if let State::Present { frame, mappings } = *state {
        *state = State::Present { frame, mappings: mappings.saturating_sub(1) };
    }
}

pub fn stats() -> (u64, u64, u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed),
     WAITS.load(Ordering::Relaxed), SHARED_MAPS.load(Ordering::Relaxed))
}
