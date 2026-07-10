//! Décodeur VP8 lossy (keyframe intra) écrit à la main — RFC 6386.
//!
//! C'est le format de la MAJORITÉ des WebP servis par les CDN (le VP8L
//! lossless de webp.rs couvre l'autre partie). Aucune crate no_std n'existe
//! (image-webp exige std, voir docs/AUDIT.md) : transcription fidèle du
//! décodeur de référence libwebp (dec/vp8_dec.c, tree_dec.c, frame_dec.c,
//! dsp/dec.c, dsp/upsampling.c, dsp/yuv.h), tables extraites mécaniquement
//! (vp8_tables.rs), et lecteur booléen normatif de la RFC 6386 §7.3.
//!
//! Pipeline : en-têtes (segments/filtre/partitions/quantif/probas) → par
//! rangée de macroblocs : modes intra (partition 0) puis résidus (partitions
//! de tokens) → reconstruction (prédiction intra 16x16/4x4/chroma + IDCT/IWHT
//! bit-exacts) → filtre de boucle (simple ou normal) → suréchantillonnage
//! chroma « fancy » + YUV→RGB en virgule fixe (constantes libwebp exactes).
//!
//! Vérifié pixel par pixel contre libwebp (Pillow) via harnais — voir commit.
//! Limitation assumée : la transparence ALPH (VP8X) n'est pas décodée, l'image
//! est rendue opaque (le framebuffer est de toute façon opaque).

use alloc::vec;
use alloc::vec::Vec;
use super::vp8_tables as t;
use super::Image;

const BPS: usize = 32; // stride du tampon de travail (comme libwebp)
const Y_OFF: usize = BPS + 8;
const U_OFF: usize = Y_OFF + BPS * 16 + BPS;
const V_OFF: usize = U_OFF + 16;
const YUV_SIZE: usize = BPS * 17 + BPS * 9;

// ── Lecteur booléen (RFC 6386 §7.3, normatif) ───────────────────────────────

struct BoolDec<'a> {
    data: &'a [u8],
    pos: usize,
    value: u32,
    range: u32,
    bit_count: i32,
    eof: bool,
}

impl<'a> BoolDec<'a> {
    fn new(data: &'a [u8]) -> BoolDec<'a> {
        let mut d = BoolDec { data, pos: 0, value: 0, range: 255, bit_count: 0, eof: false };
        for _ in 0..2 {
            d.value = (d.value << 8) | d.next_byte() as u32;
        }
        d
    }
    fn next_byte(&mut self) -> u8 {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            b
        } else {
            self.eof = true;
            0
        }
    }
    fn bit(&mut self, prob: u8) -> u32 {
        let split = 1 + (((self.range - 1) * prob as u32) >> 8);
        let bigsplit = split << 8;
        let ret;
        if self.value >= bigsplit {
            ret = 1;
            self.range -= split;
            self.value -= bigsplit;
        } else {
            ret = 0;
            self.range = split;
        }
        while self.range < 128 {
            self.value <<= 1;
            self.range <<= 1;
            self.bit_count += 1;
            if self.bit_count == 8 {
                self.bit_count = 0;
                self.value |= self.next_byte() as u32;
            }
        }
        ret
    }
    fn flag(&mut self) -> bool { self.bit(128) != 0 }
    // n bits uniformes, MSB en premier.
    fn value_bits(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for _ in 0..n { v = (v << 1) | self.bit(128); }
        v
    }
    fn signed_value(&mut self, n: u32) -> i32 {
        let v = self.value_bits(n) as i32;
        if self.flag() { -v } else { v }
    }
    // Signe d'un coefficient déjà lu.
    fn signed(&mut self, v: i32) -> i32 { if self.flag() { -v } else { v } }
}

// ── Structures d'état ────────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
struct QuantMat {
    y1: [i32; 2], // [dc, ac]
    y2: [i32; 2],
    uv: [i32; 2],
}

struct Header {
    mb_w: usize,
    mb_h: usize,
    use_segment: bool,
    update_map: bool,
    segment_probs: [u8; 3],
    filter_type: u8, // 0 aucun, 1 simple, 2 normal
    fstrengths: [[(u8, u8, u8, bool); 2]; 4], // [seg][i4x4] -> (limit, ilevel, hev, inner)
    dqm: [QuantMat; 4],
    // probas de coefficients : [type][bande][ctx][11]
    coeffs: [[[[u8; 11]; 3]; 8]; 4],
    use_skip_proba: bool,
    skip_p: u8,
}

// Données d'un macrobloc de la rangée courante.
#[derive(Clone)]
struct MbData {
    coeffs: [i16; 384],
    is_i4x4: bool,
    imodes: [u8; 16],
    uvmode: u8,
    segment: usize,
    skip: bool,
    non_zero_y: u32,
    non_zero_uv: u32,
}

impl Default for MbData {
    fn default() -> MbData {
        MbData { coeffs: [0; 384], is_i4x4: false, imodes: [0; 16], uvmode: 0, segment: 0, skip: false, non_zero_y: 0, non_zero_uv: 0 }
    }
}

fn clip127(q: i32, m: i32) -> usize { q.clamp(0, m) as usize }

// ── Décodage des coefficients (§13, transcription de GetCoeffsFast) ─────────

fn get_large_value(br: &mut BoolDec, p: &[u8; 11]) -> i32 {
    let v;
    if br.bit(p[3]) == 0 {
        if br.bit(p[4]) == 0 {
            v = 2;
        } else {
            v = 3 + br.bit(p[5]) as i32;
        }
    } else if br.bit(p[6]) == 0 {
        if br.bit(p[7]) == 0 {
            v = 5 + br.bit(159) as i32;
        } else {
            v = 7 + 2 * br.bit(165) as i32 + br.bit(145) as i32;
        }
    } else {
        let bit1 = br.bit(p[8]) as usize;
        let bit0 = br.bit(p[9 + bit1]) as usize;
        let cat = 2 * bit1 + bit0;
        let tab: &[u8] = match cat {
            0 => &t::CAT3, 1 => &t::CAT4, 2 => &t::CAT5, _ => &t::CAT6,
        };
        let mut w = 0i32;
        for &pr in tab { w += w + br.bit(pr) as i32; }
        return w + 3 + (8 << cat);
    }
    v
}

// Renvoie la position du dernier coefficient non nul + 1.
fn get_coeffs(br: &mut BoolDec, probs: &[[[u8; 11]; 3]; 8], ctx: usize, dq: &[i32; 2], first: usize, out: &mut [i16]) -> usize {
    let mut n = first;
    let mut p: &[u8; 11] = &probs[t::BANDS[n]][ctx];
    while n < 16 {
        if br.bit(p[0]) == 0 { return n; }
        while br.bit(p[1]) == 0 {
            n += 1;
            if n == 16 { return 16; }
            p = &probs[t::BANDS[n]][0];
        }
        let band_next = t::BANDS[n + 1];
        let v;
        if br.bit(p[2]) == 0 {
            v = 1;
            p = &probs[band_next][1];
        } else {
            v = get_large_value(br, p);
            p = &probs[band_next][2];
        }
        out[t::ZIGZAG[n]] = (br.signed(v) * dq[(n > 0) as usize]) as i16;
        n += 1;
    }
    16
}

// ── Transformées (§14.4, bit-exactes) ────────────────────────────────────────

fn clip8(v: i32) -> u8 { if v & !0xff == 0 { v as u8 } else if v < 0 { 0 } else { 255 } }
fn mul1(a: i32) -> i32 { ((a * 20091) >> 16) + a }
fn mul2(a: i32) -> i32 { (a * 35468) >> 16 }

fn store(buf: &mut [u8], off: usize, x: usize, y: usize, v: i32) {
    let i = off + x + y * BPS;
    buf[i] = clip8(buf[i] as i32 + (v >> 3));
}

fn transform_one(cf: &[i16], buf: &mut [u8], off: usize) {
    let mut c = [0i32; 16];
    for i in 0..4 {
        let a = cf[i] as i32 + cf[8 + i] as i32;
        let b = cf[i] as i32 - cf[8 + i] as i32;
        let c1 = mul2(cf[4 + i] as i32) - mul1(cf[12 + i] as i32);
        let d = mul1(cf[4 + i] as i32) + mul2(cf[12 + i] as i32);
        c[4 * i] = a + d;
        c[4 * i + 1] = b + c1;
        c[4 * i + 2] = b - c1;
        c[4 * i + 3] = a - d;
    }
    for i in 0..4 {
        let dc = c[i] + 4;
        let a = dc + c[8 + i];
        let b = dc - c[8 + i];
        let c1 = mul2(c[4 + i]) - mul1(c[12 + i]);
        let d = mul1(c[4 + i]) + mul2(c[12 + i]);
        store(buf, off, 0, i, a + d);
        store(buf, off, 1, i, b + c1);
        store(buf, off, 2, i, b - c1);
        store(buf, off, 3, i, a - d);
    }
}

fn transform_ac3(cf: &[i16], buf: &mut [u8], off: usize) {
    let a = cf[0] as i32 + 4;
    let c4 = mul2(cf[4] as i32);
    let d4 = mul1(cf[4] as i32);
    let c1 = mul2(cf[1] as i32);
    let d1 = mul1(cf[1] as i32);
    let rows = [a + d4, a + c4, a - c4, a - d4];
    for (y, &dc) in rows.iter().enumerate() {
        store(buf, off, 0, y, dc + d1);
        store(buf, off, 1, y, dc + c1);
        store(buf, off, 2, y, dc - c1);
        store(buf, off, 3, y, dc - d1);
    }
}

fn transform_dc(cf: &[i16], buf: &mut [u8], off: usize) {
    let dc = cf[0] as i32 + 4;
    for y in 0..4 { for x in 0..4 { store(buf, off, x, y, dc); } }
}

// IWHT (Y2) : répartit les DC dans les 16 blocs (out[j*16]).
fn transform_wht(cf: &[i16; 16], out: &mut [i16; 384]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a0 = cf[i] as i32 + cf[12 + i] as i32;
        let a1 = cf[4 + i] as i32 + cf[8 + i] as i32;
        let a2 = cf[4 + i] as i32 - cf[8 + i] as i32;
        let a3 = cf[i] as i32 - cf[12 + i] as i32;
        tmp[i] = a0 + a1;
        tmp[8 + i] = a0 - a1;
        tmp[4 + i] = a3 + a2;
        tmp[12 + i] = a3 - a2;
    }
    for i in 0..4 {
        let dc = tmp[i * 4] + 3;
        let a0 = dc + tmp[3 + i * 4];
        let a1 = tmp[1 + i * 4] + tmp[2 + i * 4];
        let a2 = tmp[1 + i * 4] - tmp[2 + i * 4];
        let a3 = dc - tmp[3 + i * 4];
        out[i * 64] = ((a0 + a1) >> 3) as i16;
        out[i * 64 + 16] = ((a3 + a2) >> 3) as i16;
        out[i * 64 + 32] = ((a0 - a1) >> 3) as i16;
        out[i * 64 + 48] = ((a3 - a2) >> 3) as i16;
    }
}

// ── Prédicteurs intra (§12, transcription dsp/dec.c) ────────────────────────

fn avg3(a: i32, b: i32, c: i32) -> u8 { ((a + 2 * b + c + 2) >> 2) as u8 }
fn avg2(a: i32, b: i32) -> u8 { ((a + b + 1) >> 1) as u8 }

// b[o] = coin haut-gauche du bloc ; taille n ; remplit avec `v`.
fn fill(b: &mut [u8], o: usize, n: usize, v: u8) {
    for j in 0..n { for i in 0..n { b[o + j * BPS + i] = v; } }
}

fn true_motion(b: &mut [u8], o: usize, n: usize) {
    let tl = b[o - BPS - 1] as i32;
    for y in 0..n {
        let l = b[o + y * BPS - 1] as i32;
        for x in 0..n {
            let v = l + b[o - BPS + x] as i32 - tl;
            b[o + y * BPS + x] = v.clamp(0, 255) as u8;
        }
    }
}

// mode : 0 DC, 1 TM, 2 VE, 3 HE, 4 DC sans haut, 5 DC sans gauche, 6 DC 128.
fn pred_nxn(b: &mut [u8], o: usize, n: usize, mode: u8) {
    match mode {
        0 => {
            let mut dc = n as i32;
            for i in 0..n { dc += b[o + i - BPS] as i32 + b[o + i * BPS - 1] as i32; }
            let shift = if n == 16 { 5 } else { 4 };
            fill(b, o, n, (dc >> shift) as u8);
        }
        1 => true_motion(b, o, n),
        2 => { for j in 0..n { for i in 0..n { b[o + j * BPS + i] = b[o + i - BPS]; } } }
        3 => { for j in 0..n { let v = b[o + j * BPS - 1]; for i in 0..n { b[o + j * BPS + i] = v; } } }
        4 => {
            let mut dc = (n / 2) as i32;
            for i in 0..n { dc += b[o + i * BPS - 1] as i32; }
            let shift = if n == 16 { 4 } else { 3 };
            fill(b, o, n, (dc >> shift) as u8);
        }
        5 => {
            let mut dc = (n / 2) as i32;
            for i in 0..n { dc += b[o + i - BPS] as i32; }
            let shift = if n == 16 { 4 } else { 3 };
            fill(b, o, n, (dc >> shift) as u8);
        }
        _ => fill(b, o, n, 0x80),
    }
}

// Prédicteurs 4x4 (modes B_*), transcription littérale.
fn gp(b: &[u8], o: usize, dx: i32, dy: i32) -> i32 {
    b[(o as i32 + dx + dy * BPS as i32) as usize] as i32
}
fn sp(b: &mut [u8], o: usize, x: usize, y: usize, v: u8) { b[o + x + y * BPS] = v; }

fn pred4(b: &mut [u8], o: usize, mode: u8) {
    macro_rules! g { ($dx:expr, $dy:expr) => { gp(b, o, $dx, $dy) } }
    macro_rules! s { ($x:expr, $y:expr, $v:expr) => { sp(b, o, $x, $y, $v) } }
    match mode {
        0 => { // DC4
            let mut dc = 4;
            for i in 0..4 { dc += g!(i as i32, -1) + g!(-1, i as i32); }
            let v = (dc >> 3) as u8;
            for j in 0..4 { for i in 0..4 { b[o + j * BPS + i] = v; } }
        }
        1 => true_motion(b, o, 4), // TM4
        2 => { // VE4 : moyenne du haut
            let vals = [
                avg3(g!(-1, -1), g!(0, -1), g!(1, -1)),
                avg3(g!(0, -1), g!(1, -1), g!(2, -1)),
                avg3(g!(1, -1), g!(2, -1), g!(3, -1)),
                avg3(g!(2, -1), g!(3, -1), g!(4, -1)),
            ];
            for j in 0..4 { for i in 0..4 { b[o + j * BPS + i] = vals[i]; } }
        }
        3 => { // HE4
            let (a, bb, c, d, e) = (g!(-1, -1), g!(-1, 0), g!(-1, 1), g!(-1, 2), g!(-1, 3));
            let rows = [avg3(a, bb, c), avg3(bb, c, d), avg3(c, d, e), avg3(d, e, e)];
            for j in 0..4 { for i in 0..4 { b[o + j * BPS + i] = rows[j]; } }
        }
        4 => { // RD4
            let (i_, j_, k, l) = (g!(-1, 0), g!(-1, 1), g!(-1, 2), g!(-1, 3));
            let (x, a, bb, c, d) = (g!(-1, -1), g!(0, -1), g!(1, -1), g!(2, -1), g!(3, -1));
            s!(0, 3, avg3(j_, k, l));
            let v = avg3(i_, j_, k); s!(1, 3, v); s!(0, 2, v);
            let v = avg3(x, i_, j_); s!(2, 3, v); s!(1, 2, v); s!(0, 1, v);
            let v = avg3(a, x, i_); s!(3, 3, v); s!(2, 2, v); s!(1, 1, v); s!(0, 0, v);
            let v = avg3(bb, a, x); s!(3, 2, v); s!(2, 1, v); s!(1, 0, v);
            let v = avg3(c, bb, a); s!(3, 1, v); s!(2, 0, v);
            s!(3, 0, avg3(d, c, bb));
        }
        6 => { // LD4
            let (a, bb, c, d) = (g!(0, -1), g!(1, -1), g!(2, -1), g!(3, -1));
            let (e, f, gg, h) = (g!(4, -1), g!(5, -1), g!(6, -1), g!(7, -1));
            s!(0, 0, avg3(a, bb, c));
            let v = avg3(bb, c, d); s!(1, 0, v); s!(0, 1, v);
            let v = avg3(c, d, e); s!(2, 0, v); s!(1, 1, v); s!(0, 2, v);
            let v = avg3(d, e, f); s!(3, 0, v); s!(2, 1, v); s!(1, 2, v); s!(0, 3, v);
            let v = avg3(e, f, gg); s!(3, 1, v); s!(2, 2, v); s!(1, 3, v);
            let v = avg3(f, gg, h); s!(3, 2, v); s!(2, 3, v);
            s!(3, 3, avg3(gg, h, h));
        }
        5 => { // VR4
            let (i_, j_, k) = (g!(-1, 0), g!(-1, 1), g!(-1, 2));
            let (x, a, bb, c, d) = (g!(-1, -1), g!(0, -1), g!(1, -1), g!(2, -1), g!(3, -1));
            let v = avg2(x, a); s!(0, 0, v); s!(1, 2, v);
            let v = avg2(a, bb); s!(1, 0, v); s!(2, 2, v);
            let v = avg2(bb, c); s!(2, 0, v); s!(3, 2, v);
            s!(3, 0, avg2(c, d));
            s!(0, 3, avg3(k, j_, i_));
            s!(0, 2, avg3(j_, i_, x));
            let v = avg3(i_, x, a); s!(0, 1, v); s!(1, 3, v);
            let v = avg3(x, a, bb); s!(1, 1, v); s!(2, 3, v);
            let v = avg3(a, bb, c); s!(2, 1, v); s!(3, 3, v);
            s!(3, 1, avg3(bb, c, d));
        }
        7 => { // VL4
            let (a, bb, c, d) = (g!(0, -1), g!(1, -1), g!(2, -1), g!(3, -1));
            let (e, f, gg, h) = (g!(4, -1), g!(5, -1), g!(6, -1), g!(7, -1));
            let v = avg2(a, bb); s!(0, 0, v);
            let v = avg2(bb, c); s!(1, 0, v); s!(0, 2, v);
            let v = avg2(c, d); s!(2, 0, v); s!(1, 2, v);
            let v = avg2(d, e); s!(3, 0, v); s!(2, 2, v);
            s!(0, 1, avg3(a, bb, c));
            let v = avg3(bb, c, d); s!(1, 1, v); s!(0, 3, v);
            let v = avg3(c, d, e); s!(2, 1, v); s!(1, 3, v);
            let v = avg3(d, e, f); s!(3, 1, v); s!(2, 3, v);
            s!(3, 2, avg3(e, f, gg));
            s!(3, 3, avg3(f, gg, h));
        }
        9 => { // HU4
            let (i_, j_, k, l) = (g!(-1, 0), g!(-1, 1), g!(-1, 2), g!(-1, 3));
            s!(0, 0, avg2(i_, j_));
            let v = avg2(j_, k); s!(2, 0, v); s!(0, 1, v);
            let v = avg2(k, l); s!(2, 1, v); s!(0, 2, v);
            s!(1, 0, avg3(i_, j_, k));
            let v = avg3(j_, k, l); s!(3, 0, v); s!(1, 1, v);
            let v = avg3(k, l, l); s!(3, 1, v); s!(1, 2, v);
            let lv = l as u8;
            s!(3, 2, lv); s!(2, 2, lv); s!(0, 3, lv); s!(1, 3, lv); s!(2, 3, lv); s!(3, 3, lv);
        }
        _ => { // 8 : HD4
            let (i_, j_, k, l) = (g!(-1, 0), g!(-1, 1), g!(-1, 2), g!(-1, 3));
            let (x, a, bb, c) = (g!(-1, -1), g!(0, -1), g!(1, -1), g!(2, -1));
            let v = avg2(i_, x); s!(0, 0, v); s!(2, 1, v);
            let v = avg2(j_, i_); s!(0, 1, v); s!(2, 2, v);
            let v = avg2(k, j_); s!(0, 2, v); s!(2, 3, v);
            s!(0, 3, avg2(l, k));
            s!(3, 0, avg3(a, bb, c));
            s!(2, 0, avg3(x, a, bb));
            let v = avg3(i_, x, a); s!(1, 0, v); s!(3, 1, v);
            let v = avg3(j_, i_, x); s!(1, 1, v); s!(3, 2, v);
            let v = avg3(k, j_, i_); s!(1, 2, v); s!(3, 3, v);
            s!(1, 3, avg3(l, k, j_));
        }
    }
}

// Ordre de balayage des 16 sous-blocs Y (kScan).
const SCAN: [usize; 16] = [
    0, 4, 8, 12,
    4 * BPS, 4 + 4 * BPS, 8 + 4 * BPS, 12 + 4 * BPS,
    8 * BPS, 4 + 8 * BPS, 8 + 8 * BPS, 12 + 8 * BPS,
    12 * BPS, 4 + 12 * BPS, 8 + 12 * BPS, 12 + 12 * BPS,
];

// Mode DC ajusté aux bords (CheckMode) : 4 = NoTop (somme la colonne gauche),
// 5 = NoLeft (somme la rangée du haut), 6 = ni l'un ni l'autre (128).
// BUG CORRIGÉ par le harnais : 4 et 5 étaient inversés (mb_x==0 = pas de
// gauche -> NoLeft=5), ce qui cassait tout MB 16x16/chroma en mode DC sur les
// bords puis se propageait à toute l'image par la prédiction intra.
fn check_mode(mb_x: usize, mb_y: usize, mode: u8) -> u8 {
    if mode == 0 {
        if mb_x == 0 {
            if mb_y == 0 { 6 } else { 5 }
        } else if mb_y == 0 { 4 } else { 0 }
    } else {
        mode
    }
}

// ── Filtre de boucle (§15, transcription dsp/dec.c) ─────────────────────────

fn sclip1(v: i32) -> i32 { v.clamp(-128, 127) }
fn sclip2(v: i32) -> i32 { v.clamp(-16, 15) }
fn uclip(v: i32) -> u8 { v.clamp(0, 255) as u8 }

fn do_filter2(p: &mut [u8], o: usize, step: isize) {
    let g = |p: &[u8], d: isize| p[(o as isize + d * step) as usize] as i32;
    let (p1, p0, q0, q1) = (g(p, -2), g(p, -1), g(p, 0), g(p, 1));
    let a = 3 * (q0 - p0) + sclip1(p1 - q1);
    let a1 = sclip2((a + 4) >> 3);
    let a2 = sclip2((a + 3) >> 3);
    p[(o as isize - step) as usize] = uclip(p0 + a2);
    p[o] = uclip(q0 - a1);
}

fn do_filter4(p: &mut [u8], o: usize, step: isize) {
    let g = |p: &[u8], d: isize| p[(o as isize + d * step) as usize] as i32;
    let (p1, p0, q0, q1) = (g(p, -2), g(p, -1), g(p, 0), g(p, 1));
    let a = 3 * (q0 - p0);
    let a1 = sclip2((a + 4) >> 3);
    let a2 = sclip2((a + 3) >> 3);
    let a3 = (a1 + 1) >> 1;
    p[(o as isize - 2 * step) as usize] = uclip(p1 + a3);
    p[(o as isize - step) as usize] = uclip(p0 + a2);
    p[o] = uclip(q0 - a1);
    p[(o as isize + step) as usize] = uclip(q1 - a3);
}

fn do_filter6(p: &mut [u8], o: usize, step: isize) {
    let g = |p: &[u8], d: isize| p[(o as isize + d * step) as usize] as i32;
    let (p2, p1, p0, q0, q1, q2) = (g(p, -3), g(p, -2), g(p, -1), g(p, 0), g(p, 1), g(p, 2));
    let a = sclip1(3 * (q0 - p0) + sclip1(p1 - q1));
    let a1 = (27 * a + 63) >> 7;
    let a2 = (18 * a + 63) >> 7;
    let a3 = (9 * a + 63) >> 7;
    p[(o as isize - 3 * step) as usize] = uclip(p2 + a3);
    p[(o as isize - 2 * step) as usize] = uclip(p1 + a2);
    p[(o as isize - step) as usize] = uclip(p0 + a1);
    p[o] = uclip(q0 - a1);
    p[(o as isize + step) as usize] = uclip(q1 - a2);
    p[(o as isize + 2 * step) as usize] = uclip(q2 - a3);
}

fn hev(p: &[u8], o: usize, step: isize, thresh: i32) -> bool {
    let g = |d: isize| p[(o as isize + d * step) as usize] as i32;
    (g(-2) - g(-1)).abs() > thresh || (g(1) - g(0)).abs() > thresh
}

fn needs_filter(p: &[u8], o: usize, step: isize, thr: i32) -> bool {
    let g = |d: isize| p[(o as isize + d * step) as usize] as i32;
    4 * (g(-1) - g(0)).abs() + (g(-2) - g(1)).abs() <= thr
}

fn needs_filter2(p: &[u8], o: usize, step: isize, thr: i32, it: i32) -> bool {
    let g = |d: isize| p[(o as isize + d * step) as usize] as i32;
    let (p3, p2, p1, p0, q0, q1, q2, q3) =
        (g(-4), g(-3), g(-2), g(-1), g(0), g(1), g(2), g(3));
    if 4 * (p0 - q0).abs() + (p1 - q1).abs() > thr { return false; }
    (p3 - p2).abs() <= it && (p2 - p1).abs() <= it && (p1 - p0).abs() <= it
        && (q3 - q2).abs() <= it && (q2 - q1).abs() <= it && (q1 - q0).abs() <= it
}

// FilterLoop26/24 : hstride = pas a travers l'arete, vstride = pas le long.
fn filter_loop(p: &mut [u8], mut o: usize, hstride: isize, vstride: usize, size: usize,
               thresh: i32, ithresh: i32, hev_t: i32, six: bool) {
    let thresh2 = 2 * thresh + 1;
    for _ in 0..size {
        if needs_filter2(p, o, hstride, thresh2, ithresh) {
            if hev(p, o, hstride, hev_t) {
                do_filter2(p, o, hstride);
            } else if six {
                do_filter6(p, o, hstride);
            } else {
                do_filter4(p, o, hstride);
            }
        }
        o += vstride;
    }
}

fn simple_filter16(p: &mut [u8], o: usize, hstride: isize, vstride: usize, thresh: i32) {
    let thresh2 = 2 * thresh + 1;
    let mut oo = o;
    for _ in 0..16 {
        if needs_filter(p, oo, hstride, thresh2) { do_filter2(p, oo, hstride); }
        oo += vstride;
    }
}

// ── YUV 4:2:0 → RGB (dsp/yuv.h + upsampling.c, « fancy », bit-exact) ────────

fn mult_hi(v: i32, coeff: i32) -> i32 { (v * coeff) >> 8 }
fn yuv_clip8(v: i32) -> u32 {
    if v & !((256 << 6) - 1) == 0 { (v >> 6) as u32 } else if v < 0 { 0 } else { 255 }
}
fn yuv_to_rgb(y: i32, u: i32, v: i32) -> u32 {
    let r = yuv_clip8(mult_hi(y, 19077) + mult_hi(v, 26149) - 14234);
    let g = yuv_clip8(mult_hi(y, 19077) - mult_hi(u, 6419) - mult_hi(v, 13320) + 8708);
    let b = yuv_clip8(mult_hi(y, 19077) + mult_hi(u, 33050) - 17685);
    (r << 16) | (g << 8) | b
}

// Paire de lignes du suréchantillonneur « fancy » (macro UPSAMPLE_FUNC).
// u/v en paires (u,v) 32 bits comme LOAD_UV.
#[allow(clippy::too_many_arguments)]
fn upsample_pair(top_y: &[u8], bottom_y: Option<&[u8]>,
                 top_u: &[u8], top_v: &[u8], cur_u: &[u8], cur_v: &[u8],
                 top_dst: &mut [u32], mut bottom_dst: Option<&mut [u32]>, len: usize) {
    let load = |u: u8, v: u8| -> u32 { u as u32 | ((v as u32) << 16) };
    let emit = |y: u8, uv: u32| -> u32 { yuv_to_rgb(y as i32, (uv & 0xff) as i32, (uv >> 16) as i32) };
    let last_pixel_pair = (len - 1) >> 1;
    let mut tl_uv = load(top_u[0], top_v[0]);
    let mut l_uv = load(cur_u[0], cur_v[0]);
    {
        let uv0 = (3 * tl_uv + l_uv + 0x0002_0002) >> 2;
        top_dst[0] = emit(top_y[0], uv0);
    }
    if let (Some(by), Some(bd)) = (bottom_y, bottom_dst.as_deref_mut()) {
        let uv0 = (3 * l_uv + tl_uv + 0x0002_0002) >> 2;
        bd[0] = emit(by[0], uv0);
    }
    for x in 1..=last_pixel_pair {
        let t_uv = load(top_u[x], top_v[x]);
        let uv = load(cur_u[x], cur_v[x]);
        let avg = tl_uv + t_uv + l_uv + uv + 0x0008_0008;
        let diag_12 = (avg + 2 * (t_uv + l_uv)) >> 3;
        let diag_03 = (avg + 2 * (tl_uv + uv)) >> 3;
        {
            let uv0 = (diag_12 + tl_uv) >> 1;
            let uv1 = (diag_03 + t_uv) >> 1;
            top_dst[2 * x - 1] = emit(top_y[2 * x - 1], uv0);
            if 2 * x < len { top_dst[2 * x] = emit(top_y[2 * x], uv1); }
        }
        if let (Some(by), Some(bd)) = (bottom_y, bottom_dst.as_deref_mut()) {
            let uv0 = (diag_03 + l_uv) >> 1;
            let uv1 = (diag_12 + uv) >> 1;
            bd[2 * x - 1] = emit(by[2 * x - 1], uv0);
            if 2 * x < len { bd[2 * x] = emit(by[2 * x], uv1); }
        }
        tl_uv = t_uv;
        l_uv = uv;
    }
    if len & 1 == 0 {
        {
            let uv0 = (3 * tl_uv + l_uv + 0x0002_0002) >> 2;
            top_dst[len - 1] = emit(top_y[len - 1], uv0);
        }
        if let (Some(by), Some(bd)) = (bottom_y, bottom_dst) {
            let uv0 = (3 * l_uv + tl_uv + 0x0002_0002) >> 2;
            bd[len - 1] = emit(by[len - 1], uv0);
        }
    }
}

// ── En-têtes ─────────────────────────────────────────────────────────────────

fn parse_header<'a>(data: &'a [u8]) -> Option<(Header, BoolDec<'a>, Vec<BoolDec<'a>>, usize, usize)> {
    if data.len() < 10 { return None; }
    let bits = data[0] as u32 | (data[1] as u32) << 8 | (data[2] as u32) << 16;
    let key_frame = bits & 1 == 0;
    let show = (bits >> 4) & 1;
    let part0_len = (bits >> 5) as usize;
    if !key_frame || show == 0 { return None; }
    if data[3] != 0x9d || data[4] != 0x01 || data[5] != 0x2a { return None; }
    let width = ((data[6] as usize) | (data[7] as usize) << 8) & 0x3fff;
    let height = ((data[8] as usize) | (data[9] as usize) << 8) & 0x3fff;
    if width == 0 || height == 0 || width * height > 40_000_000 { return None; }
    let rest = &data[10..];
    if part0_len > rest.len() { return None; }
    let mut br = BoolDec::new(&rest[..part0_len]);
    let token_data = &rest[part0_len..];

    let mut h = Header {
        mb_w: (width + 15) >> 4,
        mb_h: (height + 15) >> 4,
        use_segment: false,
        update_map: false,
        segment_probs: [255; 3],
        filter_type: 0,
        fstrengths: [[(0, 0, 0, false); 2]; 4],
        dqm: [QuantMat::default(); 4],
        coeffs: [[[[0; 11]; 3]; 8]; 4],
        use_skip_proba: false,
        skip_p: 0,
    };

    // colorspace + clamp (keyframe)
    let _ = br.flag();
    let _ = br.flag();

    // Segments (§9.3)
    let mut seg_quant = [0i32; 4];
    let mut seg_filter = [0i32; 4];
    let mut absolute_delta = false;
    h.use_segment = br.flag();
    if h.use_segment {
        h.update_map = br.flag();
        if br.flag() {
            absolute_delta = br.flag();
            for s in 0..4 {
                seg_quant[s] = if br.flag() { br.signed_value(7) } else { 0 };
            }
            for s in 0..4 {
                seg_filter[s] = if br.flag() { br.signed_value(6) } else { 0 };
            }
        }
        if h.update_map {
            for s in 0..3 {
                h.segment_probs[s] = if br.flag() { br.value_bits(8) as u8 } else { 255 };
            }
        }
    }

    // Filtre (§9.4)
    let f_simple = br.flag();
    let f_level = br.value_bits(6) as i32;
    let f_sharpness = br.value_bits(3) as i32;
    let use_lf_delta = br.flag();
    let mut ref_lf_delta = [0i32; 4];
    let mut mode_lf_delta = [0i32; 4];
    if use_lf_delta && br.flag() {
        for d in ref_lf_delta.iter_mut() {
            if br.flag() { *d = br.signed_value(6); }
        }
        for d in mode_lf_delta.iter_mut() {
            if br.flag() { *d = br.signed_value(6); }
        }
    }
    h.filter_type = if f_level == 0 { 0 } else if f_simple { 1 } else { 2 };

    // Forces de filtre par (segment, i4x4) — PrecomputeFilterStrengths.
    if h.filter_type > 0 {
        for s in 0..4 {
            let mut base_level = if h.use_segment {
                let mut l = seg_filter[s];
                if !absolute_delta { l += f_level; }
                l
            } else { f_level };
            for i4 in 0..2 {
                let mut level = base_level;
                if use_lf_delta {
                    level += ref_lf_delta[0];
                    if i4 == 1 { level += mode_lf_delta[0]; }
                }
                level = level.clamp(0, 63);
                if level > 0 {
                    let mut ilevel = level;
                    if f_sharpness > 0 {
                        if f_sharpness > 4 { ilevel >>= 2; } else { ilevel >>= 1; }
                        if ilevel > 9 - f_sharpness { ilevel = 9 - f_sharpness; }
                    }
                    if ilevel < 1 { ilevel = 1; }
                    let hev_t = if level >= 40 { 2 } else if level >= 15 { 1 } else { 0 };
                    h.fstrengths[s][i4] = ((2 * level + ilevel) as u8, ilevel as u8, hev_t as u8, i4 == 1);
                } else {
                    h.fstrengths[s][i4] = (0, 0, 0, i4 == 1);
                }
            }
            let _ = &mut base_level;
        }
    }

    // Partitions de tokens (§9.5)
    let num_parts = 1usize << br.value_bits(2);
    let last_part = num_parts - 1;
    if token_data.len() < 3 * last_part { return None; }
    let mut parts: Vec<BoolDec> = Vec::with_capacity(num_parts);
    let mut part_start = 3 * last_part;
    let mut size_left = token_data.len() - 3 * last_part;
    for p in 0..last_part {
        let sz = &token_data[p * 3..];
        let mut psize = sz[0] as usize | (sz[1] as usize) << 8 | (sz[2] as usize) << 16;
        if psize > size_left { psize = size_left; }
        parts.push(BoolDec::new(&token_data[part_start..part_start + psize]));
        part_start += psize;
        size_left -= psize;
    }
    parts.push(BoolDec::new(&token_data[part_start..]));

    // Quantification (§9.6) — VP8ParseQuant.
    let base_q0 = br.value_bits(7) as i32;
    let dqy1_dc = if br.flag() { br.signed_value(4) } else { 0 };
    let dqy2_dc = if br.flag() { br.signed_value(4) } else { 0 };
    let dqy2_ac = if br.flag() { br.signed_value(4) } else { 0 };
    let dquv_dc = if br.flag() { br.signed_value(4) } else { 0 };
    let dquv_ac = if br.flag() { br.signed_value(4) } else { 0 };
    for s in 0..4 {
        let q = if h.use_segment {
            if absolute_delta { seg_quant[s] } else { seg_quant[s] + base_q0 }
        } else {
            if s > 0 { h.dqm[s] = h.dqm[0]; continue; }
            base_q0
        };
        let m = &mut h.dqm[s];
        m.y1[0] = t::DC_TABLE[clip127(q + dqy1_dc, 127)] as i32;
        m.y1[1] = t::AC_TABLE[clip127(q, 127)] as i32;
        m.y2[0] = t::DC_TABLE[clip127(q + dqy2_dc, 127)] as i32 * 2;
        m.y2[1] = (t::AC_TABLE[clip127(q + dqy2_ac, 127)] as i32 * 101581) >> 16;
        if m.y2[1] < 8 { m.y2[1] = 8; }
        m.uv[0] = t::DC_TABLE[clip127(q + dquv_dc, 117)] as i32;
        m.uv[1] = t::AC_TABLE[clip127(q + dquv_ac, 127)] as i32;
    }

    // update_proba (ignoré sur keyframe unique), puis probas de coefficients.
    let _ = br.flag();
    for ty in 0..4 {
        for b in 0..8 {
            for c in 0..3 {
                for p in 0..11 {
                    let idx = ((ty * 8 + b) * 3 + c) * 11 + p;
                    h.coeffs[ty][b][c][p] = if br.bit(t::COEFFS_UPDATE_PROBA[idx]) != 0 {
                        br.value_bits(8) as u8
                    } else {
                        t::COEFFS_PROBA0[idx]
                    };
                }
            }
        }
    }
    h.use_skip_proba = br.flag();
    if h.use_skip_proba { h.skip_p = br.value_bits(8) as u8; }
    if br.eof { return None; }

    Some((h, br, parts, width, height))
}

// ── Modes intra d'une rangée (VP8ParseIntraModeRow) ──────────────────────────

fn parse_intra_row(br: &mut BoolDec, h: &Header, intra_t: &mut [u8], intra_l: &mut [u8; 4], row: &mut [MbData]) {
    for mb_x in 0..h.mb_w {
        let blk = &mut row[mb_x];
        blk.segment = if h.update_map {
            if br.bit(h.segment_probs[0]) == 0 {
                br.bit(h.segment_probs[1]) as usize
            } else {
                br.bit(h.segment_probs[2]) as usize + 2
            }
        } else { 0 };
        if h.use_skip_proba { blk.skip = br.bit(h.skip_p) != 0; }
        blk.is_i4x4 = br.bit(145) == 0;
        if !blk.is_i4x4 {
            // Arbre 16x16 : DC=0, V=2, H=3, TM=1.
            let ymode = if br.bit(156) != 0 {
                if br.bit(128) != 0 { 1u8 } else { 3 }
            } else if br.bit(163) != 0 { 2 } else { 0 };
            blk.imodes[0] = ymode;
            for i in 0..4 { intra_t[4 * mb_x + i] = ymode; intra_l[i] = ymode; }
        } else {
            for y in 0..4 {
                let mut ymode = intra_l[y];
                for x in 0..4 {
                    let top = intra_t[4 * mb_x + x] as usize;
                    let prob = &t::KF_BMODE_PROBA[(top * 10 + ymode as usize) * 9..];
                    // Arbre 4x4 en dur (tree_dec.c).
                    ymode = if br.bit(prob[0]) == 0 { 0 }
                        else if br.bit(prob[1]) == 0 { 1 }
                        else if br.bit(prob[2]) == 0 { 2 }
                        else if br.bit(prob[3]) == 0 {
                            if br.bit(prob[4]) == 0 { 3 }
                            else if br.bit(prob[5]) == 0 { 4 } else { 5 }
                        } else if br.bit(prob[6]) == 0 { 6 }
                        else if br.bit(prob[7]) == 0 { 7 }
                        else if br.bit(prob[8]) == 0 { 8 } else { 9 };
                    blk.imodes[y * 4 + x] = ymode;
                    intra_t[4 * mb_x + x] = ymode;
                }
                intra_l[y] = ymode;
            }
        }
        // Arbre chroma : DC=0, V=2, TM=1, H=3.
        blk.uvmode = if br.bit(142) == 0 { 0 }
            else if br.bit(114) == 0 { 2 }
            else if br.bit(183) != 0 { 1 } else { 3 };
    }
}

// ── Résidus d'un macrobloc (ParseResiduals) ─────────────────────────────────

fn nz_code_bits(nz_coeffs: u32, nz: usize, dc_nz: u32) -> u32 {
    (nz_coeffs << 2) | if nz > 3 { 3 } else if nz > 1 { 2 } else { dc_nz }
}

#[allow(clippy::too_many_arguments)]
fn parse_residuals(br: &mut BoolDec, h: &Header, blk: &mut MbData,
                   top_nz: &mut u8, top_nz_dc: &mut u8, left_nz: &mut u8, left_nz_dc: &mut u8) -> bool {
    let q = &h.dqm[blk.segment];
    blk.coeffs = [0; 384];
    let first;
    let ac_type;
    if !blk.is_i4x4 {
        let mut dc = [0i16; 16];
        let ctx = (*top_nz_dc + *left_nz_dc) as usize;
        let nz = get_coeffs(br, &h.coeffs[1], ctx, &q.y2, 0, &mut dc);
        let has = (nz > 0) as u8;
        *top_nz_dc = has;
        *left_nz_dc = has;
        if nz > 1 {
            transform_wht(&dc, &mut blk.coeffs);
        } else {
            let dc0 = ((dc[0] as i32 + 3) >> 3) as i16;
            for i in 0..16 { blk.coeffs[i * 16] = dc0; }
        }
        first = 1;
        ac_type = 0;
    } else {
        first = 0;
        ac_type = 3;
    }

    let mut tnz = (*top_nz & 0x0f) as u32;
    let mut lnz = (*left_nz & 0x0f) as u32;
    let mut non_zero_y = 0u32;
    for y in 0..4 {
        let mut l = lnz & 1;
        let mut nz_coeffs = 0u32;
        for x in 0..4 {
            let ctx = (l + (tnz & 1)) as usize;
            let dst = &mut blk.coeffs[(y * 4 + x) * 16..(y * 4 + x) * 16 + 16];
            let nz = get_coeffs(br, &h.coeffs[ac_type], ctx, &q.y1, first, dst);
            l = (nz > first) as u32;
            tnz = (tnz >> 1) | (l << 7);
            nz_coeffs = nz_code_bits(nz_coeffs, nz, (dst[0] != 0) as u32);
        }
        tnz >>= 4;
        lnz = (lnz >> 1) | (l << 7);
        non_zero_y = (non_zero_y << 8) | nz_coeffs;
    }
    let mut out_t_nz = tnz;
    let mut out_l_nz = lnz >> 4;

    let mut non_zero_uv = 0u32;
    for ch in [0usize, 2] {
        let mut nz_coeffs = 0u32;
        tnz = (*top_nz as u32) >> (4 + ch);
        lnz = (*left_nz as u32) >> (4 + ch);
        for y in 0..2 {
            let mut l = lnz & 1;
            for x in 0..2 {
                let ctx = (l + (tnz & 1)) as usize;
                let bi = 16 + ch * 2 + y * 2 + x;
                let dst = &mut blk.coeffs[bi * 16..bi * 16 + 16];
                let nz = get_coeffs(br, &h.coeffs[2], ctx, &q.uv, 0, dst);
                l = (nz > 0) as u32;
                tnz = (tnz >> 1) | (l << 3);
                nz_coeffs = nz_code_bits(nz_coeffs, nz, (dst[0] != 0) as u32);
            }
            tnz >>= 2;
            lnz = (lnz >> 1) | (l << 5);
        }
        non_zero_uv |= nz_coeffs << (4 * ch);
        out_t_nz |= (tnz << 4) << ch;
        out_l_nz |= (lnz & 0xf0) << ch;
    }
    *top_nz = out_t_nz as u8;
    *left_nz = out_l_nz as u8;
    blk.non_zero_y = non_zero_y;
    blk.non_zero_uv = non_zero_uv;
    (non_zero_y | non_zero_uv) == 0
}

// ── Reconstruction d'une rangée (ReconstructRow) ─────────────────────────────

struct TopSmp { y: [u8; 16], u: [u8; 8], v: [u8; 8] }

fn do_transform(bits: u32, cf: &[i16], buf: &mut [u8], off: usize) {
    match bits >> 30 {
        3 => transform_one(cf, buf, off),
        2 => transform_ac3(cf, buf, off),
        1 => transform_dc(cf, buf, off),
        _ => {}
    }
}

fn do_uv_transform(bits: u32, cf: &[i16], buf: &mut [u8], off: usize) {
    if bits & 0xff != 0 {
        if bits & 0xaa != 0 {
            transform_one(&cf[0..], buf, off);
            transform_one(&cf[16..], buf, off + 4);
            transform_one(&cf[32..], buf, off + 4 * BPS);
            transform_one(&cf[48..], buf, off + 4 * BPS + 4);
        } else {
            if cf[0] != 0 { transform_dc(&cf[0..], buf, off); }
            if cf[16] != 0 { transform_dc(&cf[16..], buf, off + 4); }
            if cf[32] != 0 { transform_dc(&cf[32..], buf, off + 4 * BPS); }
            if cf[48] != 0 { transform_dc(&cf[48..], buf, off + 4 * BPS + 4); }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_row(h: &Header, mb_y: usize, row: &[MbData], yuv_b: &mut [u8; YUV_SIZE],
                   yuv_t: &mut [TopSmp], py: &mut [u8], pu: &mut [u8], pv: &mut [u8],
                   ys: usize, us: usize) {
    // Bord gauche 129.
    for j in 0..16 { yuv_b[Y_OFF + j * BPS - 1] = 129; }
    for j in 0..8 {
        yuv_b[U_OFF + j * BPS - 1] = 129;
        yuv_b[V_OFF + j * BPS - 1] = 129;
    }
    if mb_y > 0 {
        yuv_b[Y_OFF - 1 - BPS] = 129;
        yuv_b[U_OFF - 1 - BPS] = 129;
        yuv_b[V_OFF - 1 - BPS] = 129;
    } else {
        // Bord haut 127 (valable pour toute la premiere rangee).
        for i in 0..16 + 4 + 1 { yuv_b[Y_OFF - BPS - 1 + i] = 127; }
        for i in 0..8 + 1 {
            yuv_b[U_OFF - BPS - 1 + i] = 127;
            yuv_b[V_OFF - BPS - 1 + i] = 127;
        }
    }

    for mb_x in 0..h.mb_w {
        let blk = &row[mb_x];
        // Fait tourner les echantillons gauches depuis le MB precedent.
        if mb_x > 0 {
            for j in -1i32..16 {
                let base = (Y_OFF as i32 + j * BPS as i32) as usize;
                for k in 0..4 { yuv_b[base - 4 + k] = yuv_b[base + 12 + k]; }
            }
            for j in -1i32..8 {
                let bu = (U_OFF as i32 + j * BPS as i32) as usize;
                let bv = (V_OFF as i32 + j * BPS as i32) as usize;
                for k in 0..4 {
                    yuv_b[bu - 4 + k] = yuv_b[bu + 4 + k];
                    yuv_b[bv - 4 + k] = yuv_b[bv + 4 + k];
                }
            }
        }
        // Charge les echantillons du haut.
        if mb_y > 0 {
            for i in 0..16 { yuv_b[Y_OFF - BPS + i] = yuv_t[mb_x].y[i]; }
            for i in 0..8 {
                yuv_b[U_OFF - BPS + i] = yuv_t[mb_x].u[i];
                yuv_b[V_OFF - BPS + i] = yuv_t[mb_x].v[i];
            }
        }

        let mut bits = blk.non_zero_y;
        if blk.is_i4x4 {
            // top-right : 4 pixels.
            if mb_y > 0 {
                if mb_x >= h.mb_w - 1 {
                    let v = yuv_t[mb_x].y[15];
                    for k in 0..4 { yuv_b[Y_OFF - BPS + 16 + k] = v; }
                } else {
                    for k in 0..4 { yuv_b[Y_OFF - BPS + 16 + k] = yuv_t[mb_x + 1].y[k]; }
                }
            }
            // Replique les top-right sur les rangees 3, 7, 11 (cols 16..19).
            for r in 1..4 {
                for k in 0..4 {
                    yuv_b[Y_OFF - BPS + 16 + r * 4 * BPS + k] = yuv_b[Y_OFF - BPS + 16 + k];
                }
            }
            for n in 0..16 {
                let dst = Y_OFF + SCAN[n];
                pred4(yuv_b, dst, blk.imodes[n]);
                do_transform(bits, &blk.coeffs[n * 16..n * 16 + 16], yuv_b, dst);
                bits <<= 2;
            }
        } else {
            let mode = check_mode(mb_x, mb_y, blk.imodes[0]);
            // Modes 16x16 : 0 DC, 1 TM, 2 VE, 3 HE, 4..6 DC bords.
            pred_nxn(yuv_b, Y_OFF, 16, mode);
            if bits != 0 {
                for n in 0..16 {
                    do_transform(bits, &blk.coeffs[n * 16..n * 16 + 16], yuv_b, Y_OFF + SCAN[n]);
                    bits <<= 2;
                }
            }
        }
        // Chroma.
        let uvm = check_mode(mb_x, mb_y, blk.uvmode);
        pred_nxn(yuv_b, U_OFF, 8, uvm);
        pred_nxn(yuv_b, V_OFF, 8, uvm);
        do_uv_transform(blk.non_zero_uv, &blk.coeffs[16 * 16..], yuv_b, U_OFF);
        do_uv_transform(blk.non_zero_uv >> 8, &blk.coeffs[20 * 16..], yuv_b, V_OFF);

        // Sauve les echantillons du bas pour la rangee suivante.
        if mb_y < h.mb_h - 1 {
            for i in 0..16 { yuv_t[mb_x].y[i] = yuv_b[Y_OFF + 15 * BPS + i]; }
            for i in 0..8 {
                yuv_t[mb_x].u[i] = yuv_b[U_OFF + 7 * BPS + i];
                yuv_t[mb_x].v[i] = yuv_b[V_OFF + 7 * BPS + i];
            }
        }
        // Copie vers les plans pleine image.
        let yo = mb_y * 16 * ys + mb_x * 16;
        let uo = mb_y * 8 * us + mb_x * 8;
        for j in 0..16 {
            for i in 0..16 { py[yo + j * ys + i] = yuv_b[Y_OFF + j * BPS + i]; }
        }
        for j in 0..8 {
            for i in 0..8 {
                pu[uo + j * us + i] = yuv_b[U_OFF + j * BPS + i];
                pv[uo + j * us + i] = yuv_b[V_OFF + j * BPS + i];
            }
        }
    }
}

// ── Filtre de boucle sur les plans (DoFilter) ────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn filter_mb(h: &Header, fi: (u8, u8, u8, bool), mb_x: usize, mb_y: usize,
             py: &mut [u8], pu: &mut [u8], pv: &mut [u8], ys: usize, us: usize) {
    let (limit, ilevel, hev_t, inner) = (fi.0 as i32, fi.1 as i32, fi.2 as i32, fi.3);
    if limit == 0 { return; }
    let yo = mb_y * 16 * ys + mb_x * 16;
    if h.filter_type == 1 {
        // Simple : Y uniquement.
        if mb_x > 0 { simple_filter16(py, yo, 1, ys, limit + 4); }
        if inner {
            for k in 1..4 { simple_filter16(py, yo + 4 * k, 1, ys, limit); }
        }
        if mb_y > 0 { simple_filter16(py, yo, ys as isize, 1, limit + 4); }
        if inner {
            for k in 1..4 { simple_filter16(py, yo + 4 * k * ys, ys as isize, 1, limit); }
        }
    } else {
        let uo = mb_y * 8 * us + mb_x * 8;
        if mb_x > 0 {
            filter_loop(py, yo, 1, ys, 16, limit + 4, ilevel, hev_t, true);
            filter_loop(pu, uo, 1, us, 8, limit + 4, ilevel, hev_t, true);
            filter_loop(pv, uo, 1, us, 8, limit + 4, ilevel, hev_t, true);
        }
        if inner {
            for k in 1..4 { filter_loop(py, yo + 4 * k, 1, ys, 16, limit, ilevel, hev_t, false); }
            filter_loop(pu, uo + 4, 1, us, 8, limit, ilevel, hev_t, false);
            filter_loop(pv, uo + 4, 1, us, 8, limit, ilevel, hev_t, false);
        }
        if mb_y > 0 {
            filter_loop(py, yo, ys as isize, 1, 16, limit + 4, ilevel, hev_t, true);
            filter_loop(pu, uo, us as isize, 1, 8, limit + 4, ilevel, hev_t, true);
            filter_loop(pv, uo, us as isize, 1, 8, limit + 4, ilevel, hev_t, true);
        }
        if inner {
            for k in 1..4 { filter_loop(py, yo + 4 * k * ys, ys as isize, 1, 16, limit, ilevel, hev_t, false); }
            filter_loop(pu, uo + 4 * us, us as isize, 1, 8, limit, ilevel, hev_t, false);
            filter_loop(pv, uo + 4 * us, us as isize, 1, 8, limit, ilevel, hev_t, false);
        }
    }
}

// ── Point d'entrée ───────────────────────────────────────────────────────────

/// Décode un flux VP8 (contenu du chunk `VP8 `) en pixels 0x00RRGGBB.
pub fn decode_frame(data: &[u8]) -> Option<Image> {
    let (h, mut br0, mut parts, width, height) = parse_header(data)?;
    let (mb_w, mb_h) = (h.mb_w, h.mb_h);
    let ys = mb_w * 16;
    let us = mb_w * 8;
    let mut py = vec![0u8; ys * mb_h * 16];
    let mut pu = vec![0u8; us * mb_h * 8];
    let mut pv = vec![0u8; us * mb_h * 8];
    let mut yuv_b = [0u8; YUV_SIZE];
    let mut yuv_t: Vec<TopSmp> = (0..mb_w).map(|_| TopSmp { y: [0; 16], u: [0; 8], v: [0; 8] }).collect();
    let mut intra_t = vec![0u8; 4 * mb_w]; // B_DC_PRED
    let mut row: Vec<MbData> = vec![MbData::default(); mb_w];
    let mut top_nz = vec![0u8; mb_w];
    let mut top_nz_dc = vec![0u8; mb_w];
    let mut f_info: Vec<(u8, u8, u8, bool)> = Vec::with_capacity(mb_w * mb_h);
    let num_parts_minus_one = parts.len() - 1;

    for mb_y in 0..mb_h {
        let mut intra_l = [0u8; 4];
        parse_intra_row(&mut br0, &h, &mut intra_t, &mut intra_l, &mut row);
        if br0.eof { return None; }
        let token_br = &mut parts[mb_y & num_parts_minus_one];
        let mut left_nz = 0u8;
        let mut left_nz_dc = 0u8;
        for mb_x in 0..mb_w {
            let blk = &mut row[mb_x];
            let mut skip = if h.use_skip_proba { blk.skip } else { false };
            if !skip {
                skip = parse_residuals(token_br, &h, blk,
                    &mut top_nz[mb_x], &mut top_nz_dc[mb_x], &mut left_nz, &mut left_nz_dc);
            } else {
                left_nz = 0;
                top_nz[mb_x] = 0;
                if !blk.is_i4x4 {
                    left_nz_dc = 0;
                    top_nz_dc[mb_x] = 0;
                }
                blk.coeffs = [0; 384];
                blk.non_zero_y = 0;
                blk.non_zero_uv = 0;
            }
            if h.filter_type > 0 {
                let mut fi = h.fstrengths[blk.segment][blk.is_i4x4 as usize];
                fi.3 |= !skip;
                f_info.push(fi);
            }
        }
        reconstruct_row(&h, mb_y, &row, &mut yuv_b, &mut yuv_t, &mut py, &mut pu, &mut pv, ys, us);
    }

    // Filtre de boucle (apres reconstruction complete : la prediction intra
    // utilise les pixels NON filtres, comme le fait libwebp via yuv_t).
    if h.filter_type > 0 {
        for mb_y in 0..mb_h {
            for mb_x in 0..mb_w {
                filter_mb(&h, f_info[mb_y * mb_w + mb_x], mb_x, mb_y, &mut py, &mut pu, &mut pv, ys, us);
            }
        }
    }

    // YUV 4:2:0 -> RGB, surechantillonnage « fancy » (EmitFancyRGB).
    let mut pix = vec![0u32; width * height];
    let uv_h = (height + 1) / 2;
    let yrow = |j: usize| &py[j * ys..j * ys + width];
    // Ligne 0 : miroir.
    upsample_pair(yrow(0), None, &pu[0..], &pv[0..], &pu[0..], &pv[0..],
                  &mut pix[0..width], None, width);
    let mut j = 1usize;
    let mut k = 0usize;
    while j + 1 < height {
        let (top_u, cur_u) = (&pu[k * us..], &pu[(k + 1).min(uv_h - 1) * us..]);
        let (top_v, cur_v) = (&pv[k * us..], &pv[(k + 1).min(uv_h - 1) * us..]);
        let (before, after) = pix.split_at_mut((j + 1) * width);
        let top_dst = &mut before[j * width..];
        let bottom_dst = &mut after[..width];
        upsample_pair(yrow(j), Some(yrow(j + 1)), top_u, top_v, cur_u, cur_v,
                      top_dst, Some(bottom_dst), width);
        j += 2;
        k += 1;
    }
    if j < height {
        // Derniere ligne (hauteur paire) : miroir bas.
        let u = &pu[(uv_h - 1) * us..];
        let v = &pv[(uv_h - 1) * us..];
        let dst = &mut pix[j * width..(j + 1) * width];
        upsample_pair(yrow(j), None, u, v, u, v, dst, None, width);
    }

    Some(Image { w: width, h: height, pix })
}
