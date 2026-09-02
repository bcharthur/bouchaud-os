//! Wake/ready-to-run latency observatory for the SMP scheduler.

use core::sync::atomic::{AtomicU64, Ordering};

static COUNT: AtomicU64 = AtomicU64::new(0);
static SUM_NS: AtomicU64 = AtomicU64::new(0);
static MAX_NS: AtomicU64 = AtomicU64::new(0);
static INTERACTIVE_COUNT: AtomicU64 = AtomicU64::new(0);
static INTERACTIVE_MAX_NS: AtomicU64 = AtomicU64::new(0);
static B_LT_100US: AtomicU64 = AtomicU64::new(0);
static B_LT_500US: AtomicU64 = AtomicU64::new(0);
static B_LT_2MS: AtomicU64 = AtomicU64::new(0);
static B_LT_8MS: AtomicU64 = AtomicU64::new(0);
static B_LT_16MS: AtomicU64 = AtomicU64::new(0);
static B_GE_16MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub count: u64,
    pub average_ns: u64,
    pub max_ns: u64,
    pub interactive_count: u64,
    pub interactive_max_ns: u64,
    pub buckets: [u64; 6],
}

pub fn record(ns: u64, interactive: bool) {
    COUNT.fetch_add(1, Ordering::Relaxed);
    SUM_NS.fetch_add(ns, Ordering::Relaxed);
    MAX_NS.fetch_max(ns, Ordering::Relaxed);
    if interactive {
        INTERACTIVE_COUNT.fetch_add(1, Ordering::Relaxed);
        INTERACTIVE_MAX_NS.fetch_max(ns, Ordering::Relaxed);
    }
    match ns {
        0..=99_999 => { B_LT_100US.fetch_add(1, Ordering::Relaxed); }
        100_000..=499_999 => { B_LT_500US.fetch_add(1, Ordering::Relaxed); }
        500_000..=1_999_999 => { B_LT_2MS.fetch_add(1, Ordering::Relaxed); }
        2_000_000..=7_999_999 => { B_LT_8MS.fetch_add(1, Ordering::Relaxed); }
        8_000_000..=15_999_999 => { B_LT_16MS.fetch_add(1, Ordering::Relaxed); }
        _ => { B_GE_16MS.fetch_add(1, Ordering::Relaxed); }
    }
}

pub fn stats() -> Stats {
    let count = COUNT.load(Ordering::Relaxed);
    Stats {
        count,
        average_ns: if count == 0 { 0 } else { SUM_NS.load(Ordering::Relaxed) / count },
        max_ns: MAX_NS.load(Ordering::Relaxed),
        interactive_count: INTERACTIVE_COUNT.load(Ordering::Relaxed),
        interactive_max_ns: INTERACTIVE_MAX_NS.load(Ordering::Relaxed),
        buckets: [
            B_LT_100US.load(Ordering::Relaxed),
            B_LT_500US.load(Ordering::Relaxed),
            B_LT_2MS.load(Ordering::Relaxed),
            B_LT_8MS.load(Ordering::Relaxed),
            B_LT_16MS.load(Ordering::Relaxed),
            B_GE_16MS.load(Ordering::Relaxed),
        ],
    }
}

pub fn log_stats() {
    let s = stats();
    crate::serial_println!(
        "[SCHED-NG-LAT] count={} avg_ns={} max_ns={} interactive_count={} interactive_max_ns={} buckets_lt100us={},lt500us={},lt2ms={},lt8ms={},lt16ms={},ge16ms={}",
        s.count, s.average_ns, s.max_ns, s.interactive_count,
        s.interactive_max_ns, s.buckets[0], s.buckets[1], s.buckets[2],
        s.buckets[3], s.buckets[4], s.buckets[5]
    );
}
