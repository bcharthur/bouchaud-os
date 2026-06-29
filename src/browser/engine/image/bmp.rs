//! BMP Windows (BITMAPINFOHEADER). Supporte 24/32 bits non compressés et 8 bits
//! palettisés (BI_RGB). Les lignes sont stockées de bas en haut (origine
//! bottom-left) sauf hauteur négative. RLE et 16 bits → None (repli propre).

use super::{composite_rgba, Image};
use alloc::vec;
use alloc::vec::Vec;

fn rd16(d: &[u8], i: usize) -> usize { (d[i] as usize) | ((d[i + 1] as usize) << 8) }
fn rd32(d: &[u8], i: usize) -> usize {
    (d[i] as usize) | ((d[i + 1] as usize) << 8) | ((d[i + 2] as usize) << 16) | ((d[i + 3] as usize) << 24)
}

pub fn decode(data: &[u8]) -> Option<Image> {
    if data.len() < 54 { return None; }
    let pixoff = rd32(data, 10);
    let hdr = rd32(data, 14);
    if hdr < 40 { return None; } // exige au moins BITMAPINFOHEADER
    let width = rd32(data, 18) as i32;
    let raw_h = rd32(data, 22) as i32;
    let top_down = raw_h < 0;
    let height = raw_h.unsigned_abs() as usize;
    let width = width.unsigned_abs() as usize;
    let bpp = rd16(data, 28);
    let compression = rd32(data, 30);
    if width == 0 || height == 0 || width > 4096 || height > 4096 { return None; }
    if width.checked_mul(height)? > 1_200_000 { return None; }
    if compression != 0 { return None; } // BI_RGB seulement

    // Palette (8 bits) : juste après l'en-tête.
    let mut palette: Vec<u32> = Vec::new();
    if bpp == 8 {
        let mut p = 14 + hdr;
        let mut ncol = rd32(data, 46);
        if ncol == 0 { ncol = 256; }
        for _ in 0..ncol {
            if p + 4 > data.len() { break; }
            // stocké BGRA
            palette.push(((data[p + 2] as u32) << 16) | ((data[p + 1] as u32) << 8) | data[p] as u32);
            p += 4;
        }
    } else if bpp != 24 && bpp != 32 {
        return None;
    }

    let row_bytes = ((width * bpp + 31) / 32) * 4; // padding 4 octets
    let mut pix = vec![0u32; width * height];
    for y in 0..height {
        let src_row = if top_down { y } else { height - 1 - y };
        let base = pixoff + src_row * row_bytes;
        if base + row_bytes > data.len() { break; }
        let row = &data[base..base + row_bytes];
        for x in 0..width {
            let rgb = match bpp {
                32 => {
                    let o = x * 4;
                    let b = row[o] as u32;
                    let g = row[o + 1] as u32;
                    let r = row[o + 2] as u32;
                    let a = row[o + 3] as u32;
                    // alpha 0 sur de nombreux BMP 32 bits = opaque ; si présent, compose.
                    if a == 0 { (r << 16) | (g << 8) | b } else { composite_rgba(r, g, b, a) }
                }
                24 => {
                    let o = x * 3;
                    let b = row[o] as u32;
                    let g = row[o + 1] as u32;
                    let r = row[o + 2] as u32;
                    (r << 16) | (g << 8) | b
                }
                _ => {
                    let idx = row[x] as usize;
                    *palette.get(idx).unwrap_or(&0)
                }
            };
            pix[y * width + x] = rgb & 0x00ff_ffff;
        }
    }
    Some(Image { w: width, h: height, pix })
}
