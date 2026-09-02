//! Physical page cache for immutable disk-backed, read-only mappings.
//!
//! P0-NG1 keeps the proven indexed cache and adds two performance properties:
//! clean-page allocation/free uses the bounded per-CPU frame cache, and reclaim
//! becomes pressure-aware. Cache state is never discarded without revalidating
//! `mappings == 0` under the entry lock.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::kernel::sync::{SpinLock, WaitQueue};
use crate::kernel::vmm::PAGE_SIZE;

const MAX_RECLAIMABLE_PAGES: usize = 16_384;
const LOW_PRESSURE_TARGET: usize = 4_096;
const CRITICAL_PRESSURE_TARGET: usize = 1_024;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub node: usize,
    pub offset: u64,
    pub generation: u64,
}

#[derive(Clone, Copy)]
enum State { Loading, Present { frame: u64, mappings: usize }, Failed }
struct Entry { key: Key, state: SpinLock<State>, waiters: WaitQueue }

struct Cache {
    entrees: BTreeMap<Key, Arc<Entry>>,
    candidats: VecDeque<Key>,
}
const LIMITE_CANDIDATS: usize = 4 * MAX_RECLAIMABLE_PAGES;
impl Cache {
    fn propose(&mut self, key: Key) {
        if self.candidats.len() < LIMITE_CANDIDATS { self.candidats.push_back(key); }
    }
}

static CACHE: SpinLock<Cache> = SpinLock::new(Cache {
    entrees: BTreeMap::new(), candidats: VecDeque::new(),
});
static RECUPERABLES: AtomicUsize = AtomicUsize::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static WAITS: AtomicU64 = AtomicU64::new(0);
static SHARED_MAPS: AtomicU64 = AtomicU64::new(0);
static RECLAIMED: AtomicU64 = AtomicU64::new(0);

#[inline]
fn devient_recuperable() { RECUPERABLES.fetch_add(1, Ordering::Relaxed); }
#[inline]
fn cesse_d_etre_recuperable() {
    let _ = RECUPERABLES.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some(v.saturating_sub(1)));
}
fn recuperable(etat: &State) -> bool {
    matches!(etat, State::Present { mappings: 0, .. } | State::Failed)
}

fn allocate_clean_frame() -> Option<u64> {
    if let Some(frame) = crate::kernel::frame_cache::alloc_frame() { return Some(frame); }
    if crate::kernel::memory_pressure::recover_allocation() {
        if let Some(frame) = crate::kernel::frame_cache::alloc_frame() { return Some(frame); }
    }
    crate::kernel::memory_pressure::note_oom();
    None
}

pub fn acquire(key: Key) -> Option<u64> {
    if crate::fs::backing::generation(key.node) != Some(key.generation)
        || key.offset % PAGE_SIZE != 0
    { return None; }

    let (entry, loader, evicted) = {
        let mut cache = CACHE.lock();
        if let Some(entry) = cache.entrees.get(&key) {
            let entry = Arc::clone(entry);
            let mut state = entry.state.lock();
            match *state {
                State::Present { frame, mappings } => {
                    *state = State::Present {
                        frame,
                        mappings: mappings.checked_add(1).expect("clean cache ref overflow"),
                    };
                    HITS.fetch_add(1, Ordering::Relaxed);
                    if mappings == 0 { cesse_d_etre_recuperable(); }
                    else { SHARED_MAPS.fetch_add(1, Ordering::Relaxed); }
                    return Some(frame);
                }
                State::Failed => return None,
                State::Loading => { drop(state); (entry, false, None) }
            }
        } else {
            let evicted = if cache.entrees.len() >= MAX_RECLAIMABLE_PAGES {
                retire_un_candidat(&mut cache)
            } else { None };
            let entry = Arc::new(Entry {
                key, state: SpinLock::new(State::Loading), waiters: WaitQueue::new(),
            });
            cache.entrees.insert(key, Arc::clone(&entry));
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
            crate::kernel::frame_cache::free_frame(frame);
            RECLAIMED.fetch_add(1, Ordering::Relaxed);
        }
    }

    if loader {
        MISSES.fetch_add(1, Ordering::Relaxed);
        let result = allocate_clean_frame().and_then(|frame| {
            let dst = crate::kernel::memory::phys_to_virt(frame);
            let bytes = unsafe { core::slice::from_raw_parts_mut(dst, PAGE_SIZE as usize) };
            let got = crate::fs::backing::read_at(key.node, key.offset as usize, bytes);
            if got == PAGE_SIZE as usize
                && crate::fs::backing::generation(key.node) == Some(key.generation)
            {
                Some(frame)
            } else {
                crate::kernel::frame_cache::free_frame(frame);
                None
            }
        });
        let mut etat = entry.state.lock();
        *etat = match result {
            Some(frame) => State::Present { frame, mappings: 1 },
            None => State::Failed,
        };
        drop(etat);
        if result.is_none() {
            devient_recuperable();
            CACHE.lock().propose(key);
        }
        entry.waiters.wake_all();
        return result;
    }

    loop {
        let ticket = entry.waiters.ticket();
        let mut state = entry.state.lock();
        match *state {
            State::Present { frame, mappings } => {
                *state = State::Present {
                    frame,
                    mappings: mappings.checked_add(1).expect("clean cache ref overflow"),
                };
                HITS.fetch_add(1, Ordering::Relaxed);
                if mappings == 0 { cesse_d_etre_recuperable(); }
                else { SHARED_MAPS.fetch_add(1, Ordering::Relaxed); }
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
    let Some(entry) = cache.entrees.get(&key) else { return false; };
    let mut state = entry.state.lock();
    if let State::Present { frame, mappings } = *state {
        *state = State::Present {
            frame,
            mappings: mappings.checked_add(1).expect("clean cache ref overflow"),
        };
        if mappings == 0 { cesse_d_etre_recuperable(); }
        true
    } else { false }
}

pub fn release(key: Key) {
    let entry = CACHE.lock().entrees.get(&key).cloned();
    let Some(entry) = entry else { panic!("clean page cache: release of unregistered key"); };
    let mut etat = entry.state.lock();
    let devenue_libre = if let State::Present { frame, mappings } = *etat {
        assert!(mappings != 0, "clean page cache: double release");
        *etat = State::Present { frame, mappings: mappings - 1 };
        mappings == 1
    } else { panic!("clean page cache: release of non-present entry"); };
    drop(etat);
    drop(entry);
    if devenue_libre {
        devient_recuperable();
        CACHE.lock().propose(key);
    }
    reclaim_excess();
}

fn retire_un_candidat(cache: &mut Cache) -> Option<Arc<Entry>> {
    while let Some(key) = cache.candidats.pop_front() {
        let sortable = match cache.entrees.get(&key) {
            Some(entry) => recuperable(&entry.state.lock()),
            None => false,
        };
        if sortable {
            cesse_d_etre_recuperable();
            return cache.entrees.remove(&key);
        }
    }
    let victime = cache.entrees.iter()
        .find(|(_, entry)| recuperable(&entry.state.lock()))
        .map(|(key, _)| *key)?;
    cesse_d_etre_recuperable();
    cache.entrees.remove(&victime)
}

fn pressure_target() -> usize {
    match crate::kernel::memory_pressure::level() {
        crate::kernel::memory_pressure::Level::Normal => MAX_RECLAIMABLE_PAGES,
        crate::kernel::memory_pressure::Level::Low => LOW_PRESSURE_TARGET,
        crate::kernel::memory_pressure::Level::Critical => CRITICAL_PRESSURE_TARGET,
    }
}

fn take_reclaimable() -> Option<Option<u64>> {
    let entry = {
        let mut cache = CACHE.lock();
        retire_un_candidat(&mut cache)?
    };
    let mut state = entry.state.lock();
    match *state {
        State::Present { frame, mappings: 0 } => {
            *state = State::Failed;
            Some(Some(frame))
        }
        State::Failed => Some(None),
        _ => Some(None),
    }
}

fn reclaim_excess() {
    let target = pressure_target();
    while RECUPERABLES.load(Ordering::Relaxed) > target {
        match take_reclaimable() {
            Some(Some(frame)) => {
                crate::kernel::frame_cache::free_frame(frame);
                RECLAIMED.fetch_add(1, Ordering::Relaxed);
            }
            Some(None) => continue,
            None => return,
        }
    }
}

/// Pressure reclaim API: pages go directly to the global VMM, bypassing local
/// caches so this operation really increases globally available RAM.
pub fn reclaim_pages(limit: usize) -> usize {
    let mut freed = 0usize;
    while freed < limit {
        match take_reclaimable() {
            Some(Some(frame)) => {
                crate::kernel::frame_cache::free_frame_global(frame);
                freed += 1;
            }
            Some(None) => continue,
            None => break,
        }
    }
    if freed != 0 { RECLAIMED.fetch_add(freed as u64, Ordering::Relaxed); }
    freed
}

pub fn stats() -> (u64, u64, u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed),
     WAITS.load(Ordering::Relaxed), SHARED_MAPS.load(Ordering::Relaxed))
}

pub fn lifetime_stats() -> (usize, usize) {
    let cache = CACHE.lock();
    let reclaimable = cache.entrees.values().filter(|entry| recuperable(&entry.state.lock())).count();
    (cache.entrees.len(), reclaimable)
}

pub fn indice_recuperables() -> usize { RECUPERABLES.load(Ordering::Relaxed) }
pub fn reclaimed_pages() -> u64 { RECLAIMED.load(Ordering::Relaxed) }

pub fn log_ng_stats() {
    // Reporting must stay lock-free: browser_report can run while compatibility
    // code still owns the BKL. Taking CACHE here would turn observability into
    // another lock-order edge. RECUPERABLES and RECLAIMED are sufficient to
    // prove that the pressure/reclaim path is active.
    crate::serial_println!(
        "[MEM-NG-PAGECACHE] reclaimable_index={} reclaimed={}",
        indice_recuperables(), reclaimed_pages()
    );
}
