#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Point { pub x: i32, pub y: i32 }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect { pub x: i32, pub y: i32, pub width: u32, pub height: u32 }

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self { Self { x, y, width, height } }
    pub fn right(self) -> i32 { self.x.saturating_add(self.width as i32) }
    pub fn bottom(self) -> i32 { self.y.saturating_add(self.height as i32) }
    pub fn contains(self, p: Point) -> bool { p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom() }
    pub fn outset(self, amount: u32) -> Self {
        let a = amount as i32;
        Self::new(self.x - a, self.y - a, self.width.saturating_add(amount * 2), self.height.saturating_add(amount * 2))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkArea(pub Rect);

impl WorkArea {
    pub fn maximize(self) -> Rect { self.0 }
    pub fn snap_left(self) -> Rect { Rect::new(self.0.x, self.0.y, self.0.width / 2, self.0.height) }
    pub fn snap_right(self) -> Rect {
        let left = self.0.width / 2;
        Rect::new(self.0.x + left as i32, self.0.y, self.0.width - left, self.0.height)
    }
    pub fn constrain(self, rect: Rect, min_width: u32, min_height: u32) -> Rect {
        let width = rect.width.max(min_width).min(self.0.width);
        let height = rect.height.max(min_height).min(self.0.height);
        let max_x = self.0.right() - width as i32;
        let max_y = self.0.bottom() - height as i32;
        Rect::new(rect.x.clamp(self.0.x, max_x), rect.y.clamp(self.0.y, max_y), width, height)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeEdge { Left, Right, Top, Bottom, NorthWest, NorthEast, SouthWest, SouthEast }

pub const WINDOW_BORDER: u32 = 1;
pub const SNAP_THRESHOLD: i32 = 16;
pub const WINDOW_RADIUS: u32 = 10;

/// One contract shared by painter, scene culling and transition damage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowRenderGeometry {
    pub outer: Rect,
    pub client: Rect,
    pub painted_bounds: Rect,
    pub opaque: Rect,
}

pub fn window_render_geometry(outer: Rect, titlebar: u32, radius: u32,
    shadow_extent: u32) -> WindowRenderGeometry {
    let inset = radius.min(outer.width / 2);
    WindowRenderGeometry {
        outer,
        client: client_rect(outer, titlebar),
        painted_bounds: outer.outset(shadow_extent),
        // A rounded rectangle is guaranteed opaque in its central vertical
        // strip. Its four corner squares are deliberately NOT advertised.
        opaque: Rect::new(outer.x + inset as i32, outer.y,
            outer.width.saturating_sub(inset * 2), outer.height),
    }
}

pub fn outer_rect_for_client_size(origin: Point, width: u32, height: u32, titlebar: u32) -> Rect {
    Rect::new(origin.x, origin.y, width + WINDOW_BORDER * 2,
        height + titlebar + WINDOW_BORDER * 2)
}

pub fn client_rect(outer: Rect, titlebar: u32) -> Rect {
    Rect::new(outer.x + WINDOW_BORDER as i32,
        outer.y + titlebar as i32 + WINDOW_BORDER as i32,
        outer.width.saturating_sub(WINDOW_BORDER * 2),
        outer.height.saturating_sub(titlebar + WINDOW_BORDER * 2))
}
