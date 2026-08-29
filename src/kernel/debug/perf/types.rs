// Types et codes d'événements de l'observatoire.
// Valeurs stables pour rendre les logs/flight recorder analysables hors noyau.

pub const PERF_EVT_BROWSER_CLICK: u32 = 1;
pub const PERF_EVT_EXEC_START: u32 = 2;
pub const PERF_EVT_FIRST_PAINT: u32 = 3;
pub const PERF_EVT_CLIENT_START: u32 = 4;
pub const PERF_EVT_CLIENT_EXIT: u32 = 5;

pub const PERF_EVT_GUI_INPUT: u32 = 10;
pub const PERF_EVT_GUI_FRAME: u32 = 11;
pub const PERF_EVT_GUI_FRAME_GAP: u32 = 12;
pub const PERF_EVT_INPUT_TO_FRAME: u32 = 13;

pub const PERF_EVT_BKL_ALERT: u32 = 20;
pub const PERF_EVT_WATCHDOG: u32 = 30;

pub const PERF_RING_CAPACITY: usize = 2048;
pub const PERF_DUMP_EVENTS: usize = 192;

#[derive(Clone, Copy, Debug, Default)]
pub struct BrowserPerfSnapshot {
    pub input_seq: u64,
    pub frame_seq: u64,
    pub last_input_ns: u64,
    pub last_input_seq: u64,
    pub last_frame_ns: u64,
    pub last_frame_input_seq: u64,
    pub frames_total: u64,
    pub inputs_total: u64,
    pub inputs_dropped: u64,
    pub wheel_coalesced: u64,
    pub frame_gap_max_ns: u64,
    pub input_to_frame_max_ns: u64,
    pub long_frame_gaps: u64,
}

#[inline]
pub const fn perf_event_name(kind: u32) -> &'static str {
    match kind {
        PERF_EVT_BROWSER_CLICK => "browser-click",
        PERF_EVT_EXEC_START => "exec-start",
        PERF_EVT_FIRST_PAINT => "first-paint",
        PERF_EVT_CLIENT_START => "client-start",
        PERF_EVT_CLIENT_EXIT => "client-exit",
        PERF_EVT_GUI_INPUT => "gui-input",
        PERF_EVT_GUI_FRAME => "gui-frame",
        PERF_EVT_GUI_FRAME_GAP => "gui-frame-gap",
        PERF_EVT_INPUT_TO_FRAME => "input-to-frame",
        PERF_EVT_BKL_ALERT => "bkl-alert",
        PERF_EVT_WATCHDOG => "watchdog",
        _ => "unknown",
    }
}
