//! Monotonic milestone markers for Ladybird startup profiling.

use core::sync::atomic::{AtomicU64, Ordering};

static BROWSER_CLICK_NS: AtomicU64 = AtomicU64::new(0);
static FIRST_PAINT_NS: AtomicU64 = AtomicU64::new(0);

pub fn browser_click() {
    let now = crate::kernel::timer::monotonic_ns();
    FIRST_PAINT_NS.store(0, Ordering::Release);
    BROWSER_CLICK_NS.store(now, Ordering::Release);
    emit("PERF_BROWSER_CLICK", now);
}

pub fn exec_start(name: &str) {
    let now = crate::kernel::timer::monotonic_ns();
    crate::kernel::dmesg::log_fmt(format_args!(
        "PERF_EXEC_START t_ns={} since_click_ms={} image={}",
        now,
        since_click_ms(now),
        name,
    ));
}

pub fn first_paint() {
    let now = crate::kernel::timer::monotonic_ns();
    if FIRST_PAINT_NS
        .compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        emit("PERF_FIRST_PAINT", now);
    }
}

fn emit(marker: &str, now: u64) {
    crate::kernel::dmesg::log_fmt(format_args!(
        "{} t_ns={} since_click_ms={}", marker, now, since_click_ms(now),
    ));
}

fn since_click_ms(now: u64) -> u64 {
    let click = BROWSER_CLICK_NS.load(Ordering::Acquire);
    if click == 0 { 0 } else { now.saturating_sub(click) / 1_000_000 }
}
