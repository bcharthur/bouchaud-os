//! Peinture de la display list Nautile.
//!
//! Cette couche transforme la display list issue du layout en pixels framebuffer.

use crate::gui::framebuffer as fb;
use crate::gui::image::Image;
use super::display_list::{Item, Layer, Page};

// ----------------------------------------------------------------------------
// Peinture
// ----------------------------------------------------------------------------

// Throttle des logs de peinture lente (ticks du dernier log).
static mut LAST_PAINT_LOG: u64 = 0;

pub fn paint(page: &Page, scroll: i32, bx: usize, by: usize, bw: usize, bh: usize) {
    let t0 = crate::kernel::timer::cycles_since_boot();
    fb::fill_rect_rgb(bx, by, bw, bh, page.bg);
    let view = (bx as i32, by as i32, bw as i32, bh as i32);
    // Ordre d'empilement (stacking) : couches z<0, puis flux normal (z=0), puis
    // couches z>=0. Les couches sont deja triees par z croissant dans layout().
    for l in &page.layers { if l.z < 0 { paint_layer(l, &page.images, scroll, view); } }
    paint_list(&page.items, scroll, &page.images, view, view);
    for l in &page.layers { if l.z >= 0 { paint_layer(l, &page.images, scroll, view); } }
    // Surveillance des frames lentes (lag de scroll), throttle ~1s.
    let mc = crate::kernel::timer::cycles_since_boot().wrapping_sub(t0) / 1_000_000;
    if mc > 80 {
        let now = crate::kernel::timer::ticks();
        unsafe {
            if now.wrapping_sub(LAST_PAINT_LOG) >= 18 {
                LAST_PAINT_LOG = now;
                crate::dlog!(crate::diag::Cat::Paint, "frame lente: {} items +{} couches -> {}Mc",
                    page.items.len(), page.layers.len(), mc);
            }
        }
    }
}

// Peint une couche positionnee : `fixed` ignore le scroll, `clip` restreint le
// viewport effectif au rectangle d'overflow (intersecte avec la fenetre).
fn paint_layer(l: &Layer, images: &[Image], scroll: i32, view: (i32, i32, i32, i32)) {
    let sc = if l.fixed { 0 } else { scroll };
    let mut vp = view;
    if let Some((cx, cy, cw, ch)) = l.clip {
        // Rectangle de clip en coords ecran (decale du scroll si non fixe).
        let sx = view.0 + cx;
        let sy = view.1 + cy - sc;
        vp = intersect(view, (sx, sy, cw, ch));
    }
    if vp.2 <= 0 || vp.3 <= 0 { return; }
    paint_list(&l.items, sc, images, vp, view);
}

// Intersection de deux rectangles (x, y, w, h).
fn intersect(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 + a.2).min(b.0 + b.2);
    let y1 = (a.1 + a.3).min(b.1 + b.3);
    (x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
}


fn paint_rounded_rect(x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32, vp: (i32, i32, i32, i32)) {
    let (vx, vy, vw, vh) = vp;
    if w <= 0 || h <= 0 { return; }
    let r = radius.min(w / 2).min(h / 2).max(0);
    let y0 = y.max(vy);
    let y1 = (y + h).min(vy + vh);
    for yy in y0..y1 {
        let local_y = yy - y;
        let dy = if local_y < r { r - local_y } else if local_y >= h - r { local_y - (h - r - 1) } else { 0 };
        let mut inset = 0;
        if r > 0 && dy > 0 {
            let rr = r * r;
            let yy2 = dy * dy;
            let mut xx = 0;
            while xx < r && xx * xx + yy2 > rr { xx += 1; }
            inset = xx;
        }
        let xs = (x + inset).max(vx);
        let xe = (x + w - inset).min(vx + vw);
        if xe > xs { fb::fill_rect_rgb(xs as usize, yy as usize, (xe - xs) as usize, 1, color); }
    }
}

// Peint une liste d'items (coords document) dans le viewport effectif `vp`
// (coords ecran), avec defilement `scroll`. `blit_view` borne le blit d'images.
fn paint_list(items: &[Item], scroll: i32, images: &[Image], vp: (i32, i32, i32, i32), _blit_view: (i32, i32, i32, i32)) {
    let (vx, vy, vw, vh) = vp;
    if vw <= 0 || vh <= 0 { return; }
    let bx = vx as usize; let by = vy as usize; let bw = vw as usize; let bh = vh as usize;
    for it in items {
        match it {
            Item::Rect { x, y, w, h, color } => {
                let sy = vy + y - scroll;
                if sy + h <= vy || sy >= vy + vh { continue; }
                let yy = sy.max(vy);
                let hh = (sy + h).min(vy + vh) - yy;
                let xx = vx + x;
                // Clip horizontal au viewport effectif (gauche + droite).
                let x0 = xx.max(vx);
                let x1 = (xx + w).min(vx + vw);
                let ww = (x1 - x0).max(0);
                if hh > 0 && ww > 0 {
                    fb::fill_rect_rgb(x0 as usize, yy as usize, ww as usize, hh as usize, *color);
                }
            }
            Item::RoundedRect { x, y, w, h, radius, color } => {
                let sy = vy + y - scroll;
                if sy + h <= vy || sy >= vy + vh { continue; }
                let xx = vx + x;
                paint_rounded_rect(xx, sy, *w, *h, *radius, *color, vp);
            }
            Item::Text { x, y, s, color, scale, bold } => {
                let sy = vy + y - scroll;
                let h = 8 * *scale as i32;
                if sy < vy || sy + h > vy + vh { continue; }
                let xx = vx + x;
                if xx >= vx && xx < vx + vw {
                    let px = 8 * *scale as i32;
                    if !super::font_ttf::draw_text(xx, sy, s, *color, px, *bold) {
                        fb::draw_text_rgb(xx as usize, sy as usize, s, *color, *scale);
                        if *bold { fb::draw_text_rgb((xx + 1) as usize, sy as usize, s, *color, *scale); }
                    }
                }
            }
            Item::Image { x, y, w: _w, h, idx } => {
                let sy = vy + y - scroll;
                if sy + h <= vy || sy >= vy + vh { continue; }
                if let Some(img) = images.get(*idx) {
                    let xx = vx + x;
                    if xx >= vx {
                        let skip = (vy - sy).max(0) as usize;
                        let draw_h = img.h.saturating_sub(skip);
                        let start = skip.saturating_mul(img.w).min(img.pix.len());
                        fb::blit_rgb(xx as usize, sy.max(vy) as usize, img.w, draw_h, &img.pix[start..], bx, by, bw, bh);
                    }
                }
            }
        }
    }
}
