//! Memory-pressure policy for P0-NG1.
//!
//! The policy is intentionally deterministic: it uses free-frame ratios,
//! drains bounded per-CPU page caches first, then asks the clean page cache to
//! reclaim reusable pages. It never kills a process while holding a kernel
//! lock. OOM is reported as a final event after reclaim was attempted.

use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level { Normal, Low, Critical }

static RECLAIM_RUNS: AtomicU64 = AtomicU64::new(0);
static RECLAIMED_PAGES: AtomicU64 = AtomicU64::new(0);
static ALLOC_FAILURES: AtomicU64 = AtomicU64::new(0);
static OOM_EVENTS: AtomicU64 = AtomicU64::new(0);

pub fn level() -> Level {
    let (used, total) = crate::kernel::vmm::frame_stats_relaxed();
    if total == 0 { return Level::Normal; }
    let free = total.saturating_sub(used);
    let pct = free.saturating_mul(100) / total;
    if pct <= 5 { Level::Critical }
    else if pct <= 15 { Level::Low }
    else { Level::Normal }
}

pub fn note_allocation_failure() {
    ALLOC_FAILURES.fetch_add(1, Ordering::Relaxed);
}

pub fn reclaim_budget() -> usize {
    match level() {
        Level::Normal => 0,
        Level::Low => 256,
        Level::Critical => 2048,
    }
}

/// Bounded synchronous reclaim. Call only outside BKL/ranked critical sections.
pub fn reclaim_now(limit: usize) -> usize {
    if limit == 0 { return 0; }
    debug_assert!(!crate::kernel::smp_lock::held_by_current_cpu());
    debug_assert_eq!(crate::kernel::sync::lockdep::depth(), 0);
    RECLAIM_RUNS.fetch_add(1, Ordering::Relaxed);
    let from_local = crate::kernel::frame_cache::drain(limit);
    let remaining = limit.saturating_sub(from_local);
    let from_clean = if remaining == 0 { 0 }
        else { crate::kernel::clean_page_cache::reclaim_pages(remaining) };
    let total = from_local + from_clean;
    RECLAIMED_PAGES.fetch_add(total as u64, Ordering::Relaxed);
    total
}

pub fn recover_allocation() -> bool {
    note_allocation_failure();
    let budget = reclaim_budget().max(64);
    reclaim_now(budget) != 0
}

pub fn note_oom() { OOM_EVENTS.fetch_add(1, Ordering::Relaxed); }

pub fn log_stats() {
    let level_name = match level() {
        Level::Normal => "normal", Level::Low => "low", Level::Critical => "critical"
    };
    crate::serial_println!(
        "[MEM-NG-PRESSURE] level={} reclaim_runs={} reclaimed_pages={} alloc_failures={} oom_events={}",
        level_name,
        RECLAIM_RUNS.load(Ordering::Relaxed),
        RECLAIMED_PAGES.load(Ordering::Relaxed),
        ALLOC_FAILURES.load(Ordering::Relaxed),
        OOM_EVENTS.load(Ordering::Relaxed)
    );
}
