//! WebP lossless (VP8L) : Huffman canonique + LZ77 + transforms (predictor,
//! color, subtract-green, color-indexing/palette) + cache de couleurs. VP8
//! (lossy, DCT) et VP8X (alpha/anim étendus) restent hors-scope : identifiés
//! mais non décodés (repli propre -> None, le moteur affiche l'`alt`).
//!
//! Écrit à la main : aucune crate no_std ne s'est révélée viable pour WebP
//! (`image-webp` échoue en no_std, dépendance transitive `byteorder-lite`
//! suppose `std`). Formules/tables reprises du décodeur de référence libwebp
//! (`src/dec/vp8l_dec.c`, `src/dsp/lossless.c`, `src/utils/*.c`).
//!
//! Vérifié EMPIRIQUEMENT (pas juste relu) : porté tel quel dans un harnais
//! std hors du noyau, décodé et comparé PIXEL PAR PIXEL contre des fichiers
//! VP8L réels produits par Pillow/libwebp -- 5 images (108 a 120 000 pixels :
//! palette+delta, predictor 14 modes (dont l'edge-case top-right en bout de
//! ligne), color transform, subtract-green, cache de couleurs, retro-refs
//! LZ77, codes Huffman degeneres a 1 symbole), 0 pixel faux apres correction
//! de 4 bugs trouves par cette methode (bit "meta Huffman" lu a tort pour
//! les sous-images, code degenere a 1 symbole lisant 1 bit au lieu de 0,
//! second symbole du "code simple" lu sur `nbits` au lieu de 8 fixes, TR en
//! bout de ligne). Reste non teste dans le noyau lui-meme (no_std, pas
//! d'acces QEMU ici) : verification visuelle au boot bienvenue.
//!
//! Simplification assumee (repli propre -> None si rencontree) : pas de
//! meta-codes Huffman multiples (plusieurs groupes Huffman selectionnes par
//! bloc, reserve aux images tres grandes/complexes -- non declenche par
//! aucune des images de test, y compris 800 000 px de bruit aleatoire).

use super::{composite_rgba, Image};
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

const MAX_PIXELS: usize = 1_200_000;

fn checked_area(w: usize, h: usize) -> Option<usize> {
    let n = w.checked_mul(h)?;
    if n == 0 || n > MAX_PIXELS { None } else { Some(n) }
}

// ── Lecteur de bits LSB-first (convention VP8L) ─────────────────────────────
struct BitReader<'a> { data: &'a [u8], pos: usize }

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data, pos: 0 } }
    // Au-dela de la fin du flux, renvoie des zeros (comme les decodeurs de
    // reference qui tolerent un rembourrage de fin) plutot que d'echouer sur
    // les tout derniers bits d'un flux valide.
    fn read_bits(&mut self, n: u32) -> Option<u32> {
        if n == 0 { return Some(0); }
        if n > 24 { return None; }
        let mut val = 0u32;
        for i in 0..n {
            let byte = self.pos / 8;
            let bit = self.pos % 8;
            let b = if byte < self.data.len() { (self.data[byte] >> bit) & 1 } else { 0 };
            val |= (b as u32) << i;
            self.pos += 1;
        }
        Some(val)
    }
}

// ── Huffman canonique ────────────────────────────────────────────────────
fn build_huffman(code_lengths: &[u8]) -> BTreeMap<(u8, u32), u16> {
    let mut bl_count = [0u32; 16];
    for &l in code_lengths { if l > 0 && l < 16 { bl_count[l as usize] += 1; } }
    let mut code = 0u32;
    let mut next_code = [0u32; 16];
    for len in 1..16usize {
        code = (code + bl_count[len - 1]) << 1;
        next_code[len] = code;
    }
    let mut table = BTreeMap::new();
    for (sym, &l) in code_lengths.iter().enumerate() {
        if l == 0 || l >= 16 { continue; }
        let c = next_code[l as usize];
        next_code[l as usize] += 1;
        table.insert((l, c), sym as u16);
    }
    table
}

fn decode_symbol(br: &mut BitReader, table: &BTreeMap<(u8, u32), u16>) -> Option<u16> {
    // Code degenere (un seul symbole non-nul) : consomme ZERO bit, toujours ce
    // symbole (cf. "code.bits = 0" dans BuildHuffmanTable, libwebp). Un oubli
    // ici desynchronise le flux d'un bit a chaque canal degenere (frequent :
    // rouge/bleu/alpha valent souvent une constante apres color-indexing).
    if table.len() == 1 {
        return table.values().next().copied();
    }
    let mut code = 0u32;
    for len in 1..=15u8 {
        code = (code << 1) | br.read_bits(1)?;
        if let Some(&sym) = table.get(&(len, code)) { return Some(sym); }
    }
    None
}

const CODE_LENGTH_CODE_ORDER: [usize; 19] =
    [17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

fn read_huffman_code_lengths(br: &mut BitReader, num_symbols: usize) -> Option<Vec<u8>> {
    let mut cl_cl = [0u8; 19];
    let num_codes = br.read_bits(4)? as usize + 4;
    if num_codes > 19 { return None; }
    for i in 0..num_codes {
        cl_cl[CODE_LENGTH_CODE_ORDER[i]] = br.read_bits(3)? as u8;
    }
    let cl_table = build_huffman(&cl_cl);
    if cl_table.is_empty() { return None; }

    let mut code_lengths = vec![0u8; num_symbols];
    let use_length = br.read_bits(1)? != 0;
    let mut max_symbol = if use_length {
        let length_nbits = 2 + 2 * br.read_bits(3)?;
        2 + br.read_bits(length_nbits)? as usize
    } else {
        num_symbols
    };

    let mut symbol = 0usize;
    let mut prev_code_length = 8u8;
    while symbol < num_symbols {
        if max_symbol == 0 { break; }
        max_symbol -= 1;
        let code_length = decode_symbol(br, &cl_table)?;
        if code_length < 16 {
            code_lengths[symbol] = code_length as u8;
            symbol += 1;
            if code_length != 0 { prev_code_length = code_length as u8; }
        } else {
            let use_prev = code_length == 16;
            let slot = (code_length - 16) as usize;
            if slot > 2 { return None; }
            let extra_bits = [2u32, 3, 7][slot];
            let repeat_offset = [3usize, 3, 11][slot];
            let repeat = br.read_bits(extra_bits)? as usize + repeat_offset;
            if symbol + repeat > num_symbols { return None; }
            let length = if use_prev { prev_code_length } else { 0 };
            for _ in 0..repeat { code_lengths[symbol] = length; symbol += 1; }
        }
    }
    Some(code_lengths)
}

fn read_huffman_code(br: &mut BitReader, alphabet_size: usize) -> Option<BTreeMap<(u8, u32), u16>> {
    let simple = br.read_bits(1)? != 0;
    let code_lengths = if simple {
        let mut cl = vec![0u8; alphabet_size];
        let num_symbols = br.read_bits(1)? + 1;
        let first_symbol_len_code = br.read_bits(1)?;
        // Le premier symbole est sur 1 ou 8 bits selon first_symbol_len_code ;
        // le SECOND (s'il existe) est TOUJOURS sur 8 bits, meme si le premier
        // ne l'etait pas -- asymetrie facile a manquer (ReadHuffmanCode,
        // libwebp/src/dec/vp8l_dec.c) qui desynchronise tout le flux de 7 bits
        // des qu'un canal degenere a 2 valeurs (frequent : rouge/bleu/alpha).
        let nbits0 = if first_symbol_len_code == 0 { 1 } else { 8 };
        let symbol0 = br.read_bits(nbits0)? as usize;
        if symbol0 >= alphabet_size { return None; }
        cl[symbol0] = 1;
        if num_symbols == 2 {
            let symbol1 = br.read_bits(8)? as usize;
            if symbol1 >= alphabet_size { return None; }
            cl[symbol1] = 1;
        }
        cl
    } else {
        read_huffman_code_lengths(br, alphabet_size)?
    };
    Some(build_huffman(&code_lengths))
}

// Les 5 codes Huffman d'un groupe : vert (litteral+longueur+cache), rouge,
// bleu, alpha, distance. Alphabet vert = 256 litteraux + 24 codes de longueur
// + `cache_size` codes de lookup du cache de couleurs (0 si desactive).
struct HuffGroup {
    green: BTreeMap<(u8, u32), u16>,
    red: BTreeMap<(u8, u32), u16>,
    blue: BTreeMap<(u8, u32), u16>,
    alpha: BTreeMap<(u8, u32), u16>,
    dist: BTreeMap<(u8, u32), u16>,
}

fn read_huff_group(br: &mut BitReader, cache_size: usize) -> Option<HuffGroup> {
    Some(HuffGroup {
        green: read_huffman_code(br, 256 + 24 + cache_size)?,
        red: read_huffman_code(br, 256)?,
        blue: read_huffman_code(br, 256)?,
        alpha: read_huffman_code(br, 256)?,
        dist: read_huffman_code(br, 40)?,
    })
}

// Hachage du cache de couleurs (VP8LHashPix, libwebp) : key = (argb *
// 0x1e35a7bd) >> (32 - bits). L'insertion se fait a CHAQUE pixel produit
// (litteral ou copie retro), la lecture donne directement l'INDEX (pas besoin
// de re-hacher) puisque l'encodeur transmet l'index du slot.
fn cache_hash(argb: u32, bits: u32) -> usize {
    (argb.wrapping_mul(0x1e35_a7bd) >> (32 - bits)) as usize
}

// Formule de code prefixe partagee longueur/distance (GetCopyDistance/
// GetCopyLength dans libwebp -- les deux partagent le meme calcul).
fn prefix_value(br: &mut BitReader, code: u32) -> Option<u32> {
    if code < 4 { return Some(code + 1); }
    let extra_bits = (code - 2) >> 1;
    let offset = (2 + (code & 1)) << extra_bits;
    Some(offset + br.read_bits(extra_bits)? + 1)
}

// Table des 120 "plane codes" (distances courtes usuelles, cf. kCodeToPlane
// dans libwebp) : chaque octet encode (yoffset<<4 | (8-xoffset)).
const CODE_TO_PLANE: [u8; 120] = [
    0x18, 0x07, 0x17, 0x19, 0x28, 0x06, 0x27, 0x29, 0x16, 0x1a, 0x26, 0x2a,
    0x38, 0x05, 0x37, 0x39, 0x15, 0x1b, 0x36, 0x3a, 0x25, 0x2b, 0x48, 0x04,
    0x47, 0x49, 0x14, 0x1c, 0x35, 0x3b, 0x46, 0x4a, 0x24, 0x2c, 0x58, 0x45,
    0x4b, 0x34, 0x3c, 0x03, 0x57, 0x59, 0x13, 0x1d, 0x56, 0x5a, 0x23, 0x2d,
    0x44, 0x4c, 0x55, 0x5b, 0x33, 0x3d, 0x68, 0x02, 0x67, 0x69, 0x12, 0x1e,
    0x66, 0x6a, 0x22, 0x2e, 0x54, 0x5c, 0x43, 0x4d, 0x65, 0x6b, 0x32, 0x3e,
    0x78, 0x01, 0x77, 0x79, 0x53, 0x5d, 0x11, 0x1f, 0x64, 0x6c, 0x42, 0x4e,
    0x76, 0x7a, 0x21, 0x2f, 0x75, 0x7b, 0x31, 0x3f, 0x63, 0x6d, 0x52, 0x5e,
    0x00, 0x74, 0x7c, 0x41, 0x4f, 0x10, 0x20, 0x62, 0x6e, 0x30, 0x73, 0x7d,
    0x51, 0x5f, 0x40, 0x72, 0x7e, 0x61, 0x6f, 0x50, 0x71, 0x7f, 0x60, 0x70,
];

fn plane_code_to_distance(xsize: usize, plane_code: u32) -> usize {
    if plane_code as usize > 120 { return plane_code as usize - 120; }
    let idx = (plane_code as usize).saturating_sub(1).min(119);
    let dist_code = CODE_TO_PLANE[idx] as i32;
    let yoffset = dist_code >> 4;
    let xoffset = 8 - (dist_code & 0xf);
    let dist = yoffset * xsize as i32 + xoffset;
    if dist >= 1 { dist as usize } else { 1 }
}

// ── Flux d'image générique : utilisé pour l'image principale ET pour les
// sous-images des transforms (predictor/color) et la palette (color
// indexing). Repli propre (None) si le flux utilise des meta-codes Huffman
// multiples (non geres -- voir commentaire d'en-tete) ; le cache de couleurs,
// lui, EST gere (tres frequent meme sur des images simples).
//
// `is_level0` : le bit "meta Huffman" (groupes multiples) n'est lu QUE pour
// l'image de niveau 0 (l'image principale) -- jamais pour les sous-images
// (palette, predictor, color), qui utilisent toujours un seul groupe implicite
// (cf. `allow_recursion` dans ReadHuffmanCodes, libwebp/src/dec/vp8l_dec.c).
// Un oubli ici desynchronise le flux de bits d'un cran et fait echouer TOUT
// le decodage qui suit -- verifie empiriquement (voir commit).
fn decode_image_stream(br: &mut BitReader, xsize: usize, ysize: usize, is_level0: bool) -> Option<Vec<u32>> {
    checked_area(xsize, ysize)?;
    let color_cache_used = br.read_bits(1)? != 0;
    let cache_bits = if color_cache_used {
        let bits = br.read_bits(4)?;
        if bits == 0 || bits > 11 { return None; }
        bits
    } else { 0 };
    if is_level0 {
        let meta_huffman_used = br.read_bits(1)? != 0;
        if meta_huffman_used { return None; }
    }

    let cache_size = if color_cache_used { 1usize << cache_bits } else { 0 };
    let group = read_huff_group(br, cache_size)?;
    let mut cache = vec![0u32; cache_size];
    let total = xsize * ysize;
    let mut pixels = vec![0u32; total];
    let mut pos = 0usize;
    while pos < total {
        let green_sym = decode_symbol(br, &group.green)?;
        if green_sym < 256 {
            let r = decode_symbol(br, &group.red)? as u32;
            let b = decode_symbol(br, &group.blue)? as u32;
            let a = decode_symbol(br, &group.alpha)? as u32;
            let g = green_sym as u32;
            let p = (a << 24) | (r << 16) | (g << 8) | b;
            pixels[pos] = p;
            if cache_size > 0 { cache[cache_hash(p, cache_bits)] = p; }
            pos += 1;
        } else if green_sym < 256 + 24 {
            let length_code = (green_sym - 256) as u32;
            let length = prefix_value(br, length_code)? as usize;
            let dist_sym = decode_symbol(br, &group.dist)? as u32;
            let dist_code = prefix_value(br, dist_sym)?;
            let dist = plane_code_to_distance(xsize, dist_code);
            if dist == 0 || dist > pos || pos + length > total { return None; }
            for i in 0..length {
                let p = pixels[pos + i - dist];
                pixels[pos + i] = p;
                if cache_size > 0 { cache[cache_hash(p, cache_bits)] = p; }
            }
            pos += length;
        } else {
            let idx = (green_sym - 256 - 24) as usize;
            let p = *cache.get(idx)?;
            pixels[pos] = p;
            pos += 1;
        }
    }
    Some(pixels)
}

fn subsample_size(size: usize, bits: u32) -> usize {
    (size + (1usize << bits) - 1) >> bits
}

// Addition octet-par-octet avec repli modulo 256 (VP8LAddPixels) : utilisee
// par le predictor (reconstruction residu -> pixel) et le depliage palette.
fn add_pixels(a: u32, b: u32) -> u32 {
    let mut r = 0u32;
    let mut shift = 0u32;
    while shift < 32 {
        let x = ((a >> shift) & 0xff) as u8;
        let y = ((b >> shift) & 0xff) as u8;
        r |= (x.wrapping_add(y) as u32) << shift;
        shift += 8;
    }
    r
}

fn ch(p: u32, shift: u32) -> i32 { ((p >> shift) & 0xff) as i32 }

fn avg2(a: u32, b: u32) -> u32 {
    let mut r = 0u32;
    let mut shift = 0u32;
    while shift < 32 {
        let x = (a >> shift) & 0xff;
        let y = (b >> shift) & 0xff;
        r |= ((x + y) / 2) << shift;
        shift += 8;
    }
    r
}

fn clamp255(v: i32) -> u32 { v.clamp(0, 255) as u32 }

fn per_channel(f: impl Fn(i32, i32) -> u32, a: u32, b: u32) -> u32 {
    let mut r = 0u32;
    let mut shift = 0u32;
    while shift < 32 {
        r |= f(ch(a, shift), ch(b, shift)) << shift;
        shift += 8;
    }
    r
}

fn per_channel3(f: impl Fn(i32, i32, i32) -> u32, a: u32, b: u32, c: u32) -> u32 {
    let mut r = 0u32;
    let mut shift = 0u32;
    while shift < 32 {
        r |= f(ch(a, shift), ch(b, shift), ch(c, shift)) << shift;
        shift += 8;
    }
    r
}

// Les 14 modes du predictor transform (cf. spec bitstream lossless WebP).
fn predict(mode: u8, l: u32, t: u32, tr: u32, tl: u32) -> u32 {
    match mode {
        0 => 0xff000000,
        1 => l,
        2 => t,
        3 => tr,
        4 => tl,
        5 => avg2(avg2(l, tr), t),
        6 => avg2(l, tl),
        7 => avg2(l, t),
        8 => avg2(tl, t),
        9 => avg2(t, tr),
        10 => avg2(avg2(l, tl), avg2(t, tr)),
        11 => {
            let p_a = ch(l, 24) + ch(t, 24) - ch(tl, 24);
            let p_r = ch(l, 16) + ch(t, 16) - ch(tl, 16);
            let p_g = ch(l, 8) + ch(t, 8) - ch(tl, 8);
            let p_b = ch(l, 0) + ch(t, 0) - ch(tl, 0);
            let p_l = (p_a - ch(l, 24)).abs() + (p_r - ch(l, 16)).abs() + (p_g - ch(l, 8)).abs() + (p_b - ch(l, 0)).abs();
            let p_t = (p_a - ch(t, 24)).abs() + (p_r - ch(t, 16)).abs() + (p_g - ch(t, 8)).abs() + (p_b - ch(t, 0)).abs();
            if p_l < p_t { l } else { t }
        }
        // ClampAddSubtractFull(L, T, TL) applique par canal.
        12 => per_channel3(|a, b, c| clamp255(a + b - c), l, t, tl),
        // ClampAddSubtractHalf(Average2(L, T), TL) applique par canal.
        13 => per_channel(|a, b| clamp255(a + (a - b) / 2), avg2(l, t), tl),
        _ => 0,
    }
}

// ── Application des transforms inverses ─────────────────────────────────
fn apply_predictor_inverse(pixels: &mut [u32], width: usize, height: usize, bits: u32, block_w: usize, block_image: &[u32]) {
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let l = if x > 0 { pixels[idx - 1] } else { 0 };
            let t = if y > 0 { pixels[idx - width] } else { 0 };
            let tl = if x > 0 && y > 0 { pixels[idx - width - 1] } else { 0 };
            // TR (top-right) : meme pour le DERNIER pixel d'une ligne, lit
            // idx-width+1 sans garde-fou -- dans le tampon plat de reference
            // (une image = un seul buffer contigu), cet index retombe
            // exactement sur le PREMIER pixel de la ligne courante (deja
            // reconstruit), pas sur une case hors bornes. Un repli "T" ici
            // (comme une premiere version le faisait) cause des erreurs qui
            // se propagent en cascade sur toute l'image -- verifie
            // empiriquement (voir commit).
            let tr = if y > 0 { pixels[idx - width + 1] } else { 0 };
            let mode = if x == 0 && y == 0 { 0 }
                else if y == 0 { 1 }
                else if x == 0 { 2 }
                else {
                    let bx = (x >> bits).min(block_w.saturating_sub(1));
                    let by = y >> bits;
                    let bidx = by * block_w + bx;
                    ((block_image.get(bidx).copied().unwrap_or(0) >> 8) & 0xff) as u8
                };
            let pred = predict(mode, l, t, tr, tl);
            pixels[idx] = add_pixels(pred, pixels[idx]);
        }
    }
}

fn color_transform_delta(t: i8, c: i8) -> i32 { ((t as i32) * (c as i32)) >> 5 }

fn apply_color_inverse(pixels: &mut [u32], width: usize, height: usize, bits: u32, block_w: usize, block_image: &[u32]) {
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let bx = (x >> bits).min(block_w.saturating_sub(1));
            let by = y >> bits;
            let bidx = by * block_w + bx;
            let c = block_image.get(bidx).copied().unwrap_or(0);
            // ColorCodeToMultipliers (libwebp) : g2r = octet0, g2b = octet1, r2b = octet2.
            let green_to_red = (c & 0xff) as u8 as i8;
            let green_to_blue = ((c >> 8) & 0xff) as u8 as i8;
            let red_to_blue = ((c >> 16) & 0xff) as u8 as i8;
            let p = pixels[idx];
            let green = ((p >> 8) & 0xff) as u8 as i8;
            let red = ((p >> 16) & 0xff) as i32;
            let blue = (p & 0xff) as i32;
            let mut new_red = (red + color_transform_delta(green_to_red, green)) & 0xff;
            let mut new_blue = (blue + color_transform_delta(green_to_blue, green)) & 0xff;
            new_blue = (new_blue + color_transform_delta(red_to_blue, new_red as u8 as i8)) & 0xff;
            new_red &= 0xff;
            pixels[idx] = (p & 0xff00_ff00) | ((new_red as u32) << 16) | (new_blue as u32);
        }
    }
}

fn apply_subtract_green_inverse(pixels: &mut [u32]) {
    for p in pixels.iter_mut() {
        let green = (*p >> 8) & 0xff;
        let red = ((*p >> 16) & 0xff).wrapping_add(green) & 0xff;
        let blue = (*p & 0xff).wrapping_add(green) & 0xff;
        *p = (*p & 0xff00_ff00) | (red << 16) | blue;
    }
}

// Table de couleurs stockee en delta (chaque entree = ecart octet-par-octet
// avec la precedente, cf. ExpandColorMap dans libwebp).
fn expand_palette(mut table: Vec<u32>) -> Vec<u32> {
    for i in 1..table.len() {
        table[i] = add_pixels(table[i], table[i - 1]);
    }
    table
}

fn apply_color_indexing_inverse(packed: &[u32], packed_w: usize, height: usize, width_bits: u32, table: &[u32], full_w: usize) -> Vec<u32> {
    let ppb = 1usize << width_bits;
    let bits_per_index = (8 / ppb.max(1)).max(1);
    let mask = if bits_per_index >= 8 { 0xffu32 } else { (1u32 << bits_per_index) - 1 };
    let mut out = vec![0u32; full_w * height];
    for y in 0..height {
        for x in 0..full_w {
            let px = x >> width_bits;
            let sub = x & (ppb - 1);
            let src_idx = y * packed_w + px;
            let green = (packed.get(src_idx).copied().unwrap_or(0) >> 8) & 0xff;
            let index = if width_bits == 0 { green } else { (green >> (sub * bits_per_index)) & mask };
            out[y * full_w + x] = table.get(index as usize).copied().unwrap_or(0xff00_0000);
        }
    }
    out
}

enum Transform {
    Predictor { bits: u32, block_w: usize, image: Vec<u32> },
    ColorT { bits: u32, block_w: usize, image: Vec<u32> },
    SubtractGreen,
    ColorIndexing { width_bits: u32, table: Vec<u32>, xsize_before: usize },
}

fn decode_vp8l(chunk: &[u8]) -> Option<Image> {
    if chunk.is_empty() || chunk[0] != 0x2f { return None; } // pas du VP8L
    let mut br = BitReader::new(&chunk[1..]);
    let width = br.read_bits(14)? as usize + 1;
    let height = br.read_bits(14)? as usize + 1;
    let _alpha_used = br.read_bits(1)?;
    let _version = br.read_bits(3)?;
    checked_area(width, height)?;

    let mut transforms: Vec<Transform> = Vec::new();
    let mut cur_w = width;
    while br.read_bits(1)? == 1 {
        if transforms.len() >= 8 { return None; }
        let ttype = br.read_bits(2)?;
        match ttype {
            0 | 1 => {
                let bits = br.read_bits(3)? + 2;
                let block_w = subsample_size(cur_w, bits);
                let block_h = subsample_size(height, bits);
                let image = decode_image_stream(&mut br, block_w, block_h, false)?;
                transforms.push(if ttype == 0 {
                    Transform::Predictor { bits, block_w, image }
                } else {
                    Transform::ColorT { bits, block_w, image }
                });
            }
            2 => transforms.push(Transform::SubtractGreen),
            3 => {
                let color_table_size = br.read_bits(8)? as usize + 1;
                let raw = decode_image_stream(&mut br, color_table_size, 1, false)?;
                let table = expand_palette(raw);
                let width_bits = if color_table_size <= 2 { 3 }
                    else if color_table_size <= 4 { 2 }
                    else if color_table_size <= 16 { 1 }
                    else { 0 };
                let xsize_before = cur_w;
                cur_w = subsample_size(cur_w, width_bits);
                transforms.push(Transform::ColorIndexing { width_bits, table, xsize_before });
            }
            _ => return None,
        }
    }

    let mut grid = decode_image_stream(&mut br, cur_w, height, true)?;
    let mut grid_w = cur_w;

    for t in transforms.iter().rev() {
        match t {
            Transform::Predictor { bits, block_w, image } => {
                apply_predictor_inverse(&mut grid, grid_w, height, *bits, *block_w, image);
            }
            Transform::ColorT { bits, block_w, image } => {
                apply_color_inverse(&mut grid, grid_w, height, *bits, *block_w, image);
            }
            Transform::SubtractGreen => apply_subtract_green_inverse(&mut grid),
            Transform::ColorIndexing { width_bits, table, xsize_before } => {
                grid = apply_color_indexing_inverse(&grid, grid_w, height, *width_bits, table, *xsize_before);
                grid_w = *xsize_before;
            }
        }
    }
    if grid_w != width || grid.len() != width * height { return None; }

    let mut pix = vec![0u32; width * height];
    for (i, &p) in grid.iter().enumerate() {
        let a = (p >> 24) & 0xff;
        let r = (p >> 16) & 0xff;
        let g = (p >> 8) & 0xff;
        let b = p & 0xff;
        pix[i] = composite_rgba(r, g, b, a);
    }
    Some(Image { w: width, h: height, pix })
}

pub fn decode(data: &[u8]) -> Option<Image> {
    if data.len() < 16 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" { return None; }
    let mut p = 12usize;
    while p + 8 <= data.len() {
        let tag = &data[p..p + 4];
        let sz = (data[p + 4] as usize) | ((data[p + 5] as usize) << 8)
            | ((data[p + 6] as usize) << 16) | ((data[p + 7] as usize) << 24);
        let body = p + 8;
        if body.checked_add(sz).map_or(true, |end| end > data.len()) { break; }
        if tag == b"VP8L" { return decode_vp8l(&data[body..body + sz]); }
        // VP8 (lossy) / VP8X (extended, alpha/anim) : non decodes -- repli propre.
        if tag == b"VP8 " || tag == b"VP8X" { return None; }
        p = body + sz + (sz & 1); // octet de bourrage si taille impaire
    }
    None
}
