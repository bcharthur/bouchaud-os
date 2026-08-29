use super::{Point, Rect, ResizeEdge, WindowId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapZone { Left, Right }

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowCommand {
    Focus(WindowId), Raise(WindowId), Move(WindowId, Point),
    Resize(WindowId, Rect, ResizeEdge), Minimize(WindowId),
    Maximize(WindowId), Restore(WindowId), Close(WindowId),
    Snap(WindowId, SnapZone), Hover(WindowId, Option<super::HitRegion>),
}
