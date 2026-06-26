//! Decodage d'images pour le navigateur : PNG (non entrelace) via notre zlib.
//!
//! Sortie : pixels `0x00RRGGBB` (alpha composite sur fond blanc, car le
//! framebuffer est opaque). Le moteur web downscale ensuite a la largeur utile,
//! ce qui donne un rendu volontairement pixelise, dans la meme DA que le texte.

use alloc::vec;
use alloc::vec::Vec;

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub pix: Vec<u32>, // w*h, 0x00RRGGBB
}

fn be32(d: &[u8], i: usize) -> usize {
    ((d[i] as usize) << 24) | ((d[i + 1] as usize) << 16) | ((d[i + 2] as usize) << 8) | d[i + 3] as usize
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// Decode une image. Reconnait PNG et JPEG baseline ; renvoie None sinon.
pub fn decode(data: &[u8]) -> Option<Image> {
    if data.len() > 8 && &data[..8] == &[137, 80, 78, 71, 13, 10, 26, 10] {
        return decode_png(data);
    }
    if data.len() > 3 && data[0] == 0xFF && data[1] == 0xD8 {
        return jpeg::decode_jpeg(data);
    }
    None
}

fn decode_png(data: &[u8]) -> Option<Image> {
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
    // Borne la memoire : on refuse les images > ~1,2 Mpx (le buffer de pixels +
    // les scanlines defiltrees + la sortie zlib resteraient sinon trop gros pour
    // le tas). Elles s'affichent alors en placeholder.
    if width.checked_mul(height)? > 1_200_000 { return None; }

    let channels = match color_type { 0 => 1, 2 => 3, 3 => 1, 4 => 2, 6 => 4, _ => return None };
    let bd = bit_depth as usize;
    if !(bd == 1 || bd == 2 || bd == 4 || bd == 8 || bd == 16) { return None; }
    // Octets par pixel pour le defiltrage (>=1).
    let bpp = ((channels * bd + 7) / 8).max(1);
    let stride = (width * channels * bd + 7) / 8;

    let raw = crate::net::inflate::zlib_decode(&idat).ok()?;
    if raw.len() < (stride + 1) * height { return None; }

    // Defiltrage des scanlines.
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
        // Lit l'echantillon `idx` (en unites de canal) a la profondeur bd.
        match bd {
            8 => row.get(idx).copied().unwrap_or(0) as u32,
            16 => row.get(idx * 2).copied().unwrap_or(0) as u32, // octet de poids fort
            _ => {
                // bd < 8 : echantillons empaquetes MSB-first.
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
                0 => { // grayscale
                    let g = sample(row, x);
                    let g8 = if bd >= 8 { g } else { g * 255 / maxv };
                    (g8 << 16) | (g8 << 8) | g8
                }
                2 => { // RGB
                    let r = sample(row, x * 3);
                    let g = sample(row, x * 3 + 1);
                    let b = sample(row, x * 3 + 2);
                    (r << 16) | (g << 8) | b
                }
                3 => { // palette
                    let idx = sample(row, x) as usize;
                    *palette.get(idx).unwrap_or(&0)
                }
                4 => { // gray + alpha
                    let g = sample(row, x * 2);
                    let a = sample(row, x * 2 + 1);
                    let g8 = composite(g, a, maxv);
                    (g8 << 16) | (g8 << 8) | g8
                }
                6 => { // RGBA
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
            let _ = &trns; // tRNS palette non gere finement (fond blanc par defaut)
            pix[y * width + x] = rgb & 0x00ff_ffff;
        }
    }
    Some(Image { w: width, h: height, pix })
}

// Composite un canal `v` avec alpha `a` sur fond blanc (255).
fn composite(v: u32, a: u32, maxv: u32) -> u32 {
    let v = if maxv == 255 { v } else { v * 255 / maxv };
    let a = if maxv == 255 { a } else { a * 255 / maxv };
    (v * a + 255 * (255 - a)) / 255
}

/// Reduit l'image a au plus `max_w` x `max_h` (plus proche voisin, pixelise).
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

// ============================================================================
// JPEG baseline (SOF0) : Huffman + dequantification + IDCT + YCbCr -> RGB.
//
// Sous-ensemble volontaire : baseline DCT (SOF0), 1 (gris) ou 3 composantes
// (YCbCr), sous-echantillonnage jusqu'a 2x2, restart markers. Tout le reste
// (progressif, arithmetique, CMYK, 16 bits) -> None (repli propre : pas d'image).
// 100% entier sauf l'IDCT (f32, dispo en core ; cos via serie de Taylor maison).
// ============================================================================
mod jpeg {
    use super::Image;
    use alloc::vec;
    use alloc::vec::Vec;

    const ZIGZAG: [usize; 64] = [
        0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34,
        27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44,
        51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
    ];

    // cos sans libm : reduction dans [0, PI/2] (cos est pair et periodique) puis Taylor.
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

    // Table de Huffman canonique (decodage bit a bit via maxcode/mincode/valptr).
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

    // Lecteur de bits MSB-first sur le segment entropique (gere le 0xFF00).
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
                        // marqueur : on recule pour le laisser au resync, on rend des 0.
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

    pub fn decode_jpeg(data: &[u8]) -> Option<Image> {
        let mut qt: [[u16; 64]; 4] = [[0; 64]; 4];
        let mut huff_dc: [Option<Huff>; 4] = [None, None, None, None];
        let mut huff_ac: [Option<Huff>; 4] = [None, None, None, None];
        let mut width = 0usize;
        let mut height = 0usize;
        let mut comps: Vec<Comp> = Vec::new();
        let mut restart = 0usize;

        let mut i = 2usize; // apres SOI (FF D8)
        while i + 1 < data.len() {
            if data[i] != 0xFF { i += 1; continue; }
            let marker = data[i + 1];
            i += 2;
            // Les marqueurs a longueur ont besoin de 2 octets de taille.
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

        // Un plan par composante, a sa resolution (mcux*h*8) x (mcuy*v*8).
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
                    // Resync sur RSTn : aligne octet + saute le marqueur.
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
                            // DC
                            let s = dch.decode(&mut br)?;
                            let diff = extend(br.receive(s as u32), s as u32);
                            preds[ci] += diff;
                            block[0] = preds[ci] * q[0] as i32;
                            // AC
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

        // Assemblage + conversion couleur.
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
}
