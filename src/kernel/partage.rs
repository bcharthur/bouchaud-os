//! SMP-safe lifetime and page coordination for file-backed `MAP_SHARED`.
//!
//! The registry lock protects only node lookup and reference counters. Page
//! allocation, backing reads, writeback, and frame release always happen after
//! that lock has been dropped.

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::sync::{SpinLock, WaitQueue};
use crate::kernel::vmm::{self, PAGE_SIZE};

#[derive(Clone, Copy)]
enum SharedPageState {
    Loading,
    Present(u64),
    Failed,
}

struct SharedPage {
    numero: u64,
    state: SpinLock<SharedPageState>,
    waiters: WaitQueue,
}

struct Partagee {
    node: usize,
    ouverts: usize,
    mappages: usize,
    pages: Vec<Arc<SharedPage>>,
}

static CACHE: SpinLock<Vec<Partagee>> = SpinLock::new(Vec::new());

fn entree_locked(cache: &mut Vec<Partagee>, node: usize) -> usize {
    if let Some(index) = cache.iter().position(|entry| entry.node == node) {
        return index;
    }
    cache.push(Partagee {
        node,
        ouverts: 0,
        mappages: 0,
        pages: Vec::new(),
    });
    cache.len() - 1
}

pub fn ouvre(node: usize) {
    let mut cache = CACHE.lock();
    let index = entree_locked(&mut cache, node);
    cache[index].ouverts += 1;
}

pub fn ferme(node: usize) {
    {
        let mut cache = CACHE.lock();
        if let Some(entry) = cache.iter_mut().find(|entry| entry.node == node) {
            entry.ouverts = entry.ouverts.saturating_sub(1);
        }
    }
    evince_si_orphelin(node);
}

pub fn mappe(node: usize) {
    let mut cache = CACHE.lock();
    let index = entree_locked(&mut cache, node);
    cache[index].mappages += 1;
}

pub fn demappe(node: usize) {
    {
        let mut cache = CACHE.lock();
        if let Some(entry) = cache.iter_mut().find(|entry| entry.node == node) {
            entry.mappages = entry.mappages.saturating_sub(1);
        }
    }
    evince_si_orphelin(node);
}

/// Return the unique physical frame for `(node, numero)`.
///
/// A registry miss publishes `Loading` before allocation and I/O. Contenders
/// clone the stable page object, drop every spin lock, and sleep on its queue.
pub fn page(node: usize, numero: u64) -> Option<u64> {
    let (page, loader) = {
        let mut cache = CACHE.lock();
        let index = entree_locked(&mut cache, node);
        if let Some(page) = cache[index]
            .pages
            .iter()
            .find(|page| page.numero == numero)
        {
            (Arc::clone(page), false)
        } else {
            let page = Arc::new(SharedPage {
                numero,
                state: SpinLock::new(SharedPageState::Loading),
                waiters: WaitQueue::new(),
            });
            cache[index].pages.push(Arc::clone(&page));
            (page, true)
        }
    };

    if loader {
        let result = vmm::alloc_frame().map(|frame| {
            let debut = (numero * PAGE_SIZE) as usize;
            let dst = crate::kernel::memory::phys_to_virt(frame);
            let bytes = unsafe {
                core::slice::from_raw_parts_mut(dst, PAGE_SIZE as usize)
            };
            let _ = crate::fs::backing::read_at(node, debut, bytes);
            frame
        });
        *page.state.lock() = match result {
            Some(frame) => SharedPageState::Present(frame),
            None => SharedPageState::Failed,
        };
        page.waiters.wake_all();
        return result;
    }

    loop {
        let ticket = page.waiters.ticket();
        match *page.state.lock() {
            SharedPageState::Present(frame) => return Some(frame),
            SharedPageState::Failed => return None,
            SharedPageState::Loading => page.waiters.wait(ticket),
        }
    }
}

fn present_pages(node: usize) -> Vec<(u64, u64)> {
    let cache = CACHE.lock();
    let Some(entry) = cache.iter().find(|entry| entry.node == node) else {
        return Vec::new();
    };
    entry
        .pages
        .iter()
        .filter_map(|page| match *page.state.lock() {
            SharedPageState::Present(frame) => Some((page.numero, frame)),
            _ => None,
        })
        .collect()
}

fn writeback_pages(node: usize, pages: &[(u64, u64)]) {
    if crate::fs::backing::is_disk_backed(node) || pages.is_empty() {
        return;
    }
    let fs = crate::fs::ramfs::fs();
    for &(numero, frame) in pages {
        let debut = (numero * PAGE_SIZE) as usize;
        if debut >= fs.nodes[node].content.len() {
            continue;
        }
        let fin = core::cmp::min(fs.nodes[node].content.len(), debut + PAGE_SIZE as usize);
        unsafe {
            core::ptr::copy_nonoverlapping(
                crate::kernel::memory::phys_to_virt(frame),
                fs.nodes[node].content[debut..fin].as_mut_ptr(),
                fin - debut,
            );
        }
    }
}

pub fn writeback(node: usize) {
    let pages = present_pages(node);
    writeback_pages(node, &pages);
}

pub fn writeback_tout() {
    let nodes: Vec<usize> = CACHE.lock().iter().map(|entry| entry.node).collect();
    for node in nodes {
        writeback(node);
    }
}

fn evince_si_orphelin(node: usize) {
    let removed = {
        let mut cache = CACHE.lock();
        let Some(index) = cache.iter().position(|entry| entry.node == node) else {
            return;
        };
        if cache[index].ouverts != 0 || cache[index].mappages != 0 {
            return;
        }
        // A caller still materialising a page will retry eviction when its
        // descriptor/mapping reference is eventually released.
        if cache[index].pages.iter().any(|page| {
            matches!(*page.state.lock(), SharedPageState::Loading)
        }) {
            return;
        }
        cache.swap_remove(index)
    };

    let anonyme = crate::fs::ramfs::fs().est_anonyme(node);
    let pages: Vec<(u64, u64)> = removed
        .pages
        .iter()
        .filter_map(|page| match *page.state.lock() {
            SharedPageState::Present(frame) => Some((page.numero, frame)),
            _ => None,
        })
        .collect();
    if !anonyme {
        writeback_pages(node, &pages);
    }
    for (_, frame) in pages {
        vmm::free_frame(frame);
    }
    if anonyme {
        crate::fs::ramfs::fs().libere_anonyme(node);
    }
}

pub fn statistiques() -> (usize, usize) {
    let cache = CACHE.lock();
    (cache.len(), cache.iter().map(|entry| entry.pages.len()).sum())
}

pub fn etat(node: usize) -> Option<(usize, usize, usize)> {
    CACHE
        .lock()
        .iter()
        .find(|entry| entry.node == node)
        .map(|entry| (entry.ouverts, entry.mappages, entry.pages.len()))
}
