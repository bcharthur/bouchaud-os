use super::{Rect, WindowId};
use alloc::string::String;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowPlacement {
    Normal,
    Maximized,
    SnappedLeft,
    SnappedRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowConstraints {
    pub min_width: u32,
    pub min_height: u32,
}

impl Default for WindowConstraints {
    fn default() -> Self {
        Self { min_width: 160, min_height: 96 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowFlags {
    pub resizable: bool,
    pub closable: bool,
    pub maximizable: bool,
    pub minimizable: bool,
    pub snappable: bool,
}

impl WindowFlags {
    pub const STANDARD: Self = Self {
        resizable: true,
        closable: true,
        maximizable: true,
        minimizable: true,
        snappable: true,
    };

    pub const FIXED_SURFACE: Self = Self {
        resizable: false,
        closable: true,
        maximizable: false,
        minimizable: true,
        snappable: false,
    };
}

impl Default for WindowFlags {
    fn default() -> Self { Self::STANDARD }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub restore_rect: Option<Rect>,
    pub placement: WindowPlacement,
    pub min: bool,
    pub constraints: WindowConstraints,
    pub flags: WindowFlags,
}

impl Window {
    pub fn new(id: WindowId, title: String, rect: Rect, flags: WindowFlags) -> Self {
        Self {
            id,
            title,
            x: rect.x,
            y: rect.y,
            w: rect.width as i32,
            h: rect.height as i32,
            restore_rect: None,
            placement: WindowPlacement::Normal,
            min: false,
            constraints: WindowConstraints::default(),
            flags,
        }
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.w.max(0) as u32, self.h.max(0) as u32)
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.x = rect.x; self.y = rect.y;
        self.w = rect.width as i32; self.h = rect.height as i32;
    }
}
