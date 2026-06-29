//! GIF87a/89a : décodage de la **première image** (les animations rendent juste
//! la frame initiale). Palette globale/locale, LZW variable-width, transparence
//! via l'extension de contrôle graphique. Entrelacement géré.

use super::{composite_rgba, Image};
use alloc::vec;
use alloc::vec::Vec;

pub fn decode(data: &[u8]) -> Option<Image> {
    if data.len() < 13 { return None; }
    let sw = (data[6] as usize) | ((data[7] as usize) << 8);
    let sh = (data[8] as usize) | ((data[9] as usize) << 8);
    let packed = data[10];
    let gct_flag = packed & 0x80 != 0;
    let gct_size = 2usize << (packed & 0x07);
    if sw == 0 || sh == 0 || sw > 4096 || sh > 4096 { return None; }
    if sw.checked_mul(sh)? > 1_200_000 { return None; }

    let mut p = 13usize;
    let global_palette = if gct_flag {
        let pal = read_palette(data, p, gct_size)?;
        p += gct_size * 3;
        pal
    } else { Vec::new() };

    let mut transparent: Option<usize> = None;

    // Parcourt les blocs jusqu'au premier descripteur d'image (0x2C).
    while p < data.len() {
        match data[p] {
            0x21 => {
                // Extension. 0xF9 = contrôle graphique (transparence).
                let label = *data.get(p + 1)?;
                p += 2;
                if label == 0xF9 {
                    let bsize = *data.get(p)? as usize;
                    if bsize >= 4 {
                        let flags = data[p + 1];
                        if flags & 0x01 != 0 { transparent = Some(data[p + 4] as usize); }
                    }
                }
                p = skip_sub_blocks(data, p)?;
            }
            0x2C => {
                // Descripteur d'image.
                let ix = (data[p + 1] as usize) | ((data[p + 2] as usize) << 8);
                let iy = (data[p + 3] as usize) | ((data[p + 4] as usize) << 8);
                let iw = (data[p + 5] as usize) | ((data[p + 6] as usize) << 8);
                let ih = (data[p + 7] as usize) | ((data[p + 8] as usize) << 8);
                let lpacked = data[p + 9];
                p += 10;
                let lct_flag = lpacked & 0x80 != 0;
                let interlaced = lpacked & 0x40 != 0;
                let palette = if lct_flag {
                    let sz = 2usize << (lpacked & 0x07);
                    let pal = read_palette(data, p, sz)?;
                    p += sz * 3;
                    pal
                } else { global_palette.clone() };
                if iw == 0 || ih == 0 || ix + iw > sw || iy + ih > sh { return None; }

                let min_code = *data.get(p)? as usize;
                p += 1;
                let lzw = gather_sub_blocks(data, p)?;
                let indices = lzw_decode(&lzw, min_code, iw * ih)?;

                let mut pix = vec![0xffffffu32; sw * sh];
                let order = deinterlace_rows(ih, interlaced);
                let mut k = 0usize;
                for &row in &order {
                    for col in 0..iw {
                        if k >= indices.len() { break; }
                        let ci = indices[k] as usize;
                        k += 1;
                        if Some(ci) == transparent { continue; } // garde le fond blanc
                        let rgb = *palette.get(ci).unwrap_or(&0);
                        let dst = (iy + row) * sw + (ix + col);
                        if dst < pix.len() { pix[dst] = rgb & 0x00ff_ffff; }
                    }
                }
                return Some(Image { w: sw, h: sh, pix });
            }
            0x3B => break, // trailer
            _ => { p += 1; }
        }
    }
    None
}

fn read_palette(d: &[u8], at: usize, n: usize) -> Option<Vec<u32>> {
    if at + n * 3 > d.len() { return None; }
    let mut pal = Vec::with_capacity(n);
    for i in 0..n {
        let o = at + i * 3;
        pal.push(((d[o] as u32) << 16) | ((d[o + 1] as u32) << 8) | d[o + 2] as u32);
    }
    let _ = composite_rgba; // partagé, gardé pour cohérence d'API
    Some(pal)
}

fn skip_sub_blocks(d: &[u8], mut p: usize) -> Option<usize> {
    loop {
        let n = *d.get(p)? as usize;
        p += 1;
        if n == 0 { return Some(p); }
        p += n;
        if p > d.len() { return None; }
    }
}

fn gather_sub_blocks(d: &[u8], mut p: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let n = *d.get(p)? as usize;
        p += 1;
        if n == 0 { return Some(out); }
        if p + n > d.len() { return None; }
        out.extend_from_slice(&d[p..p + n]);
        p += n;
    }
}

fn deinterlace_rows(h: usize, interlaced: bool) -> Vec<usize> {
    if !interlaced { return (0..h).collect(); }
    let mut rows = Vec::with_capacity(h);
    for (start, step) in [(0usize, 8usize), (4, 8), (2, 4), (1, 2)] {
        let mut y = start;
        while y < h { rows.push(y); y += step; }
    }
    rows
}

// Décodeur LZW GIF : codes de largeur variable (min_code+1 .. 12 bits).
fn lzw_decode(data: &[u8], min_code: usize, max_out: usize) -> Option<Vec<u8>> {
    if min_code < 2 || min_code > 8 { return None; }
    let clear = 1usize << min_code;
    let end = clear + 1;
    let mut dict: Vec<Vec<u8>> = Vec::with_capacity(4096);
    let reset = |dict: &mut Vec<Vec<u8>>| {
        dict.clear();
        for i in 0..clear { dict.push(vec![i as u8]); }
        dict.push(Vec::new()); // clear
        dict.push(Vec::new()); // end
    };
    reset(&mut dict);

    let mut out: Vec<u8> = Vec::with_capacity(max_out.min(1_200_000));
    let mut code_size = min_code + 1;
    let mut bitpos = 0usize;
    let total_bits = data.len() * 8;
    let mut prev: Option<usize> = None;

    let read_code = |bitpos: &mut usize, code_size: usize| -> Option<usize> {
        if *bitpos + code_size > total_bits { return None; }
        let mut val = 0usize;
        for i in 0..code_size {
            let bp = *bitpos + i;
            let bit = (data[bp / 8] >> (bp % 8)) & 1;
            val |= (bit as usize) << i;
        }
        *bitpos += code_size;
        Some(val)
    };

    while out.len() < max_out {
        let code = match read_code(&mut bitpos, code_size) { Some(c) => c, None => break };
        if code == clear {
            reset(&mut dict);
            code_size = min_code + 1;
            prev = None;
            continue;
        }
        if code == end { break; }

        let entry: Vec<u8> = if code < dict.len() {
            dict[code].clone()
        } else if code == dict.len() {
            // cas KwKwK
            let mut e = dict.get(prev?)?.clone();
            let first = *e.first()?;
            e.push(first);
            e
        } else {
            break; // code invalide -> on s'arrête proprement
        };
        out.extend_from_slice(&entry);

        if let Some(pv) = prev {
            let mut ne = dict[pv].clone();
            ne.push(*entry.first()?);
            dict.push(ne);
            if dict.len() == (1 << code_size) && code_size < 12 { code_size += 1; }
        }
        prev = Some(code);
    }
    Some(out)
}
