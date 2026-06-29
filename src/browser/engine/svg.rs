//! Rasteriseur SVG minimal pour le navigateur Nautile.
//!
//! Le web moderne utilise SVG pour la quasi-totalite des icones et logos. Ce
//! module transforme un document SVG (texte XML) en image matricielle
//! `0x00RRGGBB` (composite sur blanc), exploitable par le pipeline d'images au
//! meme titre que PNG/JPEG.
//!
//! Couverture : `viewBox`, formes `<rect>/<circle>/<ellipse>/<line>/<polygon>/
//! <polyline>/<path>` ; commandes de chemin `M L H V C S Q T A Z` (courbes et
//! arcs aplatis en segments) ; remplissage (regle non-zero) et contour epais
//! (`stroke`/`stroke-width`). Couleurs hex/nommees/`none`/`currentColor`.
//! Non geres : `transform`, gradients, `<use>`, `<text>`, animations.

use alloc::vec;
use alloc::vec::Vec;
use alloc::string::String;
use super::image::Image;

const MAX_DIM: usize = 96;       // cote max du rendu (les icones sont petites)
const CURVE_STEPS: usize = 10;   // segments par courbe de Bezier
const ARC_STEPS: usize = 24;     // segments par arc complet

/// Rasterise un document SVG. Renvoie None si non-SVG ou illisible.
pub fn rasterize(data: &[u8]) -> Option<Image> {
    let text = core::str::from_utf8(data).ok().map(String::from)
        .unwrap_or_else(|| String::from_utf8_lossy(data).into_owned());
    let svg_pos = find_ci(&text, "<svg")?;
    let svg_tag = read_tag(&text[svg_pos..])?; // attributs de <svg ...>

    // Repere de coordonnees : viewBox prioritaire, sinon width/height.
    let (vbx, vby, vbw, vbh) = if let Some(vb) = attr(&svg_tag, "viewBox") {
        let n = nums(vb);
        if n.len() >= 4 && n[2] > 0.0 && n[3] > 0.0 { (n[0], n[1], n[2], n[3]) }
        else { dims_fallback(&svg_tag) }
    } else { dims_fallback(&svg_tag) };
    if vbw <= 0.0 || vbh <= 0.0 { return None; }

    // Taille de sortie (cap MAX_DIM, ratio conserve).
    let scale = (MAX_DIM as f32 / vbw.max(vbh)).min(1.0).max(0.02);
    let ow = (ceilf(vbw * scale) as usize).clamp(1, MAX_DIM);
    let oh = (ceilf(vbh * scale) as usize).clamp(1, MAX_DIM);
    let mut img = Image { w: ow, h: oh, pix: vec![0x00ffffff; ow * oh] };

    let tf = |x: f32, y: f32| -> (f32, f32) { ((x - vbx) * scale, (y - vby) * scale) };

    // Parcourt les balises de forme dans l'ordre du document (painter's algorithm).
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut shapes = 0u32;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(tag) = read_tag(&text[i..]) {
                let name = tag_name(&tag);
                let subpaths = match name {
                    "rect" => rect_path(&tag),
                    "circle" => circle_path(&tag),
                    "ellipse" => ellipse_path(&tag),
                    "line" => line_path(&tag),
                    "polygon" => poly_path(&tag, true),
                    "polyline" => poly_path(&tag, false),
                    "path" => attr(&tag, "d").map(parse_path).unwrap_or_default(),
                    _ => Vec::new(),
                };
                if !subpaths.is_empty() {
                    // Transforme en coords ecran.
                    let polys: Vec<Vec<(f32, f32)>> = subpaths.iter()
                        .map(|sp| sp.iter().map(|&(x, y)| tf(x, y)).collect())
                        .collect();
                    if let Some(c) = fill_color(&tag) { fill_polys(&mut img, &polys, c); }
                    if let Some((c, w)) = stroke_color(&tag) {
                        stroke_polys(&mut img, &polys, c, (w * scale).max(1.0));
                    }
                    shapes += 1;
                    if shapes > 4000 { break; }
                }
            }
        }
        i += 1;
    }
    Some(img)
}

// ── Formes -> sous-chemins (listes de points) ─────────────────────────────────

fn rect_path(t: &str) -> Vec<Vec<(f32, f32)>> {
    let x = a_num(t, "x"); let y = a_num(t, "y");
    let w = a_num(t, "width"); let h = a_num(t, "height");
    if w <= 0.0 || h <= 0.0 { return Vec::new(); }
    vec![vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]]
}

fn circle_path(t: &str) -> Vec<Vec<(f32, f32)>> {
    ellipse_pts(a_num(t, "cx"), a_num(t, "cy"), a_num(t, "r"), a_num(t, "r"))
}
fn ellipse_path(t: &str) -> Vec<Vec<(f32, f32)>> {
    ellipse_pts(a_num(t, "cx"), a_num(t, "cy"), a_num(t, "rx"), a_num(t, "ry"))
}
fn ellipse_pts(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<Vec<(f32, f32)>> {
    if rx <= 0.0 || ry <= 0.0 { return Vec::new(); }
    let n = 40;
    let mut p = Vec::with_capacity(n + 1);
    for k in 0..=n {
        let a = k as f32 / n as f32 * core::f32::consts::TAU;
        p.push((cx + rx * cosf(a), cy + ry * sinf(a)));
    }
    vec![p]
}
fn line_path(t: &str) -> Vec<Vec<(f32, f32)>> {
    vec![vec![(a_num(t, "x1"), a_num(t, "y1")), (a_num(t, "x2"), a_num(t, "y2"))]]
}
fn poly_path(t: &str, close: bool) -> Vec<Vec<(f32, f32)>> {
    let pts = match attr(t, "points") { Some(p) => nums(p), None => return Vec::new() };
    let mut p: Vec<(f32, f32)> = pts.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect();
    if p.len() < 2 { return Vec::new(); }
    if close { let first = p[0]; p.push(first); }
    vec![p]
}

// ── Parseur de `path d="..."` ─────────────────────────────────────────────────

fn parse_path(d: &str) -> Vec<Vec<(f32, f32)>> {
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);     // position courante
    let (mut sx, mut sy) = (0.0f32, 0.0f32);   // debut du sous-chemin
    let (mut px, mut py) = (0.0f32, 0.0f32);   // point de controle precedent (S/T)
    let toks = tokenize_path(d);
    let mut i = 0usize;
    let mut cmd = b' ';
    while i < toks.len() {
        // Une commande explicite, ou repetition de la precedente avec de nouveaux nombres.
        if let Tok::Cmd(c) = toks[i] { cmd = c; i += 1; }
        let rel = cmd.is_ascii_lowercase();
        let cl = cmd.to_ascii_uppercase();
        macro_rules! n { () => {{ match toks.get(i) { Some(Tok::Num(v)) => { i += 1; *v } _ => break } }}; }
        match cl {
            b'M' => {
                if !cur.is_empty() { out.push(core::mem::take(&mut cur)); }
                let (nx, ny) = (n!(), n!());
                x = if rel { x + nx } else { nx }; y = if rel { y + ny } else { ny };
                sx = x; sy = y; cur.push((x, y));
                // Les paires suivantes d'un M sont des L implicites.
                while matches!(toks.get(i), Some(Tok::Num(_))) {
                    let (lx, ly) = (n!(), n!());
                    x = if rel { x + lx } else { lx }; y = if rel { y + ly } else { ly };
                    cur.push((x, y));
                }
            }
            b'L' => { let (lx, ly) = (n!(), n!()); x = if rel { x + lx } else { lx }; y = if rel { y + ly } else { ly }; cur.push((x, y)); }
            b'H' => { let lx = n!(); x = if rel { x + lx } else { lx }; cur.push((x, y)); }
            b'V' => { let ly = n!(); y = if rel { y + ly } else { ly }; cur.push((x, y)); }
            b'Z' => { cur.push((sx, sy)); x = sx; y = sy; if !cur.is_empty() { out.push(core::mem::take(&mut cur)); } }
            b'C' => {
                let (x1, y1, x2, y2, ex, ey) = (n!(), n!(), n!(), n!(), n!(), n!());
                let (c1x, c1y) = if rel { (x + x1, y + y1) } else { (x1, y1) };
                let (c2x, c2y) = if rel { (x + x2, y + y2) } else { (x2, y2) };
                let (nx, ny) = if rel { (x + ex, y + ey) } else { (ex, ey) };
                cubic(&mut cur, x, y, c1x, c1y, c2x, c2y, nx, ny);
                px = c2x; py = c2y; x = nx; y = ny;
            }
            b'S' => {
                let (x2, y2, ex, ey) = (n!(), n!(), n!(), n!());
                let (c1x, c1y) = (2.0 * x - px, 2.0 * y - py);
                let (c2x, c2y) = if rel { (x + x2, y + y2) } else { (x2, y2) };
                let (nx, ny) = if rel { (x + ex, y + ey) } else { (ex, ey) };
                cubic(&mut cur, x, y, c1x, c1y, c2x, c2y, nx, ny);
                px = c2x; py = c2y; x = nx; y = ny;
            }
            b'Q' => {
                let (x1, y1, ex, ey) = (n!(), n!(), n!(), n!());
                let (cx, cy) = if rel { (x + x1, y + y1) } else { (x1, y1) };
                let (nx, ny) = if rel { (x + ex, y + ey) } else { (ex, ey) };
                quad(&mut cur, x, y, cx, cy, nx, ny);
                px = cx; py = cy; x = nx; y = ny;
            }
            b'T' => {
                let (ex, ey) = (n!(), n!());
                let (cx, cy) = (2.0 * x - px, 2.0 * y - py);
                let (nx, ny) = if rel { (x + ex, y + ey) } else { (ex, ey) };
                quad(&mut cur, x, y, cx, cy, nx, ny);
                px = cx; py = cy; x = nx; y = ny;
            }
            b'A' => {
                let (rx, ry, _rot, _laf, _swf, ex, ey) = (n!(), n!(), n!(), n!(), n!(), n!(), n!());
                let (nx, ny) = if rel { (x + ex, y + ey) } else { (ex, ey) };
                arc(&mut cur, x, y, rx, ry, nx, ny);
                x = nx; y = ny;
            }
            _ => { i += 1; }
        }
        if cl != b'C' && cl != b'S' && cl != b'Q' && cl != b'T' { px = x; py = y; }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

fn cubic(out: &mut Vec<(f32, f32)>, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32) {
    for k in 1..=CURVE_STEPS {
        let t = k as f32 / CURVE_STEPS as f32; let u = 1.0 - t;
        let a = u * u * u; let b = 3.0 * u * u * t; let c = 3.0 * u * t * t; let d = t * t * t;
        out.push((a * x0 + b * x1 + c * x2 + d * x3, a * y0 + b * y1 + c * y2 + d * y3));
    }
}
fn quad(out: &mut Vec<(f32, f32)>, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
    for k in 1..=CURVE_STEPS {
        let t = k as f32 / CURVE_STEPS as f32; let u = 1.0 - t;
        let a = u * u; let b = 2.0 * u * t; let c = t * t;
        out.push((a * x0 + b * x1 + c * x2, a * y0 + b * y1 + c * y2));
    }
}
// Arc approxime : on echantillonne le petit-arc d'une ellipse passant de
// (x0,y0) a (x1,y1). Approximation suffisante pour des icones.
fn arc(out: &mut Vec<(f32, f32)>, x0: f32, y0: f32, rx: f32, ry: f32, x1: f32, y1: f32) {
    if rx <= 0.0 || ry <= 0.0 { out.push((x1, y1)); return; }
    // Centre approximatif = milieu, rayon moyen : echantillonnage circulaire entre
    // les deux extremites. C'est une approximation (ignore rotation/flags) mais
    // rend les courbes lisses des spinners/anneaux.
    let mx = (x0 + x1) / 2.0; let my = (y0 + y1) / 2.0;
    let a0 = atan2f(y0 - my, x0 - mx);
    let mut a1 = atan2f(y1 - my, x1 - mx);
    if a1 < a0 { a1 += core::f32::consts::TAU; }
    let r = sqrtf((x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0)) / 2.0;
    for k in 1..=ARC_STEPS {
        let a = a0 + (a1 - a0) * (k as f32 / ARC_STEPS as f32);
        out.push((mx + r * cosf(a), my + r * sinf(a)));
    }
}

// ── Rasterisation ─────────────────────────────────────────────────────────────

// Remplissage par balayage de lignes (regle non-zero approximee en even-odd-
// tolerant : on compte les croisements et on remplit les segments impairs).
fn fill_polys(img: &mut Image, polys: &[Vec<(f32, f32)>], color: u32) {
    let (w, h) = (img.w, img.h);
    for py in 0..h {
        let yc = py as f32 + 0.5;
        // Croisements de la scanline avec toutes les aretes de tous les sous-chemins.
        let mut xs: Vec<f32> = Vec::new();
        for poly in polys {
            if poly.len() < 2 { continue; }
            for win in poly.windows(2) {
                let (x0, y0) = win[0]; let (x1, y1) = win[1];
                if (y0 <= yc && y1 > yc) || (y1 <= yc && y0 > yc) {
                    let tx = x0 + (yc - y0) / (y1 - y0) * (x1 - x0);
                    xs.push(tx);
                }
            }
            // Ferme implicitement le sous-chemin pour le remplissage.
            let (fx, fy) = poly[0]; let (lx, ly) = poly[poly.len() - 1];
            if (fy <= yc && ly > yc) || (ly <= yc && fy > yc) {
                let tx = lx + (yc - ly) / (fy - ly) * (fx - lx);
                xs.push(tx);
            }
        }
        if xs.len() < 2 { continue; }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let mut k = 0;
        while k + 1 < xs.len() {
            let x0 = xs[k].max(0.0) as i32;
            let x1 = (xs[k + 1].min(w as f32)) as i32;
            for x in x0..x1 { if x >= 0 && (x as usize) < w { img.pix[py * w + x as usize] = color; } }
            k += 2;
        }
    }
}

// Contour epais : chaque arete tracee comme une bande, plus un disque a chaque
// sommet (jonctions rondes approximees).
fn stroke_polys(img: &mut Image, polys: &[Vec<(f32, f32)>], color: u32, width: f32) {
    let r = (width / 2.0).max(0.5);
    for poly in polys {
        for win in poly.windows(2) {
            thick_line(img, win[0], win[1], r, color);
        }
        for &p in poly { disc(img, p, r, color); }
    }
}

fn thick_line(img: &mut Image, a: (f32, f32), b: (f32, f32), r: f32, color: u32) {
    let dx = b.0 - a.0; let dy = b.1 - a.1;
    let len = sqrtf(dx * dx + dy * dy);
    let steps = (ceilf(len) as usize).max(1);
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        disc(img, (a.0 + dx * t, a.1 + dy * t), r, color);
    }
}

fn disc(img: &mut Image, c: (f32, f32), r: f32, color: u32) {
    let (w, h) = (img.w, img.h);
    let r2 = r * r;
    let x0 = floorf(c.0 - r).max(0.0) as i32;
    let x1 = ceilf(c.0 + r).min(w as f32) as i32;
    let y0 = floorf(c.1 - r).max(0.0) as i32;
    let y1 = ceilf(c.1 + r).min(h as f32) as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            let dxp = x as f32 + 0.5 - c.0; let dyp = y as f32 + 0.5 - c.1;
            if dxp * dxp + dyp * dyp <= r2 && x >= 0 && y >= 0 {
                img.pix[y as usize * w + x as usize] = color;
            }
        }
    }
}

// ── Attributs / couleurs / nombres ────────────────────────────────────────────

fn fill_color(t: &str) -> Option<u32> {
    match attr(t, "fill") {
        Some(v) if v.trim() == "none" => None,
        Some(v) => parse_color(v).or(Some(0x000000)),
        None => Some(0x000000), // defaut SVG : fill noir
    }.map(|c| apply_opacity(c, opacity(t, "fill-opacity")))
}

fn stroke_color(t: &str) -> Option<(u32, f32)> {
    match attr(t, "stroke") {
        Some(v) if v.trim() != "none" => {
            let c = parse_color(v).unwrap_or(0x000000);
            let w = attr(t, "stroke-width").and_then(|s| s.trim().trim_end_matches("px").parse::<f32>().ok()).unwrap_or(1.0);
            Some((apply_opacity(c, opacity(t, "stroke-opacity")), w.max(0.1)))
        }
        _ => None,
    }
}

fn opacity(t: &str, name: &str) -> f32 {
    attr(t, name).and_then(|s| s.trim().parse::<f32>().ok())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(1.0)
        .min(attr(t, "opacity").and_then(|s| s.trim().parse::<f32>().ok()).map(|v| v.clamp(0.0, 1.0)).unwrap_or(1.0))
}

// Compose une couleur opaque sur blanc selon une opacite (rendu opaque final).
fn apply_opacity(c: u32, op: f32) -> u32 {
    if op >= 0.999 { return c; }
    let blend = |sh: u32| -> u32 {
        let v = ((c >> sh) & 0xff) as f32;
        (v * op + 255.0 * (1.0 - op)) as u32 & 0xff
    };
    (blend(16) << 16) | (blend(8) << 8) | blend(0)
}

fn dims_fallback(svg_tag: &str) -> (f32, f32, f32, f32) {
    let w = attr(svg_tag, "width").map(strip_unit).unwrap_or(0.0);
    let h = attr(svg_tag, "height").map(strip_unit).unwrap_or(0.0);
    if w > 0.0 && h > 0.0 { (0.0, 0.0, w, h) } else { (0.0, 0.0, 24.0, 24.0) }
}
fn strip_unit(s: &str) -> f32 {
    s.trim().trim_end_matches("px").trim_end_matches("pt").trim().parse::<f32>().unwrap_or(0.0)
}

fn a_num(t: &str, name: &str) -> f32 { attr(t, name).map(strip_unit).unwrap_or(0.0) }

// Liste de nombres d'une chaine (separateurs espace/virgule).
fn nums(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    for tok in s.split(|c: char| c == ',' || c == ' ' || c == '\n' || c == '\t' || c == '\r') {
        if let Ok(v) = tok.trim().parse::<f32>() { out.push(v); }
    }
    out
}

// Couleur SVG : #rgb, #rrggbb, rgb(...), quelques noms, currentColor.
fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if s == "currentColor" || s == "context-fill" { return Some(0x000000); }
    if let Some(h) = s.strip_prefix('#') {
        if h.len() >= 6 { return u32::from_str_radix(&h[..6], 16).ok().map(|v| v & 0xffffff); }
        if h.len() >= 3 {
            let r = u8::from_str_radix(&h[0..1], 16).ok()?;
            let g = u8::from_str_radix(&h[1..2], 16).ok()?;
            let b = u8::from_str_radix(&h[2..3], 16).ok()?;
            return Some(((r as u32 * 17) << 16) | ((g as u32 * 17) << 8) | (b as u32 * 17));
        }
        return None;
    }
    if let Some(rest) = s.strip_prefix("rgb") {
        let inside = rest.trim_start_matches('(').trim_end_matches(')');
        let n = nums(inside);
        if n.len() >= 3 { return Some(((n[0] as u32 & 255) << 16) | ((n[1] as u32 & 255) << 8) | (n[2] as u32 & 255)); }
    }
    Some(match s {
        "black" => 0x000000, "white" => 0xffffff, "red" => 0xff0000, "green" => 0x008000,
        "blue" => 0x0000ff, "gray" | "grey" => 0x808080, "silver" => 0xc0c0c0,
        "yellow" => 0xffff00, "orange" => 0xffa500, "navy" => 0x000080, "purple" => 0x800080,
        "none" => return None, "transparent" => return None,
        _ => return None,
    })
}

// ── Mini-parseur de balises XML ───────────────────────────────────────────────

// Lit la balise a partir de `s[0]=='<'` jusqu'au `>` ; renvoie son contenu
// interne (`tagname attr=...`), sans les chevrons.
fn read_tag(s: &str) -> Option<String> {
    if !s.starts_with('<') { return None; }
    let end = s.find('>')?;
    let inner = &s[1..end];
    let inner = inner.strip_suffix('/').unwrap_or(inner);
    Some(inner.into())
}

fn tag_name(tag: &str) -> &str {
    tag.trim_start().split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("")
}

// Valeur d'un attribut `name="..."` / `name='...'` dans un fragment de balise.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let mut from = 0usize;
    loop {
        let pos = find_ci(&tag[from..], name)? + from;
        // Doit etre une frontiere d'attribut (precede d'espace ou debut).
        let ok_before = pos == 0 || bytes[pos - 1].is_ascii_whitespace();
        let after = pos + name.len();
        if ok_before && after < bytes.len() {
            let mut j = after;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
                let q = bytes.get(j).copied();
                if q == Some(b'"') || q == Some(b'\'') {
                    let quote = q.unwrap() as char;
                    let start = j + 1;
                    if let Some(e) = tag[start..].find(quote) { return Some(&tag[start..start + e]); }
                }
            }
        }
        from = pos + name.len();
        if from >= tag.len() { return None; }
    }
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes(); let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() { return None; }
    let last = h.len() - n.len();
    let mut i = 0;
    while i <= last {
        if h[i..i + n.len()].iter().zip(n).all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase()) { return Some(i); }
        i += 1;
    }
    None
}

// Tokenizer d'un attribut `d` de chemin.
enum Tok { Cmd(u8), Num(f32) }
fn tokenize_path(d: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let b = d.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphabetic() && c != b'e' && c != b'E' {
            out.push(Tok::Cmd(c)); i += 1;
        } else if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() {
            // Lit un nombre (gere notation scientifique et signe).
            let start = i; i += 1;
            while i < b.len() {
                let d2 = b[i];
                if d2.is_ascii_digit() || d2 == b'.' { i += 1; }
                else if (d2 == b'e' || d2 == b'E') && i + 1 < b.len() { i += 1; }
                else if (d2 == b'-' || d2 == b'+') && (b[i - 1] == b'e' || b[i - 1] == b'E') { i += 1; }
                else { break; }
            }
            if let Ok(v) = d[start..i].parse::<f32>() { out.push(Tok::Num(v)); }
        } else { i += 1; }
    }
    out
}

// ── Trigo (no_std, sans libm direct) via approximations suffisantes ───────────
fn cosf(x: f32) -> f32 { sinf(x + core::f32::consts::FRAC_PI_2) }
fn sinf(mut x: f32) -> f32 {
    let tau = core::f32::consts::TAU;
    x %= tau; if x < -core::f32::consts::PI { x += tau; } else if x > core::f32::consts::PI { x -= tau; }
    // Polynome de Bhaskara-like (precision ~1e-3, largement suffisant).
    let b = 4.0 / core::f32::consts::PI; let c = -4.0 / (core::f32::consts::PI * core::f32::consts::PI);
    let y = b * x + c * x * fabs(x);
    0.225 * (y * fabs(y) - y) + y
}
fn atan2f(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 { return 0.0; }
    let ax = fabs(x); let ay = fabs(y);
    let a = ax.min(ay) / ax.max(ay);
    let s = a * a;
    let mut r = ((-0.0464964749 * s + 0.15931422) * s - 0.327622764) * s * a + a;
    if ay > ax { r = core::f32::consts::FRAC_PI_2 - r; }
    if x < 0.0 { r = core::f32::consts::PI - r; }
    if y < 0.0 { r = -r; }
    r
}
fn fabs(x: f32) -> f32 { if x < 0.0 { -x } else { x } }
fn sqrtf(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    let mut g = x; // Newton-Raphson, converge vite pour nos petites valeurs.
    let mut k = 0; while k < 20 { g = 0.5 * (g + x / g); k += 1; }
    g
}
fn floorf(x: f32) -> f32 { let i = x as i32 as f32; if i > x { i - 1.0 } else { i } }
fn ceilf(x: f32) -> f32 { let i = x as i32 as f32; if i < x { i + 1.0 } else { i } }
