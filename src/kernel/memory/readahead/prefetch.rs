// Seed the existing clean-page cache. Acquire+release retains a reclaimable page.
// Backing I/O is intentionally outside the process Mm lock.

fn prefetch_after(key: Key, pages: u64) {
    let step = crate::kernel::vmm::PAGE_SIZE;
    for n in 1..=pages.min(RA_MAX_PAGES) {
        let next = Key {
            node: key.node,
            offset: key.offset.saturating_add(step.saturating_mul(n)),
            generation: key.generation,
        };
        RA_REQUESTED.fetch_add(1, Ordering::Relaxed);
        if crate::kernel::clean_page_cache::acquire(next).is_some() {
            crate::kernel::clean_page_cache::release(next);
            RA_OK.fetch_add(1, Ordering::Relaxed);
        } else {
            RA_FAIL.fetch_add(1, Ordering::Relaxed);
            break;
        }
    }
}
