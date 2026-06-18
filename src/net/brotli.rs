//! Brotli (RFC 7932) : decodeur complet from-scratch.
//!
//! Porte un decodeur valide hors-ligne contre la reference (python-brotli) sur
//! >1700 vecteurs (toutes qualites 0..11, textes/binaires/HTML/unicode, et le
//! dictionnaire lui-meme). Gere : en-tete de flux, meta-blocs (compresses,
//! non compresses, metadata), codes de prefixe simples/complexes, table
//! statique + dynamique, context maps (RLE + IMTF), commandes insert&copy,
//! distances (cache + NPOSTFIX/NDIRECT) et le dictionnaire statique de 122 Ko
//! avec ses 121 transformations (Annexe A/B).

use alloc::vec::Vec;
use alloc::vec;

include!("brotli_tables.rs");

/// Dictionnaire statique Brotli (RFC 7932 Annexe A), 122 784 octets.
static DICT: &[u8] = include_bytes!("brotli_dict.bin");


struct Br<'a> { d: &'a [u8], pos: usize, buf: u64, cnt: u32 }
impl<'a> Br<'a> {
    fn new(d: &'a [u8]) -> Br<'a> { Br { d, pos: 0, buf: 0, cnt: 0 } }
    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 { return 0; }
        while self.cnt < n {
            let b = if self.pos < self.d.len() { self.d[self.pos] } else { 0 };
            self.pos += 1; self.buf |= (b as u64) << self.cnt; self.cnt += 8;
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n; self.cnt -= n; v
    }
    fn align(&mut self) {
        let whole = (self.cnt / 8) as usize;
        self.pos -= whole; self.buf = 0; self.cnt = 0;
    }
}

fn rev_bits(mut v: u32, n: u32) -> u32 {
    let mut r = 0u32;
    for _ in 0..n { r = (r << 1) | (v & 1); v >>= 1; }
    r
}

// Code canonique brotli, lu LSB-first. entries: (symbole, longueur) dans
// l'ordre d'assignation des codes a longueur egale.
struct Huff { entries: Vec<(u8, u32, u16)>, single: i32 } // (len, rev_code, symbol)
impl Huff {
    fn build(entries: &[(u16, u8)]) -> Huff {
        let mut nonzero = 0; let mut last = 0i32;
        for &(s, l) in entries { if l > 0 { nonzero += 1; last = s as i32; } }
        let mut e: Vec<(u8, u32, u16)> = Vec::new();
        let mut code: u32 = 0;
        for len in 1..16u32 {
            // Symboles de meme longueur tries par valeur (canonique). Les codes
            // simples sont lus dans un ordre arbitraire ; ce tri est requis.
            let mut group: Vec<u16> = entries.iter()
                .filter(|&&(_, l)| l as u32 == len).map(|&(s, _)| s).collect();
            group.sort_unstable();
            for s in group {
                e.push((len as u8, rev_bits(code, len), s));
                code += 1;
            }
            code <<= 1;
        }
        let single = if entries.len() == 1 { entries[0].0 as i32 } else if nonzero == 1 { last } else { -1 };
        Huff { entries: e, single }
    }
    fn decode(&self, br: &mut Br) -> Option<u16> {
        if self.single >= 0 { return Some(self.single as u16); }
        let mut acc: u32 = 0;
        for nbits in 1..16u32 {
            acc |= br.bits(1) << (nbits - 1);
            for &(l, c, s) in &self.entries {
                if l as u32 == nbits && c == acc { return Some(s); }
            }
        }
        None
    }
}

const CLCL_ORDER: [usize; 18] = [1,2,3,4,0,5,17,6,16,7,8,9,10,11,12,13,14,15];
const CLC_PREFIX_LEN: [u8; 16] = [2,2,2,3,2,2,2,4,2,2,2,3,2,2,2,4];
const CLC_PREFIX_VAL: [u8; 16] = [0,4,3,2,0,4,3,1,0,4,3,2,0,4,3,5];
const BLOCK_LENGTH: [(u8, u32); 26] = [
    (2,1),(2,5),(2,9),(2,13),(3,17),(3,25),(3,33),(3,41),(4,49),(4,65),(4,81),(4,97),
    (5,113),(5,145),(5,177),(5,209),(6,241),(6,305),(7,369),(8,497),(9,753),(10,1265),
    (11,2289),(12,4337),(13,8433),(24,16625)];

fn varlen_u8(br: &mut Br) -> u32 {
    if br.bits(1) == 0 { return 0; }
    let n = br.bits(3);
    if n == 0 { return 1; }
    (1 << n) + br.bits(n)
}

// Lit un code de Huffman (simple ou complexe). alphabet_size symboles.
fn read_huffman(br: &mut Br, alphabet_size: u32) -> Option<Huff> {
    let code_type = br.bits(2);
    if code_type == 1 {
        // Simple : 1..4 symboles.
        let nsym = br.bits(2) + 1;
        let mut maxbits = 0u32; { let mut x = alphabet_size - 1; while x != 0 { x >>= 1; maxbits += 1; } }
        let mut syms = Vec::new();
        for _ in 0..nsym { let v = br.bits(maxbits); if v >= alphabet_size { return None; } syms.push(v as u16); }
        let entries: Vec<(u16,u8)> = match nsym {
            1 => vec![(syms[0],0)],
            2 => vec![(syms[0],1),(syms[1],1)],
            3 => vec![(syms[0],1),(syms[1],2),(syms[2],2)],
            _ => { // 4 symboles : 1 bit de selection
                if br.bits(1) == 1 { vec![(syms[0],1),(syms[1],2),(syms[2],3),(syms[3],3)] }
                else { vec![(syms[0],2),(syms[1],2),(syms[2],2),(syms[3],2)] }
            }
        };
        return Some(Huff::build(&entries));
    }
    // Complexe : code_type = nombre de longueurs de code a sauter.
    let skip = code_type as usize;
    let mut clcl = [0u8; 18];
    let mut space = 32i32; let mut num_codes = 0;
    let mut i = skip;
    while i < 18 {
        let peek = (br.buf as u32) & 0xF; // peek 4 bits sans consommer si dispo
        // S'assure d'avoir >=4 bits tampon
        let ix = { while br.cnt < 4 { let b = if br.pos<br.d.len(){br.d[br.pos]}else{0}; br.pos+=1; br.buf|=(b as u64)<<br.cnt; br.cnt+=8;} (br.buf as u32)&0xF };
        let _ = peek;
        let l = CLC_PREFIX_LEN[ix as usize] as u32;
        let v = CLC_PREFIX_VAL[ix as usize];
        // consomme l bits
        br.buf >>= l; br.cnt -= l;
        clcl[CLCL_ORDER[i]] = v;
        if v != 0 {
            space -= 32 >> v;
            num_codes += 1;
            if space <= 0 { break; }
        }
        i += 1;
    }
    if num_codes != 1 && space != 0 { return None; }
    // Construit le code des longueurs (alphabet 18).
    let clcl_entries: Vec<(u16,u8)> = (0..18u16).map(|s| (s, clcl[s as usize])).collect();
    let clhuff = Huff::build(&clcl_entries);
    // Lit les longueurs de code de l'alphabet principal.
    let mut lengths = vec![0u8; alphabet_size as usize];
    let mut symbol = 0usize; let mut space2 = 32768i32;
    let mut prev_len = 8u8; let mut repeat = 0u32; let mut repeat_code_len = 0u8;
    while symbol < alphabet_size as usize && space2 > 0 {
        let sym = clhuff.decode(br)?;
        if sym < 16 {
            repeat = 0;
            lengths[symbol] = sym as u8;
            if sym != 0 { prev_len = sym as u8; space2 -= 32768 >> sym; }
            symbol += 1;
        } else {
            let extra_bits = if sym == 16 { 2 } else { 3 };
            let repeat_delta = br.bits(extra_bits);
            let new_len = if sym == 16 { prev_len } else { 0 };
            if repeat_code_len != new_len { repeat = 0; repeat_code_len = new_len; }
            let old_repeat = repeat;
            if repeat > 0 { repeat -= 2; repeat <<= extra_bits; }
            repeat += repeat_delta + 3;
            let delta = repeat - old_repeat;
            if symbol + delta as usize > alphabet_size as usize { return None; }
            if repeat_code_len != 0 {
                for _ in 0..delta { lengths[symbol] = repeat_code_len; symbol += 1; }
                space2 -= (delta as i32) << (15 - repeat_code_len as i32);
            } else {
                symbol += delta as usize;
            }
        }
    }
    let entries: Vec<(u16,u8)> = (0..alphabet_size as u16).map(|s| (s, lengths[s as usize])).collect();
    Some(Huff::build(&entries))
}

fn read_block_length(br: &mut Br, tree: &Huff) -> u32 {
    let code = tree.decode(br).unwrap_or(0) as usize;
    let (nbits, offset) = BLOCK_LENGTH[code.min(25)];
    offset + br.bits(nbits as u32)
}

fn decode_context_map(br: &mut Br, size: usize) -> Option<(u32, Vec<u8>)> {
    let num_htrees = varlen_u8(br) + 1;
    let mut map = vec![0u8; size];
    if num_htrees <= 1 { return Some((num_htrees, map)); }
    let use_rle = br.bits(1) != 0;
    let max_run = if use_rle { br.bits(4) + 1 } else { 0 };
    let tree = read_huffman(br, num_htrees + max_run)?;
    let mut i = 0usize;
    while i < size {
        let code = tree.decode(br)? as u32;
        if code == 0 { map[i] = 0; i += 1; }
        else if code <= max_run {
            let reps = (1u32 << code) + br.bits(code);
            for _ in 0..reps { if i >= size { return None; } map[i] = 0; i += 1; }
        } else { map[i] = (code - max_run) as u8; i += 1; }
    }
    if br.bits(1) != 0 { imtf(&mut map); }
    Some((num_htrees, map))
}

fn imtf(v: &mut [u8]) {
    let mut mtf = [0u8; 256];
    for i in 0..256 { mtf[i] = i as u8; }
    for x in v.iter_mut() {
        let idx = *x as usize;
        let val = mtf[idx];
        *x = val;
        let mut j = idx;
        while j >= 1 { mtf[j] = mtf[j-1]; j -= 1; }
        mtf[0] = val;
    }
}

fn to_upper(p: &mut [u8]) -> usize {
    if p[0] < 0xC0 { if p[0] >= b'a' && p[0] <= b'z' { p[0] ^= 32; } return 1; }
    if p[0] < 0xE0 { if p.len() > 1 { p[1] ^= 32; } return 2; }
    if p.len() > 2 { p[2] ^= 5; } 3
}

// Applique une transformation du dictionnaire ; renvoie le mot transforme.
fn transform_word(word: &[u8], tidx: usize) -> Vec<u8> {
    let (pid, ttype, sid) = TRANSFORMS[tidx];
    let mut out = Vec::new();
    // prefixe
    let po = PREFIX_SUFFIX_MAP[pid as usize] as usize;
    let plen = PREFIX_SUFFIX[po] as usize;
    out.extend_from_slice(&PREFIX_SUFFIX[po+1..po+1+plen]);
    // corps transforme
    let t = ttype;
    let mut w = word.to_vec();
    let mut len = w.len();
    if t >= 1 && t <= 9 { // OMIT_LAST_n
        if len >= t as usize { len -= t as usize; } else { len = 0; }
        w.truncate(len);
    } else if t >= 12 && t <= 20 { // OMIT_FIRST_n
        let skip = (t - 11) as usize;
        if len >= skip { w.drain(0..skip); } else { w.clear(); }
    }
    let body_start = out.len();
    out.extend_from_slice(&w);
    let blen = out.len() - body_start;
    if t == 10 { // UPPERCASE_FIRST
        to_upper(&mut out[body_start..]);
    } else if t == 11 { // UPPERCASE_ALL
        let mut k = body_start;
        while k < body_start + blen { let step = to_upper(&mut out[k..body_start+blen]); k += step; }
    }
    // suffixe
    let so = PREFIX_SUFFIX_MAP[sid as usize] as usize;
    let slen = PREFIX_SUFFIX[so] as usize;
    out.extend_from_slice(&PREFIX_SUFFIX[so+1..so+1+slen]);
    out
}

pub fn decompress(data: &[u8], dict: &[u8]) -> Option<Vec<u8>> {
    let mut br = Br::new(data);
    // WBITS
    let wbits = { if br.bits(1) == 0 { 16 } else { let n = br.bits(3); if n != 0 { 17 + n } else { let m = br.bits(3); if m != 0 { 8 + m } else { 17 } } } };
    let max_backward = (1u32 << wbits) - 16;
    let mut out: Vec<u8> = Vec::new();

    loop {
        let islast = br.bits(1);
        if islast == 1 && br.bits(1) == 1 { break; }
        let nibbles_code = br.bits(2);
        if nibbles_code == 3 {
            if br.bits(1) != 0 { return None; }
            let skip_bytes = br.bits(2);
            let mut skip_len = 0u32;
            for i in 0..skip_bytes { skip_len |= br.bits(8) << (8*i); }
            if skip_bytes > 0 { skip_len += 1; }
            br.align();
            br.pos += skip_len as usize;
            if islast == 1 { break; }
            continue;
        }
        let mnibbles = nibbles_code + 4;
        let mut mlen = 0u32;
        for i in 0..mnibbles { mlen |= br.bits(4) << (4*i); }
        let mlen = (mlen + 1) as usize;
        if islast == 0 {
            if br.bits(1) == 1 { // ISUNCOMPRESSED
                br.align();
                for _ in 0..mlen { out.push(br.bits(8) as u8); }
                continue;
            }
        }
        // --- meta-bloc compresse ---
        let mut nbltypes = [0u32; 3];
        let mut type_tree: [Option<Huff>; 3] = [None, None, None];
        let mut len_tree: [Option<Huff>; 3] = [None, None, None];
        let mut blen = [0u32; 3];
        let mut btype = [0u32; 3];
        let mut btype_prev = [0u32; 3];
        for cat in 0..3 {
            nbltypes[cat] = varlen_u8(&mut br) + 1;
            if nbltypes[cat] >= 2 {
                type_tree[cat] = Some(read_huffman(&mut br, nbltypes[cat] + 2)?);
                len_tree[cat] = Some(read_huffman(&mut br, 26)?);
                blen[cat] = read_block_length(&mut br, len_tree[cat].as_ref().unwrap());
            } else { blen[cat] = 1 << 28; }
            btype[cat] = 0; btype_prev[cat] = 1;
        }
        let npostfix = br.bits(2);
        let ndirect = br.bits(4) << npostfix;
        let postfix_mask = (1u32 << npostfix) - 1;
        let num_dist_codes = 16 + ndirect + (48 << npostfix);
        let mut cmode = vec![0u8; nbltypes[0] as usize];
        for i in 0..nbltypes[0] as usize { cmode[i] = br.bits(2) as u8; }
        let (num_htrees_l, cmap_l) = decode_context_map(&mut br, (nbltypes[0] as usize) << 6)?;
        let (num_htrees_d, cmap_d) = decode_context_map(&mut br, (nbltypes[2] as usize) << 2)?;
        let mut htrees_l = Vec::new();
        for _ in 0..num_htrees_l { htrees_l.push(read_huffman(&mut br, 256)?); }
        let mut htrees_i = Vec::new();
        for _ in 0..nbltypes[1] { htrees_i.push(read_huffman(&mut br, 704)?); }
        let mut htrees_d = Vec::new();
        for _ in 0..num_htrees_d { htrees_d.push(read_huffman(&mut br, num_dist_codes)?); }

        let mut dist_rb = [16i32, 15, 11, 4];
        let mut dist_rb_idx = 0i32;
        let mlen_end = out.len() + mlen;


        while out.len() < mlen_end {
            // commande
            if blen[1] == 0 {
                let (tt, lt) = (type_tree[1].as_ref().unwrap(), len_tree[1].as_ref().unwrap());
                switch_block(&mut br, 1, tt, &mut btype, &mut btype_prev, nbltypes[1]);
                blen[1] = read_block_length(&mut br, lt);
            }
            let cmd = htrees_i[btype[1] as usize].decode(&mut br)? as usize;
            blen[1] -= 1;
            let (ie, ce, dc, ctx, io, co) = CMD_LUT[cmd];
            let insert_len = io as u32 + br.bits(ie as u32);
            let copy_len = co as u32 + br.bits(ce as u32);
            // insertions (litteraux)
            for _ in 0..insert_len {
                if blen[0] == 0 {
                    let (tt, lt) = (type_tree[0].as_ref().unwrap(), len_tree[0].as_ref().unwrap());
                    switch_block(&mut br, 0, tt, &mut btype, &mut btype_prev, nbltypes[0]);
                    blen[0] = read_block_length(&mut br, lt);
                }
                let p1 = if out.len() >= 1 { out[out.len()-1] } else { 0 };
                let p2 = if out.len() >= 2 { out[out.len()-2] } else { 0 };
                let mode = cmode[btype[0] as usize] as usize;
                let cid = (CONTEXT_LOOKUP[mode*512 + p1 as usize] | CONTEXT_LOOKUP[mode*512 + 256 + p2 as usize]) as usize;
                let htree = cmap_l[(btype[0] as usize) * 64 + cid] as usize;
                let lit = htrees_l[htree].decode(&mut br)? as u8;
                out.push(lit);
                blen[0] -= 1;
            }
            if out.len() >= mlen_end { break; }
            // distance : `ctx` (contexte cmdLut, base sur la longueur de copie)
            // selectionne l'arbre ; `roll_context` (0/1, reinitialise) sert a la
            // compensation du ring-buffer en cas de reference dictionnaire.
            let mut roll_context = 0i32;
            let distance: i32;
            if dc == 0 {
                // distance implicite (derniere distance)
                roll_context = 1;
                dist_rb_idx -= 1;
                distance = dist_rb[(dist_rb_idx & 3) as usize];
            } else {
                if blen[2] == 0 {
                    let (tt, lt) = (type_tree[2].as_ref().unwrap(), len_tree[2].as_ref().unwrap());
                    switch_block(&mut br, 2, tt, &mut btype, &mut btype_prev, nbltypes[2]);
                    blen[2] = read_block_length(&mut br, lt);
                }
                let htree = cmap_d[(btype[2] as usize) * 4 + ctx as usize] as usize;
                let mut dcode = htrees_d[htree].decode(&mut br)? as i32;
                blen[2] -= 1;
                if (dcode & !0xF) == 0 {
                    // code court via ring buffer
                    let (d, dctx) = take_dist_ring(dcode, &dist_rb, &mut dist_rb_idx);
                    distance = d; roll_context = dctx;
                } else {
                    let num_direct = 16i32 + ndirect as i32;
                    let distval = dcode - num_direct;
                    if distval >= 0 {
                        let postfix = distval & postfix_mask as i32;
                        let dv = distval >> npostfix;
                        let nbits = (dv >> 1) + 1;
                        let bits = br.bits(nbits as u32) as i32;
                        let offset = ((2 + (dv & 1)) << nbits) - 4;
                        dcode = num_direct + ((offset + bits) << npostfix) + postfix;
                    }
                    distance = dcode - 16 + 1;
                }
            }
            let pos = out.len() as i32;
            let max_distance = if pos < max_backward as i32 { pos } else { max_backward as i32 };
            if distance > max_distance {
                // reference au dictionnaire statique
                let i = copy_len as usize;
                if i < 4 || i > 24 { return None; }
                let address = (distance - max_distance - 1) as usize;
                let offset0 = DICT_OFFSETS[i] as usize;
                let shift = DICT_SIZE_BITS[i] as u32;
                let mask = (1usize << shift) - 1;
                let word_idx = address & mask;
                let tidx = address >> shift;
                dist_rb_idx += roll_context;
                if tidx >= 121 { return None; }
                let woff = offset0 + word_idx * i;
                if woff + i > dict.len() { return None; }
                let word = &dict[woff..woff+i];
                let tw = transform_word(word, tidx);
                out.extend_from_slice(&tw);
            } else {
                // copie LZ77
                let start = (pos - distance) as usize;
                for k in 0..copy_len as usize { let b = out[start + k]; out.push(b); }
                dist_rb[(dist_rb_idx & 3) as usize] = distance;
                dist_rb_idx += 1;
            }
        }
        if islast == 1 { break; }
    }
    Some(out)
}


fn switch_block(br: &mut Br, cat: usize, tt: &Huff, btype: &mut [u32;3], btype_prev: &mut [u32;3], max: u32) {
    let sym = tt.decode(br).unwrap_or(0) as u32;
    let mut nt = if sym == 0 { btype_prev[cat] }
                 else if sym == 1 { btype[cat] + 1 }
                 else { sym - 2 };
    if nt >= max { nt -= max; }
    btype_prev[cat] = btype[cat];
    btype[cat] = nt;
}

fn take_dist_ring(dcode: i32, dist_rb: &[i32;4], idx: &mut i32) -> (i32, i32) {
    if dcode == 0 {
        *idx -= 1;
        ( dist_rb[(*idx & 3) as usize], 1 )
    } else {
        let dc2 = (dcode << 1) as u32;
        const IDX_OFFSET: u32 = 0xAAAFFF1B;
        const VAL_OFFSET: u32 = 0xFA5FA500;
        let v = (*idx + ((IDX_OFFSET >> dc2) & 3) as i32) & 3;
        let mut d = dist_rb[v as usize];
        let valoff = ((VAL_OFFSET >> dc2) & 3) as i32;
        if (dc2 & 0x3) != 0 { d += valoff; } else { d -= valoff; if d <= 0 { d = 0x7FFFFFFF; } }
        (d, 0)
    }
}

const SELFTEST_COMP: [u8; 87] = [27,97,0,224,141,132,220,68,235,39,30,136,104,155,74,221,150,234,66,50,104,104,23,172,130,161,233,47,40,8,135,227,57,76,228,144,226,118,225,3,7,27,112,192,94,196,145,12,38,247,75,213,138,189,145,58,159,233,130,126,0,137,85,182,27,200,244,191,139,209,42,92,201,224,198,128,100,209,2,221,69,68,26,131,246,141,54];
const SELFTEST_MSG: &str = "Bouchaud OS - decodeur Brotli RFC 7932 - the world wide web http://www.example.com/ </body></html>";


/// Decode un flux Brotli (`Content-Encoding: br`) avec le dictionnaire embarque.
pub fn decode(data: &[u8]) -> Option<Vec<u8>> {
    decompress(data, DICT)
}

/// Auto-test : decompresse un flux brotli q=11 et compare au texte attendu.
pub fn selftest() -> Result<(), &'static str> {
    let comp: &[u8] = &SELFTEST_COMP;
    let expected = SELFTEST_MSG;
    match decode(comp) {
        Some(out) => {
            if out.as_slice() == expected.as_bytes() { Ok(()) } else { Err("brotli: sortie inattendue") }
        }
        None => Err("brotli: echec de decodage"),
    }
}
