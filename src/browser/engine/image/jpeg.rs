//! JPEG baseline (SOF0) : Huffman + déquantification + IDCT + YCbCr → RGB.
//!
//! Sous-ensemble volontaire : baseline DCT (SOF0), 1 (gris) ou 3 composantes
//! (YCbCr), sous-échantillonnage jusqu'à 2x2, restart markers. Tout le reste
//! (progressif, arithmétique, CMYK, 16 bits) → None (repli propre : pas d'image).
//! 100% entier sauf l'IDCT (f32, dispo en core ; cos via série de Taylor maison).

use super::Image;
use alloc::vec;
use alloc::vec::Vec;

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44,
    51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

// cos sans libm : réduction dans [0, PI/2] (cos est pair et périodique) puis Taylor.
fn cos_(x: f32) -> f32 {
    use core::f32::consts::PI;
    let mut a = if x < 0.0 { -x } else { x };
    while a > 2.0 * PI { a -= 2.0 * PI; }
    if a > PI { a = 2.0 * PI - a; }
    let neg = a > PI / 2.0;
    if neg { a = PI - a; }
    let x2 = a * a;
    let c = 1.0 - x2 / 2.0 + x2 * x2 / 24.0 - x2 * x2 * x2 / 720.0 + x2 * x2 * x2 * x2 / 40320.0;
    if neg { -c } else { c }
}

fn idct_basis() -> [[f32; 8]; 8] {
    use core::f32::consts::PI;
    let mut t = [[0.0f32; 8]; 8];
    let inv_sqrt2 = 0.707_106_77f32;
    for x in 0..8 {
        for u in 0..8 {
            let cu = if u == 0 { inv_sqrt2 } else { 1.0 };
            t[x][u] = 0.5 * cu * cos_(((2 * x + 1) as f32) * (u as f32) * PI / 16.0);
        }
    }
    t
}

fn idct(coef: &[i32; 64], basis: &[[f32; 8]; 8], out: &mut [u8; 64]) {
    let mut tmp = [0.0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut s = 0.0f32;
            for u in 0..8 { s += basis[x][u] * coef[y * 8 + u] as f32; }
            tmp[y * 8 + x] = s;
        }
    }
    for x in 0..8 {
        for y in 0..8 {
            let mut s = 0.0f32;
            for v in 0..8 { s += basis[y][v] * tmp[v * 8 + x]; }
            let val = s + 128.0;
            out[y * 8 + x] = if val < 0.0 { 0 } else if val > 255.0 { 255 } else { val as u8 };
        }
    }
}

// Table de Huffman canonique (décodage bit à bit via maxcode/mincode/valptr).
struct Huff {
    mincode: [i32; 17],
    maxcode: [i32; 17],
    valptr: [usize; 17],
    vals: Vec<u8>,
}
impl Huff {
    fn build(counts: &[u8; 16], vals: &[u8]) -> Huff {
        let mut mincode = [0i32; 17];
        let mut maxcode = [-1i32; 17];
        let mut valptr = [0usize; 17];
        let mut code = 0i32;
        let mut k = 0usize;
        for l in 1..=16usize {
            let n = counts[l - 1] as i32;
            if n > 0 {
                valptr[l] = k;
                mincode[l] = code;
                code += n;
                maxcode[l] = code - 1;
                k += n as usize;
            } else {
                maxcode[l] = -1;
            }
            code <<= 1;
        }
        Huff { mincode, maxcode, valptr, vals: vals.to_vec() }
    }
    fn decode(&self, br: &mut BitReader) -> Option<u8> {
        let mut code = 0i32;
        for l in 1..=16usize {
            code = (code << 1) | br.bit() as i32;
            if self.maxcode[l] >= 0 && code <= self.maxcode[l] {
                let idx = self.valptr[l] + (code - self.mincode[l]) as usize;
                return self.vals.get(idx).copied();
            }
        }
        None
    }
}

// Lecteur de bits MSB-first sur le segment entropique (gère le 0xFF00).
struct BitReader<'a> {
    d: &'a [u8],
    pos: usize,
    cur: u32,
    n: u32,
}
impl<'a> BitReader<'a> {
    fn bit(&mut self) -> u32 {
        if self.n == 0 {
            if self.pos >= self.d.len() {
                return 0;
            }
            let mut b = self.d[self.pos];
            self.pos += 1;
            if b == 0xFF {
                let nx = if self.pos < self.d.len() { self.d[self.pos] } else { 0xD9 };
                if nx == 0x00 {
                    self.pos += 1; // octet de bourrage -> vrai 0xFF
                } else {
                    self.pos -= 1;
                    b = 0;
                }
            }
            self.cur = b as u32;
            self.n = 8;
        }
        self.n -= 1;
        (self.cur >> self.n) & 1
    }
    fn receive(&mut self, s: u32) -> i32 {
        let mut v = 0i32;
        for _ in 0..s { v = (v << 1) | self.bit() as i32; }
        v
    }
}
fn extend(v: i32, s: u32) -> i32 {
    if s == 0 { 0 } else if v < (1 << (s - 1)) { v - (1 << s) + 1 } else { v }
}

#[derive(Clone, Copy)]
struct Comp { id: u8, h: usize, v: usize, tq: usize, td: usize, ta: usize }

fn rd16(d: &[u8], i: usize) -> usize { ((d[i] as usize) << 8) | d[i + 1] as usize }

pub fn decode(data: &[u8]) -> Option<Image> {
    let mut qt: [[u16; 64]; 4] = [[0; 64]; 4];
    let mut huff_dc: [Option<Huff>; 4] = [None, None, None, None];
    let mut huff_ac: [Option<Huff>; 4] = [None, None, None, None];
    let mut width = 0usize;
    let mut height = 0usize;
    let mut comps: Vec<Comp> = Vec::new();
    let mut restart = 0usize;

    let mut i = 2usize; // après SOI (FF D8)
    while i + 1 < data.len() {
        if data[i] != 0xFF { i += 1; continue; }
        let marker = data[i + 1];
        i += 2;
        if !matches!(marker, 0xD9 | 0x01 | 0xD0..=0xD7) && i + 1 >= data.len() { return None; }
        match marker {
            0xD9 => return None, // EOI sans SOS
            0xC0 => {
                if i + 8 > data.len() { return None; }
                let len = rd16(data, i);
                if data[i + 2] != 8 { return None; }
                height = rd16(data, i + 3);
                width = rd16(data, i + 5);
                let nc = data[i + 7] as usize;
                if nc == 0 || nc > 4 { return None; }
                if width == 0 || height == 0 || width.checked_mul(height)? > 1_500_000 { return None; }
                let mut p = i + 8;
                for _ in 0..nc {
                    if p + 3 > data.len() { return None; }
                    let hv = data[p + 1];
                    comps.push(Comp { id: data[p], h: (hv >> 4) as usize, v: (hv & 0xf) as usize, tq: (data[p + 2] & 3) as usize, td: 0, ta: 0 });
                    p += 3;
                }
                i += len;
            }
            0xC1..=0xC3 | 0xC5..=0xCF => return None, // SOF non baseline
            0xC4 => {
                let len = rd16(data, i);
                let end = (i + len).min(data.len());
                let mut p = i + 2;
                while p + 17 <= end {
                    let tc_th = data[p];
                    p += 1;
                    let class = (tc_th >> 4) as usize;
                    let id = (tc_th & 0xf) as usize;
                    if id > 3 { return None; }
                    let mut counts = [0u8; 16];
                    let mut total = 0usize;
                    for k in 0..16 { counts[k] = data[p + k]; total += counts[k] as usize; }
                    p += 16;
                    if p + total > end { return None; }
                    let h = Huff::build(&counts, &data[p..p + total]);
                    p += total;
                    if class == 0 { huff_dc[id] = Some(h); } else { huff_ac[id] = Some(h); }
                }
                i += len;
            }
            0xDB => {
                let len = rd16(data, i);
                let end = (i + len).min(data.len());
                let mut p = i + 2;
                while p < end {
                    let pq_tq = data[p];
                    p += 1;
                    let pq = (pq_tq >> 4) as usize;
                    let tq = (pq_tq & 3) as usize;
                    for k in 0..64 {
                        if pq == 0 { qt[tq][k] = data[p] as u16; p += 1; }
                        else { qt[tq][k] = rd16(data, p) as u16; p += 2; }
                    }
                }
                i += len;
            }
            0xDD => {
                restart = rd16(data, i + 2);
                i += rd16(data, i);
            }
            0xDA => {
                let len = rd16(data, i);
                let ns = data[i + 2] as usize;
                let mut p = i + 3;
                for _ in 0..ns {
                    let cs = data[p];
                    let tdta = data[p + 1];
                    p += 2;
                    for c in comps.iter_mut() {
                        if c.id == cs { c.td = (tdta >> 4) as usize; c.ta = (tdta & 0xf) as usize; }
                    }
                }
                let scan_start = i + len;
                return decode_scan(data, scan_start, width, height, &comps, &qt, &huff_dc, &huff_ac, restart);
            }
            0x01 | 0xD0..=0xD7 => {} // marqueurs sans longueur
            _ => { i += rd16(data, i); } // APPn, COM... -> saute par longueur
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn decode_scan(
    data: &[u8], scan_start: usize, width: usize, height: usize, comps: &[Comp],
    qt: &[[u16; 64]; 4], huff_dc: &[Option<Huff>; 4], huff_ac: &[Option<Huff>; 4], restart: usize,
) -> Option<Image> {
    if comps.is_empty() { return None; }
    let basis = idct_basis();
    let hmax = comps.iter().map(|c| c.h).max().unwrap_or(1).max(1);
    let vmax = comps.iter().map(|c| c.v).max().unwrap_or(1).max(1);
    let mcux = (width + 8 * hmax - 1) / (8 * hmax);
    let mcuy = (height + 8 * vmax - 1) / (8 * vmax);

    let mut planes: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    for c in comps {
        let cw = mcux * c.h * 8;
        let ch = mcuy * c.v * 8;
        if cw.checked_mul(ch)? > 8_000_000 { return None; }
        planes.push((cw, ch, vec![0u8; cw * ch]));
    }

    let mut br = BitReader { d: data, pos: scan_start, cur: 0, n: 0 };
    let mut preds = vec![0i32; comps.len()];
    let mut block = [0i32; 64];
    let mut out = [0u8; 64];
    let mut mcu = 0usize;

    for my in 0..mcuy {
        for mx in 0..mcux {
            if restart > 0 && mcu > 0 && mcu % restart == 0 {
                br.n = 0;
                while br.pos + 1 < data.len() && !(data[br.pos] == 0xFF && (0xD0..=0xD7).contains(&data[br.pos + 1])) { br.pos += 1; }
                if br.pos + 1 < data.len() { br.pos += 2; }
                for p in preds.iter_mut() { *p = 0; }
            }
            for (ci, c) in comps.iter().enumerate() {
                let dch = huff_dc.get(c.td)?.as_ref()?;
                let ach = huff_ac.get(c.ta)?.as_ref()?;
                let q = qt.get(c.tq)?;
                for by in 0..c.v {
                    for bx in 0..c.h {
                        for x in block.iter_mut() { *x = 0; }
                        let s = dch.decode(&mut br)?;
                        let diff = extend(br.receive(s as u32), s as u32);
                        preds[ci] += diff;
                        block[0] = preds[ci] * q[0] as i32;
                        let mut k = 1usize;
                        while k < 64 {
                            let rs = ach.decode(&mut br)?;
                            let r = (rs >> 4) as usize;
                            let sb = (rs & 0xf) as u32;
                            if sb == 0 {
                                if r == 15 { k += 16; continue; } else { break; }
                            }
                            k += r;
                            if k >= 64 { break; }
                            let val = extend(br.receive(sb), sb);
                            block[ZIGZAG[k]] = val * q[k] as i32;
                            k += 1;
                        }
                        idct(&block, &basis, &mut out);
                        let (cw, ch, plane) = &mut planes[ci];
                        let ox = (mx * c.h + bx) * 8;
                        let oy = (my * c.v + by) * 8;
                        for yy in 0..8 {
                            let py = oy + yy;
                            if py >= *ch { break; }
                            for xx in 0..8 {
                                let px = ox + xx;
                                if px < *cw { plane[py * *cw + px] = out[yy * 8 + xx]; }
                            }
                        }
                    }
                }
            }
            mcu += 1;
        }
    }

    let mut pix = vec![0u32; width * height];
    let clamp = |v: f32| -> u32 { if v < 0.0 { 0 } else if v > 255.0 { 255 } else { v as u32 } };
    let sample = |idx: usize, x: usize, y: usize| -> i32 {
        let (pw, ph, p) = &planes[idx];
        let cx = (x * comps[idx].h / hmax).min(pw - 1);
        let cy = (y * comps[idx].v / vmax).min(ph - 1);
        p[cy * *pw + cx] as i32
    };
    for y in 0..height {
        for x in 0..width {
            if comps.len() == 1 {
                let g = sample(0, x, y) as u32 & 0xff;
                pix[y * width + x] = (g << 16) | (g << 8) | g;
            } else {
                let yy = sample(0, x, y) as f32;
                let cb = (sample(1, x, y) - 128) as f32;
                let cr = (sample(2, x, y) - 128) as f32;
                let r = yy + 1.402 * cr;
                let g = yy - 0.344_136 * cb - 0.714_136 * cr;
                let b = yy + 1.772 * cb;
                pix[y * width + x] = (clamp(r) << 16) | (clamp(g) << 8) | clamp(b);
            }
        }
    }
    Some(Image { w: width, h: height, pix })
}
