pub fn log_stats() {
    crate::serial_println!(
        "[MM-READAHEAD] observe={} sequential={} requested={} ok={} fail={} w2={} w4={} w8={} w16={} max_window={}",
        RA_OBSERVE.load(Ordering::Relaxed),
        RA_SEQUENTIAL.load(Ordering::Relaxed),
        RA_REQUESTED.load(Ordering::Relaxed),
        RA_OK.load(Ordering::Relaxed),
        RA_FAIL.load(Ordering::Relaxed),
        RA_WINDOW_2.load(Ordering::Relaxed),
        RA_WINDOW_4.load(Ordering::Relaxed),
        RA_WINDOW_8.load(Ordering::Relaxed),
        RA_WINDOW_16.load(Ordering::Relaxed),
        RA_MAX_WINDOW_SEEN.load(Ordering::Relaxed),
    );
}
