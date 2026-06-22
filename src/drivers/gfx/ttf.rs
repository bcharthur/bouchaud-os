//! Rendu de texte TrueType anti-aliasé via fontdue (no_std + alloc).
//!
//! Deux modes :
//!  - **cell**  : avance fixe `cell_w` px (compatibilité layouts OS/GUI)
//!  - **prop**  : avance naturelle de la police (navigateur, contenu web)
//!
//! Chargement paresseux : chaque police est parsée la première fois qu'un
//! glyphe de cette police est demandé, évitant l'épuisement du tas au boot.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use fontdue::{Font, FontSettings};

pub const SANS:      usize = 0;
pub const SANS_BOLD: usize = 1;
pub const MONO:      usize = 2;
pub const MONO_BOLD: usize = 3;

static SANS_TTF:      &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");
static SANS_BOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans-Bold.ttf");
static MONO_TTF:      &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");
static MONO_BOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono-Bold.ttf");

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
    sans:      Option<Font>,
    sans_bold: Option<Font>,
    mono:      Option<Font>,
    mono_bold: Option<Font>,
    cache:     BTreeMap<u64, CachedGlyph>,
}

static mut SYS: Option<TtfSystem> = None;

/// Initialise la structure TTF (aucun parsing de police au boot).
pub fn init() {
    unsafe {
        if SYS.is_some() { return; }
        SYS = Some(TtfSystem {
            sans: None, sans_bold: None, mono: None, mono_bold: None,
            cache: BTreeMap::new(),
        });
    }
}

fn sys() -> &'static mut TtfSystem {
    unsafe { SYS.as_mut().expect("ttf not initialised") }
}

fn font_bytes(font_id: usize) -> &'static [u8] {
    match font_id {
        SANS_BOLD => SANS_BOLD_TTF,
        MONO      => MONO_TTF,
        MONO_BOLD => MONO_BOLD_TTF,
        _         => SANS_TTF,
    }
}

// Charge la police si elle n'est pas encore en mémoire.
fn load_font(sys: &mut TtfSystem, font_id: usize) {
    let already = match font_id {
        SANS_BOLD => sys.sans_bold.is_some(),
        MONO      => sys.mono.is_some(),
        MONO_BOLD => sys.mono_bold.is_some(),
        _         => sys.sans.is_some(),
    };
    if already { return; }
    let font = Font::from_bytes(font_bytes(font_id), FontSettings::default()).ok();
    match font_id {
        SANS_BOLD => sys.sans_bold = font,
        MONO      => sys.mono      = font,
        MONO_BOLD => sys.mono_bold = font,
        _         => sys.sans      = font,
    }
}

fn cache_key(c: char, font_id: usize, px_q: u32) -> u64 {
    (c as u64) | ((font_id as u64) << 21) | ((px_q as u64) << 24)
}

// Assure que le glyphe est dans le cache (police déjà chargée par `load_font`).
fn ensure_glyph(sys: &mut TtfSystem, c: char, font_id: usize, px: f32) -> u64 {
    let key = cache_key(c, font_id, (px * 4.0) as u32);
    if sys.cache.contains_key(&key) { return key; }

    // Emprunts disjoints : borrow immutable de la police → released → borrow mutable du cache.
    let cached: Option<CachedGlyph> = {
        let font: Option<&Font> = match font_id {
            SANS_BOLD => sys.sans_bold.as_ref(),
            MONO      => sys.mono.as_ref(),
            MONO_BOLD => sys.mono_bold.as_ref(),
            _         => sys.sans.as_ref(),
        };
        font.map(|f| {
            let (m, bitmap) = f.rasterize(c, px);
            CachedGlyph {
                bitmap, w: m.width, h: m.height,
                xmin: m.xmin, ymin: m.ymin, advance: m.advance_width,
            }
        })
    };
    if let Some(g) = cached {
        sys.cache.insert(key, g);
    }
    key
}

fn ascent(sys: &TtfSystem, font_id: usize, px: f32) -> i32 {
    let font: Option<&Font> = match font_id {
        SANS_BOLD => sys.sans_bold.as_ref(),
        MONO      => sys.mono.as_ref(),
        MONO_BOLD => sys.mono_bold.as_ref(),
        _         => sys.sans.as_ref(),
    };
    font.and_then(|f| f.horizontal_line_metrics(px))
        .map(|m| ceil(m.ascent))
        .unwrap_or((px * 0.78_f32) as i32)
}

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
pub fn draw_cell(buf: &mut [u32], x: usize, y: usize, s: &str, fg: u32,
                 cell_w: usize, font_id: usize, px: f32) {
    let sys = sys();
    load_font(sys, font_id);
    let baseline_y = y as i32 + ascent(sys, font_id, px);
    let mut cx = x as i32;
    for ch in s.chars() {
        let key = ensure_glyph(sys, ch, font_id, px);
        if let Some(g) = sys.cache.get(&key) {
            blit_glyph(buf, g, cx, baseline_y, fg);
        }
        cx += cell_w as i32;
    }
}

/// Dessine `s` en mode **proportionnel**.
/// Retourne la position X finale.
pub fn draw_prop(buf: &mut [u32], x: usize, y: usize, s: &str, fg: u32,
                 font_id: usize, px: f32) -> usize {
    let sys = sys();
    load_font(sys, font_id);
    let baseline_y = y as i32 + ascent(sys, font_id, px);
    let mut cx = x as i32;
    for ch in s.chars() {
        let key = ensure_glyph(sys, ch, font_id, px);
        if let Some(g) = sys.cache.get(&key) {
            blit_glyph(buf, g, cx, baseline_y, fg);
            cx += ceil(g.advance);
        }
    }
    cx.max(0) as usize
}

/// Largeur en pixels d'une chaîne avec avance proportionnelle (sans dessin).
pub fn str_width(s: &str, font_id: usize, px: f32) -> usize {
    let sys = sys();
    load_font(sys, font_id);
    let mut w = 0i32;
    for ch in s.chars() {
        let key = ensure_glyph(sys, ch, font_id, px);
        if let Some(g) = sys.cache.get(&key) {
            w += ceil(g.advance);
        }
    }
    w.max(0) as usize
}
