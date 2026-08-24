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
    evicting: bool,
    lifecycle: Arc<WaitQueue>,
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
        evicting: false,
        lifecycle: Arc::new(WaitQueue::new()),
    });
    cache.len() - 1
}

pub fn ouvre(node: usize) {
    loop {
        let wait = {
            let mut cache = CACHE.lock();
            let index = entree_locked(&mut cache, node);
            if cache[index].evicting {
                Some((Arc::clone(&cache[index].lifecycle), cache[index].lifecycle.ticket()))
            } else {
                cache[index].ouverts += 1;
                None
            }
        };
        match wait { Some((queue, ticket)) => queue.wait(ticket), None => return }
    }
}

pub fn ferme(node: usize) {
    {
        let mut cache = CACHE.lock();
        let entry = cache.iter_mut().find(|entry| entry.node == node)
            .expect("shared cache: close of unknown node");
        assert!(entry.ouverts != 0, "shared cache: descriptor ref underflow");
        entry.ouverts -= 1;
    }
    evince_si_orphelin(node);
}

pub fn mappe(node: usize) {
    loop {
        let wait = {
            let mut cache = CACHE.lock();
            let index = entree_locked(&mut cache, node);
            if cache[index].evicting {
                Some((Arc::clone(&cache[index].lifecycle), cache[index].lifecycle.ticket()))
            } else {
                cache[index].mappages += 1;
                None
            }
        };
        match wait { Some((queue, ticket)) => queue.wait(ticket), None => return }
    }
}

pub fn demappe(node: usize) {
    {
        let mut cache = CACHE.lock();
        let entry = cache.iter_mut().find(|entry| entry.node == node)
            .expect("shared cache: unmap of unknown node");
        assert!(entry.mappages != 0, "shared cache: mapping ref underflow");
        entry.mappages -= 1;
    }
    evince_si_orphelin(node);
}

/// Return the unique physical frame for `(node, numero)`.
///
/// A registry miss publishes `Loading` before allocation and I/O. Contenders
/// clone the stable page object, drop every spin lock, and sleep on its queue.
pub fn page(node: usize, numero: u64) -> Option<u64> {
    let (page, loader) = loop {
        let result = {
            let mut cache = CACHE.lock();
            let index = entree_locked(&mut cache, node);
            if cache[index].evicting {
                Err((Arc::clone(&cache[index].lifecycle), cache[index].lifecycle.ticket()))
            } else if let Some(page) = cache[index]
                .pages.iter().find(|page| page.numero == numero)
            {
                Ok((Arc::clone(page), false))
            } else {
                let page = Arc::new(SharedPage {
                    numero,
                    state: SpinLock::new(SharedPageState::Loading),
                    waiters: WaitQueue::new(),
                });
                cache[index].pages.push(Arc::clone(&page));
                Ok((page, true))
            }
        };
        match result {
            Ok(result) => break result,
            Err((queue, ticket)) => queue.wait(ticket),
        }
    };

    if loader {
        let result = vmm::alloc_frame().and_then(|frame| {
            let debut = (numero * PAGE_SIZE) as usize;
            let dst = crate::kernel::memory::phys_to_virt(frame);
            let bytes = unsafe {
                core::slice::from_raw_parts_mut(dst, PAGE_SIZE as usize)
            };
            // alloc_frame() zeroes the complete page. A tail page therefore
            // has explicit zero-fill semantics, but a short read before EOF is
            // an I/O failure and must never be published as Present.
            let logical = crate::fs::backing::logical_len(node);
            let expected = logical.saturating_sub(debut).min(PAGE_SIZE as usize);
            let got = crate::fs::backing::read_at(node, debut, &mut bytes[..expected]);
            if got == expected {
                Some(frame)
            } else {
                vmm::free_frame(frame);
                None
            }
        });
        *page.state.lock() = match result {
            Some(frame) => SharedPageState::Present(frame),
            None => SharedPageState::Failed,
        };
        page.waiters.wake_all();
        // The last mapping/descriptor may have disappeared while this loader
        // was outside CACHE doing I/O. Re-run orphan collection now; otherwise
        // no later lifecycle event is guaranteed to reclaim the frame.
        evince_si_orphelin(node);
        return result;
    }

    loop {
        let ticket = page.waiters.ticket();
        let state = page.state.lock();
        match *state {
            SharedPageState::Present(frame) => return Some(frame),
            SharedPageState::Failed => return None,
            SharedPageState::Loading => {
                drop(state);
                page.waiters.wait(ticket);
            }
        }
    }
}

fn present_pages(node: usize) -> Vec<(u64, u64)> {
    loop {
        let snapshot = {
            let cache = CACHE.lock();
            let Some(entry) = cache.iter().find(|entry| entry.node == node) else {
                return Vec::new();
            };
            if entry.evicting {
                Err((Arc::clone(&entry.lifecycle), entry.lifecycle.ticket()))
            } else {
                Ok(entry.pages.clone())
            }
        };
        match snapshot {
            Ok(pages) => return pages.iter().filter_map(|page| {
                let state = page.state.lock();
                match *state {
                    SharedPageState::Present(frame) => Some((page.numero, frame)),
                    _ => None,
                }
            }).collect(),
            Err((queue, ticket)) => queue.wait(ticket),
        }
    }
}

fn writeback_pages(node: usize, pages: &[(u64, u64)]) {
    if crate::fs::backing::is_disk_backed(node) || pages.is_empty() {
        return;
    }
    let _kernel = crate::kernel::smp_lock::enter();
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
    // Pin the node with a descriptor-style lifecycle reference so eviction
    // cannot free a frame after present_pages() snapshots it but before the
    // physical copy completes.
    ouvre(node);
    let pages = present_pages(node);
    writeback_pages(node, &pages);
    ferme(node);
}

pub fn writeback_tout() {
    let nodes: Vec<usize> = CACHE.lock().iter().map(|entry| entry.node).collect();
    for node in nodes {
        writeback(node);
    }
}

fn evince_si_orphelin(node: usize) {
    let (lifecycle, pages) = {
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
        cache[index].evicting = true;
        let lifecycle = Arc::clone(&cache[index].lifecycle);
        let pages = core::mem::take(&mut cache[index].pages);
        (lifecycle, pages)
    };

    let anonyme = {
        let _kernel = crate::kernel::smp_lock::enter();
        crate::fs::ramfs::fs().est_anonyme(node)
    };
    let pages: Vec<(u64, u64)> = pages
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
        let _kernel = crate::kernel::smp_lock::enter();
        crate::fs::ramfs::fs().libere_anonyme(node);
    }
    {
        let mut cache = CACHE.lock();
        if let Some(index) = cache.iter().position(|entry| {
            entry.node == node && Arc::ptr_eq(&entry.lifecycle, &lifecycle)
        }) {
            debug_assert!(cache[index].evicting);
            cache.swap_remove(index);
        }
    }
    lifecycle.wake_all();
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
