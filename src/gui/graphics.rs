//! Allocation-free CPU visual primitives. Iteration is bounded by the active
//! damage clip before any rounded-corner calculation is performed.

use crate::gui::windowing::Rect;

/// Exact bounded painter used by runtime chrome and host contract tests.
pub fn paint_window_shape<F: FnMut(i32, i32, u32)>(
    geometry: crate::gui::windowing::WindowRenderGeometry,
    radius: u32, shadow_extent: u32, clip: Rect, surface: u32, border: u32,
    mut paint: F,
) {
    for extent in (1..=shadow_extent).rev() {
        let shadow = geometry.outer.outset(extent);
        let shade = 0x07090d + extent * 0x010101;
        stroke_rounded_rect(shadow, radius + extent, 1, clip,
            |x, y| paint(x, y, shade));
    }
    fill_rounded_rect(geometry.outer, radius, clip,
        |x, y| paint(x, y, surface));
    stroke_rounded_rect(geometry.outer, radius, 1, clip,
        |x, y| paint(x, y, border));
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    if right <= x || bottom <= y { None }
    else { Some(Rect::new(x, y, (right - x) as u32, (bottom - y) as u32)) }
}

fn inside_rounded(x: i32, y: i32, width: i32, height: i32, radius: i32) -> bool {
    let radius = radius.max(0).min(width.min(height) / 2);
    if radius == 0 { return true }
    let cx = if x < radius { radius - 1 }
        else if x >= width - radius { width - radius } else { return true };
    let cy = if y < radius { radius - 1 }
        else if y >= height - radius { height - radius } else { return true };
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}

/// Visits only pixels in `rect ∩ clip`; returns the number considered so host
/// tests can prove that sparse damage bounds CPU work.
pub fn fill_rounded_rect<F: FnMut(i32, i32)>(rect: Rect, radius: u32, clip: Rect,
    mut paint: F) -> usize {
    let Some(area) = intersection(rect, clip) else { return 0 };
    let mut visited = 0;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            visited += 1;
            if inside_rounded(x - rect.x, y - rect.y, rect.width as i32,
                rect.height as i32, radius as i32) { paint(x, y); }
        }
    }
    visited
}

pub fn stroke_rounded_rect<F: FnMut(i32, i32)>(rect: Rect, radius: u32,
    thickness: u32, clip: Rect, mut paint: F) -> usize {
    let Some(area) = intersection(rect, clip) else { return 0 };
    let inset = thickness as i32;
    let inner_width = rect.width as i32 - inset * 2;
    let inner_height = rect.height as i32 - inset * 2;
    let mut visited = 0;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            visited += 1;
            let local_x = x - rect.x;
            let local_y = y - rect.y;
            let outer = inside_rounded(local_x, local_y, rect.width as i32,
                rect.height as i32, radius as i32);
            let inner = inner_width > 0 && inner_height > 0
                && local_x >= inset && local_y >= inset
                && inside_rounded(local_x - inset, local_y - inset, inner_width,
                    inner_height, radius.saturating_sub(thickness) as i32);
            if outer && !inner { paint(x, y); }
        }
    }
    visited
}
