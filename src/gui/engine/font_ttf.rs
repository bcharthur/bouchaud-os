//! Rasterizer de police TrueType (`glyf`) from-scratch, no_std, antialiase.
//!
//! Lit une police TrueType embarquee (DejaVu Sans), decode les contours
//! (`head`/`cmap` format 4/`loca`/`glyf` quadratiques/`hmtx`/`hhea`), les
//! remplit par balayage (scanline) avec couverture sous-pixel, et melange le
//! resultat dans le framebuffer. Glyphes mis en cache par (glyphe, taille px).
//!
//! Si la police est absente ou illisible, tout renvoie un repli (largeurs
//! monospace / `draw_text` -> false) et le moteur web retombe sur la police
//! bitmap 8x8 : le boot n'est jamais casse.
//!
//! Seul usage du flottant : `f32` (additions/multiplications, dispo en `core`).

use crate::gui::framebuffer as fb;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

// Police embarquee (le fichier est fourni par l'utilisateur, voir `fonts/`).
static FONT_DATA: &[u8] = include_bytes!("fonts/DejaVuSans.ttf");

// ----------------------------------------------------------------------------
// Lecture big-endian
// ----------------------------------------------------------------------------
fn u16b(d: &[u8], i: usize) -> u16 { ((d[i] as u16) << 8) | d[i + 1] as u16 }
fn i16b(d: &[u8], i: usize) -> i16 { u16b(d, i) as i16 }
fn u32b(d: &[u8], i: usize) -> u32 {
    ((d[i] as u32) << 24) | ((d[i + 1] as u32) << 16) | ((d[i + 2] as u32) << 8) | d[i + 3] as u32
}

fn ifloor(x: f32) -> i32 { let t = x as i32; if (t as f32) > x { t - 1 } else { t } }
fn iceil(x: f32) -> i32 { let t = x as i32; if (t as f32) < x { t + 1 } else { t } }

// ----------------------------------------------------------------------------
// Police analysee (offsets dans `data`)
// ----------------------------------------------------------------------------
struct Font {
    data: &'static [u8],
    upem: f32,
    ascent: f32,
    loca_long: bool,
    loca: usize,
    glyf: usize,
    cmap_sub: usize,
    hmtx: usize,
    num_h: usize,
    num_glyphs: usize,
}

impl Font {
    fn parse(data: &'static [u8]) -> Option<Font> {
        if data.len() < 12 { return None; }
        let num_tables = u16b(data, 4) as usize;
        let mut head = 0usize; let mut maxp = 0; let mut loca = 0; let mut glyf = 0;
        let mut cmap = 0; let mut hmtx = 0; let mut hhea = 0;
        for i in 0..num_tables {
            let rec = 12 + i * 16;
            if rec + 16 > data.len() { return None; }
            let tag = &data[rec..rec + 4];
            let off = u32b(data, rec + 8) as usize;
            match tag {
                b"head" => head = off, b"maxp" => maxp = off, b"loca" => loca = off,
                b"glyf" => glyf = off, b"cmap" => cmap = off, b"hmtx" => hmtx = off,
                b"hhea" => hhea = off, _ => {}
            }
        }
        if head == 0 || maxp == 0 || loca == 0 || glyf == 0 || cmap == 0 || hmtx == 0 || hhea == 0 { return None; }
        if head + 54 > data.len() || hhea + 36 > data.len() || maxp + 6 > data.len() { return None; }

        let upem = u16b(data, head + 18) as f32;
        if upem < 1.0 { return None; }
        let loca_long = i16b(data, head + 50) != 0;
        let num_glyphs = u16b(data, maxp + 4) as usize;
        let ascent = i16b(data, hhea + 4) as f32;
        let num_h = u16b(data, hhea + 34) as usize;

        // cmap : on cherche une sous-table Unicode BMP (3/1) ou (0/x), format 4.
        let nsub = u16b(data, cmap + 2) as usize;
        let mut sub = 0usize;
        for i in 0..nsub {
            let rec = cmap + 4 + i * 8;
            if rec + 8 > data.len() { break; }
            let plat = u16b(data, rec);
            let enc = u16b(data, rec + 2);
            let off = u32b(data, rec + 4) as usize;
            if (plat == 3 && (enc == 1 || enc == 0)) || plat == 0 {
                let so = cmap + off;
                if so + 2 <= data.len() && u16b(data, so) == 4 { sub = so; if plat == 3 && enc == 1 { break; } }
            }
        }
        if sub == 0 { return None; }

        Some(Font { data, upem, ascent, loca_long, loca, glyf, cmap_sub: sub, hmtx, num_h, num_glyphs })
    }

    // cmap format 4 : caractere BMP -> identifiant de glyphe.
    fn glyph_id(&self, ch: char) -> u16 {
        let c = ch as u32;
        if c > 0xFFFF { return 0; }
        let c = c as u16;
        let d = self.data;
        let s = self.cmap_sub;
        let segx2 = u16b(d, s + 6) as usize;
        let segc = segx2 / 2;
        let end_o = s + 14;
        let start_o = end_o + segx2 + 2;
        let delta_o = start_o + segx2;
        let range_o = delta_o + segx2;
        for i in 0..segc {
            let end = u16b(d, end_o + i * 2);
            if c <= end {
                let start = u16b(d, start_o + i * 2);
                if c < start { return 0; }
                let delta = u16b(d, delta_o + i * 2);
                let ro = u16b(d, range_o + i * 2) as usize;
                if ro == 0 {
                    return c.wrapping_add(delta);
                } else {
                    let gi = range_o + i * 2 + ro + (c - start) as usize * 2;
                    if gi + 1 >= d.len() { return 0; }
                    let g = u16b(d, gi);
                    if g == 0 { return 0; }
                    return g.wrapping_add(delta);
                }
            }
        }
        0
    }

    fn advance(&self, gid: u16) -> u16 {
        let g = gid as usize;
        let i = if g < self.num_h { g } else { self.num_h.saturating_sub(1) };
        let o = self.hmtx + i * 4;
        if o + 2 > self.data.len() { return 0; }
        u16b(self.data, o)
    }

    // (offset, length) du glyphe dans la table glyf.
    fn glyph_range(&self, gid: u16) -> Option<(usize, usize)> {
        let g = gid as usize;
        if g + 1 > self.num_glyphs { return None; }
        let (a, b) = if self.loca_long {
            (u32b(self.data, self.loca + g * 4) as usize, u32b(self.data, self.loca + g * 4 + 4) as usize)
        } else {
            (u16b(self.data, self.loca + g * 2) as usize * 2, u16b(self.data, self.loca + g * 2 + 2) as usize * 2)
        };
        if b <= a { return Some((0, 0)); } // glyphe vide (ex. espace)
        Some((self.glyf + a, b - a))
    }

    // Contours du glyphe en unites de police (y vers le HAUT). Gere composites.
    fn outline(&self, gid: u16, depth: u32) -> Vec<Vec<(f32, f32, bool)>> {
        let mut out: Vec<Vec<(f32, f32, bool)>> = Vec::new();
        if depth > 5 { return out; }
        let (off, len) = match self.glyph_range(gid) { Some(v) => v, None => return out };
        if len == 0 || off + 10 > self.data.len() { return out; }
        let d = self.data;
        let nc = i16b(d, off);
        if nc < 0 {
            // glyphe composite
            let mut p = off + 10;
            loop {
                if p + 4 > d.len() { break; }
                let flags = u16b(d, p);
                let cgid = u16b(d, p + 2);
                p += 4;
                let (dx, dy);
                if flags & 0x0001 != 0 { // ARG_1_AND_2_ARE_WORDS
                    dx = i16b(d, p) as f32; dy = i16b(d, p + 2) as f32; p += 4;
                } else {
                    dx = (d[p] as i8) as f32; dy = (d[p + 1] as i8) as f32; p += 2;
                }
                // echelle (on gere translation + echelle uniforme ; 2x2 ignore)
                let mut sx = 1.0f32; let mut sy = 1.0f32;
                if flags & 0x0008 != 0 { let s = f2dot14(d, p); sx = s; sy = s; p += 2; }
                else if flags & 0x0040 != 0 { sx = f2dot14(d, p); sy = f2dot14(d, p + 2); p += 4; }
                else if flags & 0x0080 != 0 { sx = f2dot14(d, p); sy = f2dot14(d, p + 6); p += 8; }
                let xy = flags & 0x0002 != 0; // ARGS_ARE_XY_VALUES
                let sub = self.outline(cgid, depth + 1);
                for c in sub {
                    let mut nc2: Vec<(f32, f32, bool)> = Vec::with_capacity(c.len());
                    for (x, y, on) in c {
                        let nx = x * sx + if xy { dx } else { 0.0 };
                        let ny = y * sy + if xy { dy } else { 0.0 };
                        nc2.push((nx, ny, on));
                    }
                    out.push(nc2);
                }
                if flags & 0x0020 == 0 { break; } // MORE_COMPONENTS
            }
            return out;
        }
        // glyphe simple
        let nc = nc as usize;
        let mut p = off + 10;
        if p + nc * 2 + 2 > d.len() { return out; }
        let mut ends = Vec::with_capacity(nc);
        for i in 0..nc { ends.push(u16b(d, p + i * 2) as usize); }
        p += nc * 2;
        let npts = ends.last().map(|e| e + 1).unwrap_or(0);
        let il = u16b(d, p) as usize; p += 2 + il; // saute les instructions
        // drapeaux
        let mut flags = Vec::with_capacity(npts);
        while flags.len() < npts {
            if p >= d.len() { return out; }
            let f = d[p]; p += 1; flags.push(f);
            if f & 0x08 != 0 { // repeat
                if p >= d.len() { return out; }
                let r = d[p]; p += 1;
                for _ in 0..r { if flags.len() < npts { flags.push(f); } }
            }
        }
        // coordonnees X
        let mut xs = Vec::with_capacity(npts);
        let mut x = 0i32;
        for &f in &flags {
            if f & 0x02 != 0 { if p >= d.len() { return out; } let dxv = d[p] as i32; p += 1; x += if f & 0x10 != 0 { dxv } else { -dxv }; }
            else if f & 0x10 == 0 { if p + 1 >= d.len() { return out; } x += i16b(d, p) as i32; p += 2; }
            xs.push(x);
        }
        // coordonnees Y
        let mut ys = Vec::with_capacity(npts);
        let mut y = 0i32;
        for &f in &flags {
            if f & 0x04 != 0 { if p >= d.len() { return out; } let dyv = d[p] as i32; p += 1; y += if f & 0x20 != 0 { dyv } else { -dyv }; }
            else if f & 0x20 == 0 { if p + 1 >= d.len() { return out; } y += i16b(d, p) as i32; p += 2; }
            ys.push(y);
        }
        // decoupe en contours
        let mut start = 0usize;
        for &e in &ends {
            if e < start || e >= npts { break; }
            let mut c = Vec::with_capacity(e - start + 1);
            for k in start..=e {
                c.push((xs[k] as f32, ys[k] as f32, flags[k] & 0x01 != 0));
            }
            out.push(c);
            start = e + 1;
        }
        out
    }
}

fn f2dot14(d: &[u8], i: usize) -> f32 { i16b(d, i) as f32 / 16384.0 }

// ----------------------------------------------------------------------------
// Glyphe rasterise + cache
// ----------------------------------------------------------------------------
#[derive(Clone)]
struct Glyph {
    w: usize,
    h: usize,
    left: i32,    // decalage X depuis la plume
    top: i32,     // hauteur au-dessus de la ligne de base
    advance: i32, // avance en pixels
    cov: Vec<u8>, // couverture alpha (w*h)
}

type Edge = (f32, f32, f32, f32);

fn flatten_quad(edges: &mut Vec<Edge>, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)) {
    let n = 8;
    let mut prev = p0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        let x = mt * mt * p0.0 + 2.0 * mt * t * p1.0 + t * t * p2.0;
        let y = mt * mt * p0.1 + 2.0 * mt * t * p1.1 + t * t * p2.1;
        edges.push((prev.0, prev.1, x, y));
        prev = (x, y);
    }
}

// Transforme un contour (points on/off, y up) en aretes scalees (y up).
fn flatten_contour(pts: &[(f32, f32, bool)], scale: f32, edges: &mut Vec<Edge>) {
    let n = pts.len();
    if n < 2 { return; }
    let sp = |q: (f32, f32, bool)| (q.0 * scale, q.1 * scale, q.2);
    let s = pts.iter().position(|q| q.2);
    let begin = s.unwrap_or(0);
    let start = match s {
        Some(i) => (pts[i].0 * scale, pts[i].1 * scale),
        None => (((pts[0].0 + pts[n - 1].0) * 0.5) * scale, ((pts[0].1 + pts[n - 1].1) * 0.5) * scale),
    };
    let mut cur = start;
    let mut ctrl: Option<(f32, f32)> = None;
    for j in 1..n {
        let (px, py, on) = sp(pts[(begin + j) % n]);
        if on {
            match ctrl.take() {
                Some(c) => flatten_quad(edges, cur, c, (px, py)),
                None => edges.push((cur.0, cur.1, px, py)),
            }
            cur = (px, py);
        } else {
            match ctrl.take() {
                Some(c) => { let mid = ((c.0 + px) * 0.5, (c.1 + py) * 0.5); flatten_quad(edges, cur, c, mid); cur = mid; ctrl = Some((px, py)); }
                None => ctrl = Some((px, py)),
            }
        }
    }
    match ctrl {
        Some(c) => flatten_quad(edges, cur, c, start),
        None => edges.push((cur.0, cur.1, start.0, start.1)),
    }
}

// Remplissage scanline (regle non-zero) avec 4 sous-lignes verticales et
// couverture horizontale fractionnaire.
fn rasterize(edges: &[Edge], w: usize, h: usize) -> Vec<u8> {
    let mut acc = vec![0f32; w * h];
    const SS: usize = 4;
    let inv = 1.0 / SS as f32;
    let mut xs: Vec<(f32, i32)> = Vec::new();
    for py in 0..h {
        for sub in 0..SS {
            let sy = py as f32 + (sub as f32 + 0.5) * inv;
            xs.clear();
            for &(x0, y0, x1, y1) in edges {
                let (ya, yb, xa, xb, dir) = if y0 < y1 { (y0, y1, x0, x1, 1) } else { (y1, y0, x1, x0, -1) };
                if sy >= ya && sy < yb && yb > ya {
                    let t = (sy - ya) / (yb - ya);
                    xs.push((xa + (xb - xa) * t, dir));
                }
            }
            if xs.len() < 2 { continue; }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
            let mut wind = 0i32;
            let mut i = 0;
            while i + 1 < xs.len() {
                wind += xs[i].1;
                if wind != 0 {
                    add_span(&mut acc, py, w, xs[i].0, xs[i + 1].0, inv);
                }
                i += 1;
            }
        }
    }
    acc.iter().map(|&c| { let v = c * 255.0; if v < 0.0 { 0 } else if v > 255.0 { 255 } else { v as u8 } }).collect()
}

fn add_span(acc: &mut [f32], row: usize, w: usize, x0: f32, x1: f32, weight: f32) {
    let mut a = if x0 < 0.0 { 0.0 } else { x0 };
    let b = if x1 > w as f32 { w as f32 } else { x1 };
    if b <= a { return; }
    while a < b {
        let xi = a as usize; // a >= 0 -> troncature = floor
        if xi >= w { break; }
        let nx = ((xi + 1) as f32).min(b);
        acc[row * w + xi] += (nx - a) * weight;
        a = nx;
    }
}

fn raster_glyph(f: &Font, gid: u16, px: i32) -> Glyph {
    let advance = (f.advance(gid) as f32 * px as f32 / f.upem + 0.5) as i32;
    let scale = px as f32 / f.upem;
    let contours = f.outline(gid, 0);
    let mut edges: Vec<Edge> = Vec::new();
    let (mut minx, mut maxx, mut miny, mut maxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for c in &contours {
        let before = edges.len();
        flatten_contour(c, scale, &mut edges);
        for &(x0, y0, x1, y1) in &edges[before..] {
            for &(x, y) in &[(x0, y0), (x1, y1)] {
                if x < minx { minx = x; } if x > maxx { maxx = x; }
                if y < miny { miny = y; } if y > maxy { maxy = y; }
            }
        }
    }
    if edges.is_empty() || maxx <= minx || maxy <= miny {
        return Glyph { w: 0, h: 0, left: 0, top: 0, advance, cov: Vec::new() };
    }
    let left = ifloor(minx);
    let top = iceil(maxy);
    let right = iceil(maxx);
    let bottom = ifloor(miny);
    let w = (right - left).max(0) as usize;
    let h = (top - bottom).max(0) as usize;
    if w == 0 || h == 0 || w > 256 || h > 256 {
        return Glyph { w: 0, h: 0, left: 0, top: 0, advance, cov: Vec::new() };
    }
    // aretes en coordonnees raster (y vers le bas, origine coin haut-gauche).
    let redges: Vec<Edge> = edges.iter().map(|&(x0, y0, x1, y1)| {
        (x0 - left as f32, top as f32 - y0, x1 - left as f32, top as f32 - y1)
    }).collect();
    let cov = rasterize(&redges, w, h);
    Glyph { w, h, left, top, advance, cov }
}

// ----------------------------------------------------------------------------
// Etat global (mono-thread, comme le reste du noyau)
// ----------------------------------------------------------------------------
static mut FONT: Option<Font> = None;
static mut TRIED: bool = false;
static mut CACHE: Option<BTreeMap<(u16, i32), Glyph>> = None;

fn font() -> Option<&'static Font> {
    unsafe {
        if !TRIED {
            TRIED = true;
            FONT = Font::parse(FONT_DATA);
            CACHE = Some(BTreeMap::new());
        }
        FONT.as_ref()
    }
}

/// La police vectorielle est-elle disponible ?
pub fn ready() -> bool { font().is_some() }

/// Avance (largeur) d'un caractere en pixels a la taille `px`. Repli monospace
/// (`px` ~ 8*scale = une cellule bitmap) si la police vectorielle est absente.
pub fn char_width(c: char, px: i32) -> i32 {
    match font() {
        Some(f) => (f.advance(f.glyph_id(c)) as f32 * px as f32 / f.upem + 0.5) as i32,
        None => px,
    }
}

/// Largeur totale d'une chaine en pixels. Repli monospace (`px` par caractere).
pub fn text_width(s: &str, px: i32) -> i32 {
    match font() {
        Some(f) => {
            let mut w = 0i32;
            for c in s.chars() { w += (f.advance(f.glyph_id(c)) as f32 * px as f32 / f.upem + 0.5) as i32; }
            w
        }
        None => px * s.chars().count() as i32,
    }
}

fn blit_glyph(g: &Glyph, gx0: i32, gy0: i32, rgb: u32, bold: bool) {
    if g.w == 0 || g.h == 0 { return; }
    for ry in 0..g.h {
        let py = gy0 + ry as i32;
        if py < 0 || py as usize >= fb::HEIGHT { continue; }
        for rx in 0..g.w {
            let a = g.cov[ry * g.w + rx];
            if a == 0 { continue; }
            let pxs = gx0 + rx as i32;
            if pxs >= 0 && (pxs as usize) < fb::WIDTH {
                fb::blend_rgb(pxs as usize, py as usize, rgb, a);
                if bold && (pxs as usize) + 1 < fb::WIDTH { fb::blend_rgb(pxs as usize + 1, py as usize, rgb, a); }
            }
        }
    }
}

/// Dessine `s` a la couleur `rgb`, sommet du texte a `y_top`, taille `px`.
/// Renvoie `false` si la police vectorielle est indisponible (repli appelant).
pub fn draw_text(x: i32, y_top: i32, s: &str, rgb: u32, px: i32, bold: bool) -> bool {
    let f = match font() { Some(f) => f, None => return false };
    let scale = px as f32 / f.upem;
    let baseline = y_top + (f.ascent * scale + 0.5) as i32;
    let mut pen = x;
    for ch in s.chars() {
        let gid = f.glyph_id(ch);
        unsafe {
            let cache = match CACHE.as_mut() { Some(c) => c, None => return true };
            if !cache.contains_key(&(gid, px)) {
                let gl = raster_glyph(f, gid, px);
                cache.insert((gid, px), gl);
            }
            let g = cache.get(&(gid, px)).unwrap();
            blit_glyph(g, pen + g.left, baseline - g.top, rgb, bold);
            pen += g.advance;
        }
    }
    true
}
