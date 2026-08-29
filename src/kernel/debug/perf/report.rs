// Rapport synthétique déclenché depuis le journal du client GUI.
// Il remplace des dizaines de logs événementiels par une vue périodique dense.

static REPORT_PREV_FRAMES: AtomicU64 = AtomicU64::new(0);
static REPORT_PREV_INPUTS: AtomicU64 = AtomicU64::new(0);
static REPORT_PREV_DROPPED: AtomicU64 = AtomicU64::new(0);
static REPORT_PREV_COALESCED: AtomicU64 = AtomicU64::new(0);

pub fn browser_report(pid: u32, silence_ms: u64) {
    let snap = browser_snapshot();
    let prev_frames = REPORT_PREV_FRAMES.swap(snap.frames_total, Ordering::AcqRel);
    let prev_inputs = REPORT_PREV_INPUTS.swap(snap.inputs_total, Ordering::AcqRel);
    let prev_dropped = REPORT_PREV_DROPPED.swap(snap.inputs_dropped, Ordering::AcqRel);
    let prev_coalesced = REPORT_PREV_COALESCED.swap(snap.wheel_coalesced, Ordering::AcqRel);

    let (bottleneck, pf_delta) = browser_watchdog(pid, silence_ms);

    crate::serial_println!(
        "[PERF-BROWSER] pid={} frames_delta={} inputs_delta={} dropped_delta={} \
         wheel_coalesced_delta={} silence_ms={} frame_gap_max_ms={} \
         input_to_frame_max_ms={} pending_input={} pf_delta={} bottleneck={}",
        pid,
        snap.frames_total.saturating_sub(prev_frames),
        snap.inputs_total.saturating_sub(prev_inputs),
        snap.inputs_dropped.saturating_sub(prev_dropped),
        snap.wheel_coalesced.saturating_sub(prev_coalesced),
        silence_ms,
        snap.frame_gap_max_ns / 1_000_000,
        snap.input_to_frame_max_ns / 1_000_000,
        (snap.last_input_seq != 0 && snap.last_input_seq != snap.last_frame_input_seq) as u8,
        pf_delta,
        bottleneck_name(bottleneck),
    );
}
