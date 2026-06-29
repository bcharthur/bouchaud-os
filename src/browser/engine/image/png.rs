//! Décodage PNG non entrelacé : toutes profondeurs de bits (1/2/4/8/16),
//! types couleur gris, RGB, palette, gris+alpha, RGBA. Défiltrage des scanlines
//! (None/Sub/Up/Average/Paeth) puis composite alpha sur fond blanc.

use super::{be32, composite, Image};
use alloc::vec;
use alloc::vec::Vec;

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

pub fn decode(data: &[u8]) -> Option<Image> {
    let mut i = 8usize;
    let mut width = 0usize;
    let mut height = 0usize;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut palette: Vec<u32> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();

    while i + 8 <= data.len() {
        let len = be32(data, i);
        let ctype = &data[i + 4..i + 8];
        let ds = i + 8;
        if ds + len + 4 > data.len() { break; }
        let chunk = &data[ds..ds + len];
        match ctype {
            b"IHDR" => {
                if len < 13 { return None; }
                width = be32(chunk, 0);
                height = be32(chunk, 4);
                bit_depth = chunk[8];
                color_type = chunk[9];
                interlace = chunk[12];
            }
            b"PLTE" => {
                let mut k = 0;
                while k + 3 <= chunk.len() {
                    palette.push(((chunk[k] as u32) << 16) | ((chunk[k + 1] as u32) << 8) | chunk[k + 2] as u32);
                    k += 3;
                }
            }
            b"tRNS" => { trns = chunk.to_vec(); }
            b"IDAT" => { idat.extend_from_slice(chunk); }
            b"IEND" => break,
            _ => {}
        }
        i = ds + len + 4; // saute le CRC
    }

    if width == 0 || height == 0 || interlace != 0 { return None; }
    if width > 4096 || height > 4096 { return None; }
    // Borne mémoire : ~1,2 Mpx max (sinon placeholder).
    if width.checked_mul(height)? > 1_200_000 { return None; }

    let channels = match color_type { 0 => 1, 2 => 3, 3 => 1, 4 => 2, 6 => 4, _ => return None };
    let bd = bit_depth as usize;
    if !(bd == 1 || bd == 2 || bd == 4 || bd == 8 || bd == 16) { return None; }
    let bpp = ((channels * bd + 7) / 8).max(1);
    let stride = (width * channels * bd + 7) / 8;

    let raw = crate::net::inflate::zlib_decode(&idat).ok()?;
    if raw.len() < (stride + 1) * height { return None; }

    // Défiltrage des scanlines.
    let mut recon = vec![0u8; stride * height];
    let mut pos = 0usize;
    for y in 0..height {
        let ft = raw[pos]; pos += 1;
        for x in 0..stride {
            let cur = raw[pos + x] as i32;
            let a = if x >= bpp { recon[y * stride + x - bpp] as i32 } else { 0 };
            let b = if y > 0 { recon[(y - 1) * stride + x] as i32 } else { 0 };
            let c = if y > 0 && x >= bpp { recon[(y - 1) * stride + x - bpp] as i32 } else { 0 };
            let pred = match ft { 0 => 0, 1 => a, 2 => b, 3 => (a + b) / 2, 4 => paeth(a, b, c), _ => 0 };
            recon[y * stride + x] = ((cur + pred) & 0xff) as u8;
        }
        pos += stride;
    }

    // Conversion en RGB (composite alpha sur blanc).
    let mut pix = vec![0u32; width * height];
    let sample = |row: &[u8], idx: usize| -> u32 {
        match bd {
            8 => row.get(idx).copied().unwrap_or(0) as u32,
            16 => row.get(idx * 2).copied().unwrap_or(0) as u32, // octet de poids fort
            _ => {
                let bit = idx * bd;
                let byte = row.get(bit / 8).copied().unwrap_or(0) as u32;
                let shift = 8 - bd - (bit % 8);
                let mask = (1u32 << bd) - 1;
                (byte >> shift) & mask
            }
        }
    };
    let maxv = ((1u32 << bd.min(8)) - 1).max(1);
    for y in 0..height {
        let row = &recon[y * stride..(y + 1) * stride];
        for x in 0..width {
            let rgb = match color_type {
                0 => {
                    let g = sample(row, x);
                    let g8 = if bd >= 8 { g } else { g * 255 / maxv };
                    (g8 << 16) | (g8 << 8) | g8
                }
                2 => {
                    let r = sample(row, x * 3);
                    let g = sample(row, x * 3 + 1);
                    let b = sample(row, x * 3 + 2);
                    (r << 16) | (g << 8) | b
                }
                3 => {
                    let idx = sample(row, x) as usize;
                    // tRNS palette : alpha par index si fourni.
                    let base = *palette.get(idx).unwrap_or(&0);
                    if let Some(&a) = trns.get(idx) {
                        let r = (base >> 16) & 0xff;
                        let g = (base >> 8) & 0xff;
                        let b = base & 0xff;
                        super::composite_rgba(r, g, b, a as u32)
                    } else { base }
                }
                4 => {
                    let g = sample(row, x * 2);
                    let a = sample(row, x * 2 + 1);
                    let g8 = composite(g, a, maxv);
                    (g8 << 16) | (g8 << 8) | g8
                }
                6 => {
                    let r = sample(row, x * 4);
                    let g = sample(row, x * 4 + 1);
                    let b = sample(row, x * 4 + 2);
                    let a = sample(row, x * 4 + 3);
                    let r8 = composite(r, a, maxv);
                    let g8 = composite(g, a, maxv);
                    let b8 = composite(b, a, maxv);
                    (r8 << 16) | (g8 << 8) | b8
                }
                _ => 0xffffff,
            };
            pix[y * width + x] = rgb & 0x00ff_ffff;
        }
    }
    Some(Image { w: width, h: height, pix })
}
