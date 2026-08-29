// Detect sequential clean-page consumption independently on each logical CPU.

pub fn observe_clean(key: Key) {
    RA_OBSERVE.fetch_add(1, Ordering::Relaxed);
    let cpu = crate::arch::x86_64::usermode::cpu_index().min(RA_CPUS - 1);
    let page = crate::kernel::vmm::PAGE_SIZE;
    let old_node = LAST_NODE[cpu].swap(key.node, Ordering::Relaxed);
    let old_offset = LAST_OFFSET[cpu].swap(key.offset, Ordering::Relaxed);
    let sequential = old_node == key.node && old_offset != u64::MAX
        && key.offset == old_offset.saturating_add(page);
    let run = if sequential {
        RUN[cpu].fetch_add(1, Ordering::Relaxed) + 1
    } else {
        RUN[cpu].swap(1, Ordering::Relaxed);
        1
    };
    if sequential { RA_SEQUENTIAL.fetch_add(1, Ordering::Relaxed); }
    let window = ra_window(run);
    if window == 0 { return; }
    RA_MAX_WINDOW_SEEN.fetch_max(window, Ordering::Relaxed);
    match window {
        2 => { RA_WINDOW_2.fetch_add(1, Ordering::Relaxed); }
        4 => { RA_WINDOW_4.fetch_add(1, Ordering::Relaxed); }
        8 => { RA_WINDOW_8.fetch_add(1, Ordering::Relaxed); }
        16 => { RA_WINDOW_16.fetch_add(1, Ordering::Relaxed); }
        _ => {}
    }
    prefetch_after(key, window);
}
