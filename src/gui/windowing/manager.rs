use super::{maximize_button_rect, minimize_button_rect, close_button_rect, ChromeMetrics,
    HitRegion, Point, Rect, SnapZone, Window, WindowCommand, WindowConstraints, WindowFlags,
    WindowId, WindowPlacement, WorkArea};
use alloc::{string::String, vec::Vec};

pub const SHADOW_EXTENT: u32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Damage(pub Rect);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Transition { pub damage: Vec<Damage> }

pub struct BouchaudWindowManager {
    windows: Vec<Window>,
    z_order: Vec<WindowId>,
    focus: Option<WindowId>,
    hover: Option<(WindowId, HitRegion)>,
    work_area: WorkArea,
    chrome: ChromeMetrics,
}

impl BouchaudWindowManager {
    pub fn new(work_area: WorkArea) -> Self {
        Self {
            windows: Vec::new(), z_order: Vec::new(), focus: None, hover: None,
            work_area,
            chrome: super::WINDOW_CHROME,
        }
    }

    pub fn create(&mut self, title: String, rect: Rect, constraints: WindowConstraints,
        flags: WindowFlags) -> WindowId {
        let id = WindowId::allocate();
        let rect = self.work_area.constrain(rect, constraints.min_width, constraints.min_height);
        let mut window = Window::new(id, title, rect, flags);
        window.constraints = constraints;
        self.windows.push(window);
        self.z_order.push(id);
        id
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|window| window.id == id)
    }
    pub fn z_order(&self) -> &[WindowId] { &self.z_order }
    pub fn focus(&self) -> Option<WindowId> { self.focus }
    pub fn windows(&self) -> &[Window] { &self.windows }
    pub fn hover(&self) -> Option<(WindowId, HitRegion)> { self.hover }
    pub fn footprint(rect: Rect) -> Rect {
        super::window_render_geometry(rect, super::TITLEBAR_HEIGHT,
            super::WINDOW_RADIUS, SHADOW_EXTENT).painted_bounds
    }

    fn damage_window(out: &mut Transition, rect: Rect) {
        out.damage.push(Damage(Self::footprint(rect)));
    }

    fn top_visible(&self) -> Option<WindowId> {
        self.z_order.iter().rev().copied().find(|id| {
            self.window(*id).map(|window| !window.min).unwrap_or(false)
        })
    }

    fn hover_rect(&self, id: WindowId, region: HitRegion) -> Option<Rect> {
        let rect = self.window(id)?.rect();
        match region {
            HitRegion::Close => Some(close_button_rect(rect, self.chrome)),
            HitRegion::Maximize => Some(maximize_button_rect(rect, self.chrome)),
            HitRegion::Minimize => Some(minimize_button_rect(rect, self.chrome)),
            _ => None,
        }
    }

    pub fn apply(&mut self, command: WindowCommand) -> Transition {
        let mut out = Transition::default();
        match command {
            WindowCommand::Close(id) => {
                let Some(index) = self.windows.iter().position(|window| window.id == id) else { return out };
                if !self.windows[index].flags.closable { return out }
                let closed = self.windows.remove(index);
                Self::damage_window(&mut out, closed.rect());
                self.z_order.retain(|candidate| *candidate != id);
                if self.focus == Some(id) {
                    self.focus = self.top_visible();
                    if let Some(rect) = self.focus.and_then(|new_id| self.window(new_id).map(|w| w.rect())) {
                        Self::damage_window(&mut out, rect);
                    }
                }
            }
            WindowCommand::Focus(id) => {
                if self.window(id).map(|window| window.min).unwrap_or(true)
                    || self.focus == Some(id) { return out }
                if let Some(rect) = self.focus.and_then(|old| self.window(old).map(|w| w.rect())) {
                    Self::damage_window(&mut out, rect);
                }
                self.focus = Some(id);
                Self::damage_window(&mut out, self.window(id).unwrap().rect());
            }
            WindowCommand::Raise(id) => {
                if self.window(id).is_some() {
                    self.z_order.retain(|candidate| *candidate != id);
                    self.z_order.push(id);
                }
            }
            WindowCommand::Hover(id, region) => {
                let next = region.map(|value| (id, value));
                if self.hover != next {
                    if let Some((old_id, old_region)) = self.hover {
                        if let Some(rect) = self.hover_rect(old_id, old_region) { out.damage.push(Damage(rect)); }
                    }
                    self.hover = next;
                    if let Some((new_id, new_region)) = next {
                        if let Some(rect) = self.hover_rect(new_id, new_region) { out.damage.push(Damage(rect)); }
                    }
                }
            }
            other => self.apply_geometry(other, &mut out),
        }
        out
    }

    fn apply_geometry(&mut self, command: WindowCommand, out: &mut Transition) {
        let Some(id) = command_id(&command) else { return };
        let Some(index) = self.windows.iter().position(|window| window.id == id) else { return };
        let flags = self.windows[index].flags;
        let allowed = match command {
            WindowCommand::Resize(..) => flags.resizable,
            WindowCommand::Minimize(..) => flags.minimizable,
            WindowCommand::Maximize(..) => flags.maximizable,
            WindowCommand::Snap(..) => flags.snappable,
            _ => true,
        };
        if !allowed { return }
        let old = self.windows[index].rect();
        match command {
            WindowCommand::Move(_, Point { x, y }) => {
                let window = &mut self.windows[index];
                let size = if window.placement == WindowPlacement::Normal {
                    window.rect()
                } else {
                    window.restore_rect.take().unwrap_or(window.rect())
                };
                let rect = self.work_area.constrain(Rect::new(x, y, size.width,
                    size.height), window.constraints.min_width, window.constraints.min_height);
                window.set_rect(rect);
                window.placement = WindowPlacement::Normal;
            }
            WindowCommand::Resize(_, rect, _) => {
                let window = &mut self.windows[index];
                let rect = self.work_area.constrain(rect, window.constraints.min_width,
                    window.constraints.min_height);
                window.set_rect(rect);
                window.placement = WindowPlacement::Normal;
            }
            WindowCommand::Minimize(_) => {
                if self.windows[index].min { return }
                self.windows[index].min = true;
                if self.focus == Some(id) {
                    self.focus = self.top_visible();
                }
            }
            WindowCommand::Maximize(_) => {
                let window = &mut self.windows[index];
                if window.placement == WindowPlacement::Maximized { return }
                if window.placement == WindowPlacement::Normal { window.restore_rect = Some(window.rect()); }
                window.set_rect(self.work_area.maximize());
                window.placement = WindowPlacement::Maximized;
            }
            WindowCommand::Restore(_) => {
                let window = &mut self.windows[index];
                if window.min {
                    window.min = false;
                } else if window.placement != WindowPlacement::Normal {
                    if let Some(rect) = window.restore_rect.take() { window.set_rect(rect); }
                    window.placement = WindowPlacement::Normal;
                } else { return }
            }
            WindowCommand::Snap(_, zone) => {
                let window = &mut self.windows[index];
                if window.placement == WindowPlacement::Normal { window.restore_rect = Some(window.rect()); }
                match zone {
                    SnapZone::Left => { window.set_rect(self.work_area.snap_left()); window.placement = WindowPlacement::SnappedLeft; }
                    SnapZone::Right => { window.set_rect(self.work_area.snap_right()); window.placement = WindowPlacement::SnappedRight; }
                }
            }
            _ => return,
        }
        Self::damage_window(out, old);
        Self::damage_window(out, self.windows[index].rect());
        if matches!(command, WindowCommand::Minimize(_)) {
            if let Some(rect) = self.focus.and_then(|new_id| self.window(new_id).map(|w| w.rect())) {
                Self::damage_window(out, rect);
            }
        }
    }
}

fn command_id(command: &WindowCommand) -> Option<WindowId> {
    match *command {
        WindowCommand::Move(id, _) | WindowCommand::Resize(id, _, _)
        | WindowCommand::Minimize(id) | WindowCommand::Maximize(id)
        | WindowCommand::Restore(id) | WindowCommand::Snap(id, _) => Some(id),
        _ => None,
    }
}
