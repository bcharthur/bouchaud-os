//! Décodage d'images pour le navigateur — un module par format.
//!
//! Sortie commune : pixels `0x00RRGGBB` (alpha composité sur fond blanc, car le
//! framebuffer est opaque). Le moteur web downscale ensuite à la largeur utile,
//! ce qui donne un rendu volontairement pixelisé, dans la même DA que le texte.
//!
//!   - `png`  : PNG non entrelacé (toutes profondeurs, gris/RGB/palette/alpha) ;
//!   - `jpeg` : JPEG baseline (SOF0), gris ou YCbCr, sous-échantillonnage, restart ;
//!   - `gif`  : GIF87a/89a, première image (LZW) ;
//!   - `bmp`  : BMP Windows (BITMAPINFOHEADER) 24/32/8 bits ;
//!   - `webp` : détection (VP8/VP8L/VP8X) — décodage non supporté, repli propre ;
//!   - SVG délègue à `super::svg`.

pub mod png;
pub mod jpeg;
pub mod gif;
pub mod bmp;
pub mod webp;

use alloc::vec;
use alloc::vec::Vec;

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub pix: Vec<u32>, // w*h, 0x00RRGGBB
}

/// Décode une image. Reconnaît PNG, JPEG, GIF, BMP, (WebP détecté) et SVG ;
/// renvoie None si le format est inconnu ou non supporté (repli propre).
pub fn decode(data: &[u8]) -> Option<Image> {
    if data.len() > 8 && &data[..8] == &[137, 80, 78, 71, 13, 10, 26, 10] {
        return png::decode(data);
    }
    if data.len() > 3 && data[0] == 0xFF && data[1] == 0xD8 {
        return jpeg::decode(data);
    }
    if data.len() > 6 && (&data[..6] == b"GIF87a" || &data[..6] == b"GIF89a") {
        return gif::decode(data);
    }
    if data.len() > 2 && data[0] == b'B' && data[1] == b'M' {
        return bmp::decode(data);
    }
    if data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return webp::decode(data);
    }
    if looks_like_svg(data) {
        return super::svg::rasterize(data);
    }
    None
}

/// Détecte un document SVG : `<svg` (éventuellement précédé de `<?xml`, d'un BOM
/// ou d'espaces) dans le préfixe.
fn looks_like_svg(data: &[u8]) -> bool {
    let n = data.len().min(512);
    let head = &data[..n];
    if head.len() < 4 { return false; }
    for w in head.windows(4) {
        if w[0] == b'<' && (w[1] | 32) == b's' && (w[2] | 32) == b'v' && (w[3] | 32) == b'g' {
            return true;
        }
    }
    false
}

// ── Helpers partagés entre formats ──────────────────────────────────────────

/// Lit un entier big-endian 32 bits (PNG, RIFF…).
pub(crate) fn be32(d: &[u8], i: usize) -> usize {
    ((d[i] as usize) << 24) | ((d[i + 1] as usize) << 16) | ((d[i + 2] as usize) << 8) | d[i + 3] as usize
}

/// Composite un canal `v` (max `maxv`) avec un alpha `a` (max `maxv`) sur fond
/// blanc (255). Renvoie un canal 8 bits.
pub(crate) fn composite(v: u32, a: u32, maxv: u32) -> u32 {
    let v = if maxv == 255 { v } else { v * 255 / maxv };
    let a = if maxv == 255 { a } else { a * 255 / maxv };
    (v * a + 255 * (255 - a)) / 255
}

/// Composite un pixel RGBA 8 bits sur fond blanc -> `0x00RRGGBB`.
pub(crate) fn composite_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    let f = |c: u32| (c * a + 255 * (255 - a)) / 255;
    (f(r) << 16) | (f(g) << 8) | f(b)
}

/// Réduit l'image à au plus `max_w` x `max_h` (plus proche voisin, pixelisé).
pub fn downscale(img: &Image, max_w: usize, max_h: usize) -> Image {
    if img.w == 0 || img.h == 0 { return Image { w: 0, h: 0, pix: Vec::new() }; }
    if img.w <= max_w && img.h <= max_h {
        return Image { w: img.w, h: img.h, pix: img.pix.clone() };
    }
    let sx = img.w as u64 * 1000 / (max_w.max(1) as u64);
    let sy = img.h as u64 * 1000 / (max_h.max(1) as u64);
    let s = sx.max(sy).max(1000); // garde le ratio, ne grossit pas
    let nw = (img.w as u64 * 1000 / s).max(1) as usize;
    let nh = (img.h as u64 * 1000 / s).max(1) as usize;
    let mut pix = vec![0u32; nw * nh];
    for y in 0..nh {
        let srcy = (y as u64 * s / 1000) as usize;
        for x in 0..nw {
            let srcx = (x as u64 * s / 1000) as usize;
            let si = srcy.min(img.h - 1) * img.w + srcx.min(img.w - 1);
            pix[y * nw + x] = img.pix[si];
        }
    }
    Image { w: nw, h: nh, pix }
}
