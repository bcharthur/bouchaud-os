use super::{Point, Rect, WindowId};

pub const TITLEBAR_HEIGHT: u32 = 32;
pub const WINDOW_BUTTON_WIDTH: u32 = 40;
pub const RESIZE_BORDER: u32 = 5;

pub const WINDOW_CHROME: ChromeMetrics = ChromeMetrics {
    titlebar_height: TITLEBAR_HEIGHT,
    resize_border: RESIZE_BORDER,
    button_width: WINDOW_BUTTON_WIDTH,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitRegion {
    Outside, Client, Titlebar, Close, Minimize, Maximize,
    Left, Right, Top, Bottom, NorthWest, NorthEast, SouthWest, SouthEast,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChromeMetrics {
    pub titlebar_height: u32,
    pub resize_border: u32,
    pub button_width: u32,
}

pub fn titlebar_rect(window: Rect, metrics: ChromeMetrics) -> Rect {
    Rect::new(window.x, window.y, window.width, metrics.titlebar_height)
}

fn button_rect(window: Rect, metrics: ChromeMetrics, from_right: u32) -> Rect {
    Rect::new(window.right() - (metrics.button_width * from_right) as i32,
        window.y, metrics.button_width, metrics.titlebar_height)
}

pub fn close_button_rect(window: Rect, metrics: ChromeMetrics) -> Rect {
    button_rect(window, metrics, 1)
}

pub fn maximize_button_rect(window: Rect, metrics: ChromeMetrics) -> Rect {
    button_rect(window, metrics, 2)
}

pub fn minimize_button_rect(window: Rect, metrics: ChromeMetrics) -> Rect {
    button_rect(window, metrics, 3)
}

/// Controls have priority over resize borders. A fixed-surface window passes
/// `resizable = false`, in which case this function never returns a resize
/// region and edge pixels fall back to controls, titlebar, or client.
pub fn hit_test(window: Rect, point: Point, metrics: ChromeMetrics,
    resizable: bool) -> HitRegion {
    if !window.outset(metrics.resize_border).contains(point) { return HitRegion::Outside; }
    if close_button_rect(window, metrics).contains(point) { return HitRegion::Close }
    if maximize_button_rect(window, metrics).contains(point) { return HitRegion::Maximize }
    if minimize_button_rect(window, metrics).contains(point) { return HitRegion::Minimize }
    if !resizable {
        return if titlebar_rect(window, metrics).contains(point) { HitRegion::Titlebar }
            else if window.contains(point) { HitRegion::Client } else { HitRegion::Outside };
    }
    let border = metrics.resize_border as i32;
    let left = point.x < window.x + border;
    let right = point.x >= window.right() - border;
    let top = point.y < window.y + border;
    let bottom = point.y >= window.bottom() - border;
    match (left, right, top, bottom) {
        (true, _, true, _) => return HitRegion::NorthWest,
        (_, true, true, _) => return HitRegion::NorthEast,
        (true, _, _, true) => return HitRegion::SouthWest,
        (_, true, _, true) => return HitRegion::SouthEast,
        (true, _, _, _) => return HitRegion::Left,
        (_, true, _, _) => return HitRegion::Right,
        (_, _, true, _) => return HitRegion::Top,
        (_, _, _, true) => return HitRegion::Bottom,
        _ => {}
    }
    if titlebar_rect(window, metrics).contains(point) { HitRegion::Titlebar }
    else { HitRegion::Client }
}

/// Monotonic-time double click recognizer, independent from frame scheduling.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoubleClickDetector { last: Option<(WindowId, u8, Point, u64)> }

impl DoubleClickDetector {
    pub const MAX_DELAY_MS: u64 = 400;
    pub const MAX_DISTANCE: i32 = 6;

    pub fn click(&mut self, id: WindowId, button: u8, point: Point, now_ms: u64) -> bool {
        let double = self.last.map(|(old_id, old_button, old_point, old_ms)| {
            old_id == id && old_button == button
                && now_ms.saturating_sub(old_ms) <= Self::MAX_DELAY_MS
                && (old_point.x - point.x).abs() <= Self::MAX_DISTANCE
                && (old_point.y - point.y).abs() <= Self::MAX_DISTANCE
        }).unwrap_or(false);
        self.last = if double { None } else { Some((id, button, point, now_ms)) };
        double
    }
}
