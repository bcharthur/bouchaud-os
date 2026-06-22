//! Rendu de texte TrueType anti-aliasé via fontdue (no_std + alloc).
//!
//! Deux modes :
//!  - **cell**  : avance fixe `cell_w` px (compatibilité layouts OS/GUI)
//!  - **prop**  : avance naturelle de la police (navigateur, contenu web)
//!
//! Les quatre polices DejaVu sont embarquées dans le binaire et rastérisées
//! à la demande ; les glyphes rastérisés sont mis en cache dans un BTreeMap.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use fontdue::{Font, FontSettings};

// Identifiants de police (à passer en `font_id`).
pub const SANS:      usize = 0;
pub const SANS_BOLD: usize = 1;
pub const MONO:      usize = 2;
pub const MONO_BOLD: usize = 3;

static SANS_TTF:      &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");
static SANS_BOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans-Bold.ttf");
static MONO_TTF:      &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");
static MONO_BOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono-Bold.ttf");

// ceil(x) pour f32 positif (pas de libm requis en no_std)
#[inline]
fn ceil(x: f32) -> i32 {
    let i = x as i32;
    if x > i as f32 { i + 1 } else { i }
}

struct CachedGlyph {
    bitmap:  Vec<u8>,
    w:       usize,
    h:       usize,
    xmin:    i32,
    ymin:    i32,
    advance: f32,
}

struct TtfSystem {
    fonts:  [Font; 4],
    cache:  BTreeMap<u64, CachedGlyph>,
}

static mut SYS: Option<TtfSystem> = None;

/// Initialise le système TTF (appelé une seule fois depuis `gfx::enter`).
pub fn init() {
    unsafe {
        if SYS.is_some() { return; }
        let s = FontSettings::default();
        let f0 = Font::from_bytes(SANS_TTF,      s).unwrap_or_else(|_| panic!("DejaVuSans"));
        let f1 = Font::from_bytes(SANS_BOLD_TTF, s).unwrap_or_else(|_| panic!("DejaVuSans-Bold"));
        let f2 = Font::from_bytes(MONO_TTF,      s).unwrap_or_else(|_| panic!("DejaVuSansMono"));
        let f3 = Font::from_bytes(MONO_BOLD_TTF, s).unwrap_or_else(|_| panic!("DejaVuSansMono-Bold"));
        SYS = Some(TtfSystem { fonts: [f0, f1, f2, f3], cache: BTreeMap::new() });
    }
}

fn sys() -> &'static mut TtfSystem {
    unsafe { SYS.as_mut().expect("ttf not initialised") }
}

// Clé de cache : 21 bits char | 3 bits font_id | 8+ bits taille (×4 pour 0.25px)
fn cache_key(c: char, font_id: usize, px_q: u32) -> u64 {
    (c as u64) | ((font_id as u64) << 21) | ((px_q as u64) << 24)
}

// Assure que le glyphe est dans le cache ; renvoie la clé.
fn ensure_glyph(sys: &mut TtfSystem, c: char, font_id: usize, px: f32) -> u64 {
    let key = cache_key(c, font_id, (px * 4.0) as u32);
    // Emprunts disjoints sur les deux champs : fonts (immutable) / cache (mutable).
    let fonts = &sys.fonts;
    sys.cache.entry(key).or_insert_with(|| {
        let (m, bitmap) = fonts[font_id].rasterize(c, px);
        CachedGlyph { bitmap, w: m.width, h: m.height, xmin: m.xmin, ymin: m.ymin, advance: m.advance_width }
    });
    key
}

// Hauteur au-dessus de la ligne de base pour cette police/taille.
fn ascent(sys: &TtfSystem, font_id: usize, px: f32) -> i32 {
    sys.fonts[font_id]
        .horizontal_line_metrics(px)
        .map(|m| ceil(m.ascent))
        .unwrap_or((px * 0.78_f32) as i32)
}

// Mélange un pixel en alpha-blending (coverage 0..255) sur le back-buffer.
#[inline]
fn blend(buf: &mut [u32], x: usize, y: usize, cov: u8, fg: u32) {
    use super::{WIDTH, HEIGHT};
    if x >= WIDTH || y >= HEIGHT { return; }
    let idx = y * WIDTH + x;
    if cov == 255 { buf[idx] = fg; return; }
    if cov == 0   { return; }
    let bg = buf[idx];
    let a  = cov as u32;
    let ia = 255 - a;
    let r = ((fg >> 16 & 0xff) * a + (bg >> 16 & 0xff) * ia) / 255;
    let g = ((fg >>  8 & 0xff) * a + (bg >>  8 & 0xff) * ia) / 255;
    let b = ((fg       & 0xff) * a + (bg       & 0xff) * ia) / 255;
    buf[idx] = (r << 16) | (g << 8) | b;
}

// Dessine un seul glyphe en (cx, baseline_y) écran.
fn blit_glyph(buf: &mut [u32], g: &CachedGlyph, cx: i32, baseline_y: i32, fg: u32) {
    let gx0 = cx + g.xmin;
    let gy0 = baseline_y - g.h as i32 - g.ymin;
    for row in 0..g.h {
        for col in 0..g.w {
            let cov = g.bitmap[row * g.w + col];
            if cov == 0 { continue; }
            let px = gx0 + col as i32;
            let py = gy0 + row as i32;
            if px >= 0 && py >= 0 {
                blend(buf, px as usize, py as usize, cov, fg);
            }
        }
    }
}

/// Dessine `s` en mode **cellule** : chaque caractère avance de `cell_w` px.
/// Compatible avec les layouts fixes de l'OS (terminal, barres, titres...).
pub fn draw_cell(buf: &mut [u32], x: usize, y: usize, s: &str, fg: u32,
                 cell_w: usize, font_id: usize, px: f32) {
    let sys = sys();
    let baseline_y = y as i32 + ascent(sys, font_id, px);
    let mut cx = x as i32;
    for ch in s.chars() {
        let key = ensure_glyph(sys, ch, font_id, px);
        let g = sys.cache.get(&key).unwrap();
        blit_glyph(buf, g, cx, baseline_y, fg);
        cx += cell_w as i32;
    }
}

/// Dessine `s` en mode **proportionnel** (avance naturelle fontdue).
/// Retourne la position X finale (utile pour chaîner plusieurs segments).
pub fn draw_prop(buf: &mut [u32], x: usize, y: usize, s: &str, fg: u32,
                 font_id: usize, px: f32) -> usize {
    let sys = sys();
    let baseline_y = y as i32 + ascent(sys, font_id, px);
    let mut cx = x as i32;
    for ch in s.chars() {
        let key = ensure_glyph(sys, ch, font_id, px);
        let g = sys.cache.get(&key).unwrap();
        blit_glyph(buf, g, cx, baseline_y, fg);
        cx += ceil(g.advance);
    }
    cx.max(0) as usize
}

/// Largeur en pixels d'une chaîne avec avance proportionnelle (sans dessin).
pub fn str_width(s: &str, font_id: usize, px: f32) -> usize {
    let sys = sys();
    let mut w = 0i32;
    for ch in s.chars() {
        let key = ensure_glyph(sys, ch, font_id, px);
        let g = sys.cache.get(&key).unwrap();
        w += ceil(g.advance);
    }
    w.max(0) as usize
}
