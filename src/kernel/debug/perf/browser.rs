// Corrélation browser/GUI.
// L'objectif est de répondre à une question simple : après un input, combien
// de temps avant qu'une nouvelle frame client arrive au noyau ?

static BROWSER_CLICK_NS: AtomicU64 = AtomicU64::new(0);
static FIRST_PAINT_NS: AtomicU64 = AtomicU64::new(0);
static PERF_ID: AtomicU64 = AtomicU64::new(0);

static INPUT_SEQ: AtomicU64 = AtomicU64::new(0);
static FRAME_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_INPUT_NS: AtomicU64 = AtomicU64::new(0);
static LAST_INPUT_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_FRAME_NS: AtomicU64 = AtomicU64::new(0);
static LAST_FRAME_INPUT_SEQ: AtomicU64 = AtomicU64::new(0);

static INPUTS_TOTAL: AtomicU64 = AtomicU64::new(0);
static INPUTS_DROPPED: AtomicU64 = AtomicU64::new(0);
static WHEEL_COALESCED: AtomicU64 = AtomicU64::new(0);
static FRAMES_TOTAL: AtomicU64 = AtomicU64::new(0);
static LONG_FRAME_GAPS: AtomicU64 = AtomicU64::new(0);

static FRAME_GAP_MAX_NS: AtomicU64 = AtomicU64::new(0);
static INPUT_TO_FRAME_MAX_NS: AtomicU64 = AtomicU64::new(0);

pub fn browser_click() {
    let now = crate::kernel::timer::monotonic_ns();
    let perf_id = PERF_ID.fetch_add(1, Ordering::AcqRel).saturating_add(1);
    FIRST_PAINT_NS.store(0, Ordering::Release);
    BROWSER_CLICK_NS.store(now, Ordering::Release);
    emit("PERF_BROWSER_CLICK", now, perf_id);
    perf_record(PERF_EVT_BROWSER_CLICK, 0, perf_id, 0);
}

pub fn exec_start(name: &str) {
    let now = crate::kernel::timer::monotonic_ns();
    crate::kernel::dmesg::log_fmt(format_args!(
        "PERF_EXEC_START perf_id={} t_ns={} since_click_ms={} image={}",
        PERF_ID.load(Ordering::Acquire),
        now,
        since_click_ms(now),
        name,
    ));
    perf_record(PERF_EVT_EXEC_START, 0, PERF_ID.load(Ordering::Acquire), 0);
}

pub fn first_paint() {
    let now = crate::kernel::timer::monotonic_ns();
    if FIRST_PAINT_NS
        .compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        emit("PERF_FIRST_PAINT", now, PERF_ID.load(Ordering::Acquire));
        perf_record(PERF_EVT_FIRST_PAINT, 0, PERF_ID.load(Ordering::Acquire), 0);
    }
}

pub fn browser_client_start(pid: u32) {
    perf_record(PERF_EVT_CLIENT_START, pid, 0, 0);
}

pub fn browser_process_exit(pid: u32, code: i32) {
    perf_record(PERF_EVT_CLIENT_EXIT, pid, code as i64 as u64, 0);
}

/// Note un input effectivement accepté par le canal GUI.
///
/// `coalesced` signifie que deux Wheel consécutifs ont été fusionnés dans le
/// message non lu précédent. La distance totale de scroll est conservée mais le
/// client traite moins de messages.
pub fn browser_input(pid: u32, delivered: bool, coalesced: bool) -> u64 {
    let now = crate::kernel::timer::monotonic_ns();
    let seq = INPUT_SEQ.fetch_add(1, Ordering::AcqRel).saturating_add(1);
    INPUTS_TOTAL.fetch_add(1, Ordering::Relaxed);
    if !delivered {
        INPUTS_DROPPED.fetch_add(1, Ordering::Relaxed);
    }
    if coalesced {
        WHEEL_COALESCED.fetch_add(1, Ordering::Relaxed);
    }

    LAST_INPUT_NS.store(now, Ordering::Release);
    LAST_INPUT_SEQ.store(seq, Ordering::Release);
    perf_record(
        PERF_EVT_GUI_INPUT,
        pid,
        seq,
        (delivered as u64) | ((coalesced as u64) << 1),
    );
    seq
}

/// Note une frame valide reçue depuis le client GUI.
pub fn browser_frame(pid: u32, damage_pixels: u64) {
    let now = crate::kernel::timer::monotonic_ns();
    let frame = FRAME_SEQ.fetch_add(1, Ordering::AcqRel).saturating_add(1);
    FRAMES_TOTAL.fetch_add(1, Ordering::Relaxed);

    let previous = LAST_FRAME_NS.swap(now, Ordering::AcqRel);
    if previous != 0 {
        let gap = now.saturating_sub(previous);
        FRAME_GAP_MAX_NS.fetch_max(gap, Ordering::Relaxed);
        if gap >= 250_000_000 {
            LONG_FRAME_GAPS.fetch_add(1, Ordering::Relaxed);
            perf_record(PERF_EVT_GUI_FRAME_GAP, pid, frame, gap);
        }
    }

    let input_seq = LAST_INPUT_SEQ.load(Ordering::Acquire);
    let already_served = LAST_FRAME_INPUT_SEQ.load(Ordering::Acquire);
    if input_seq != 0 && input_seq != already_served {
        let input_ns = LAST_INPUT_NS.load(Ordering::Acquire);
        let latency = now.saturating_sub(input_ns);
        INPUT_TO_FRAME_MAX_NS.fetch_max(latency, Ordering::Relaxed);
        LAST_FRAME_INPUT_SEQ.store(input_seq, Ordering::Release);
        perf_record(PERF_EVT_INPUT_TO_FRAME, pid, input_seq, latency);
    }

    perf_record(PERF_EVT_GUI_FRAME, pid, frame, damage_pixels);
}

pub fn browser_snapshot() -> BrowserPerfSnapshot {
    BrowserPerfSnapshot {
        input_seq: INPUT_SEQ.load(Ordering::Acquire),
        frame_seq: FRAME_SEQ.load(Ordering::Acquire),
        last_input_ns: LAST_INPUT_NS.load(Ordering::Acquire),
        last_input_seq: LAST_INPUT_SEQ.load(Ordering::Acquire),
        last_frame_ns: LAST_FRAME_NS.load(Ordering::Acquire),
        last_frame_input_seq: LAST_FRAME_INPUT_SEQ.load(Ordering::Acquire),
        frames_total: FRAMES_TOTAL.load(Ordering::Relaxed),
        inputs_total: INPUTS_TOTAL.load(Ordering::Relaxed),
        inputs_dropped: INPUTS_DROPPED.load(Ordering::Relaxed),
        wheel_coalesced: WHEEL_COALESCED.load(Ordering::Relaxed),
        frame_gap_max_ns: FRAME_GAP_MAX_NS.load(Ordering::Relaxed),
        input_to_frame_max_ns: INPUT_TO_FRAME_MAX_NS.load(Ordering::Relaxed),
        long_frame_gaps: LONG_FRAME_GAPS.load(Ordering::Relaxed),
    }
}

fn emit(marker: &str, now: u64, perf_id: u64) {
    crate::kernel::dmesg::log_fmt(format_args!(
        "{} perf_id={} t_ns={} since_click_ms={}",
        marker, perf_id, now, since_click_ms(now),
    ));
}

fn since_click_ms(now: u64) -> alloc::string::String {
    let click = BROWSER_CLICK_NS.load(Ordering::Acquire);
    if click == 0 {
        alloc::string::String::from("na")
    } else {
        alloc::format!("{}", now.saturating_sub(click) / 1_000_000)
    }
}
