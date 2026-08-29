//! Bouchaud Window System policy layer.
//!
//! This module is deliberately independent from the compositor and wire
//! protocol: it turns explicit commands into state transitions and painted
//! footprints. The existing event loop can therefore migrate incrementally.
//!
//! Current boundary: runtime `Vec<Win>` still owns application payloads and
//! paint order, while `Window` is its canonical geometry/visibility state and
//! runtime geometry actions use `WindowCommand`. `BouchaudWindowManager` is the
//! policy/test model for the future focus and z-order migration; no runtime
//! instance of it is claimed yet.

mod command;
mod geometry;
mod hit_test;
mod id;
pub(crate) mod manager;
mod state;

pub use command::{SnapZone, WindowCommand};
pub use geometry::{client_rect, outer_rect_for_client_size, Point, Rect, ResizeEdge,
    window_render_geometry, WindowRenderGeometry, WorkArea, SNAP_THRESHOLD, WINDOW_BORDER,
    WINDOW_RADIUS};
pub use hit_test::{close_button_rect, hit_test, maximize_button_rect,
    minimize_button_rect, titlebar_rect, ChromeMetrics, DoubleClickDetector, HitRegion};
pub use hit_test::{RESIZE_BORDER, TITLEBAR_HEIGHT, WINDOW_BUTTON_WIDTH, WINDOW_CHROME};
pub use id::WindowId;
pub use manager::{BouchaudWindowManager, Damage, Transition};
pub use state::{Window, WindowConstraints, WindowFlags, WindowPlacement};
