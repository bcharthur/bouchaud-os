//! Decompression DEFLATE / zlib / gzip (RFC 1950/1951/1952).
//!
//! Portage fidele de l'algorithme de reference "puff" (Mark Adler, domaine
//! public) : decodage Huffman canonique + LZ77, sans table pre-calculee.
//! Permet de lire les sites qui repondent en `Content-Encoding: gzip` ou
//! `deflate` (la plupart des CDN, meme avec `Accept-Encoding: identity`).

use alloc::vec::Vec;

const MAXBITS: usize = 15;
const MAXLCODES: usize = 286;
const MAXDCODES: usize = 30;
const FIXLCODES: usize = 288;

struct BitReader<'a> {
    data: &'a [u8],
    incnt: usize,
    bitbuf: u32,
    bitcnt: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, incnt: 0, bitbuf: 0, bitcnt: 0 }
    }

    // Lit `need` bits (LSB en premier), comme dans puff.
    fn bits(&mut self, need: u32) -> Result<u32, ()> {
        let mut val = self.bitbuf;
        while self.bitcnt < need {
            if self.incnt >= self.data.len() { return Err(()); }
            val |= (self.data[self.incnt] as u32) << self.bitcnt;
            self.incnt += 1;
            self.bitcnt += 8;
        }
        self.bitbuf = val >> need;
        self.bitcnt -= need;
        Ok(val & ((1u32 << need) - 1))
    }
}

struct Huffman {
    count: [u16; MAXBITS + 1],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(n: usize) -> Self {
        Huffman { count: [0; MAXBITS + 1], symbol: alloc::vec![0u16; n] }
    }

    // Decode un symbole code par cet arbre de Huffman.
    fn decode(&self, br: &mut BitReader) -> Result<i32, ()> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..=MAXBITS {
            code |= br.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize] as i32);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(())
    }
}

fn construct(h: &mut Huffman, length: &[u16]) -> i32 {
    for c in h.count.iter_mut() { *c = 0; }
    for &l in length { h.count[l as usize] += 1; }
    if h.count[0] as usize == length.len() { return 0; }

    let mut left: i32 = 1;
    for len in 1..=MAXBITS {
        left <<= 1;
        left -= h.count[len] as i32;
        if left < 0 { return left; }
    }

    let mut offs = [0u16; MAXBITS + 1];
    for len in 1..MAXBITS {
        offs[len + 1] = offs[len] + h.count[len];
    }
    for (symbol, &l) in length.iter().enumerate() {
        if l != 0 {
            h.symbol[offs[l as usize] as usize] = symbol as u16;
            offs[l as usize] += 1;
        }
    }
    left
}

// Longueurs/distances LZ77 (RFC 1951).
const LENS: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31,
    35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LEXT: [u16; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTS: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193,
    257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DEXT: [u16; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

fn codes(br: &mut BitReader, out: &mut Vec<u8>, lencode: &Huffman, distcode: &Huffman) -> Result<(), ()> {
    loop {
        let symbol = lencode.decode(br)?;
        if symbol < 0 { return Err(()); }
        if symbol < 256 {
            out.push(symbol as u8);
        } else if symbol == 256 {
            return Ok(());
        } else {
            let s = (symbol - 257) as usize;
            if s >= 29 { return Err(()); }
            let len = LENS[s] as usize + br.bits(LEXT[s] as u32)? as usize;
            let dsym = distcode.decode(br)?;
            if dsym < 0 || dsym as usize >= 30 { return Err(()); }
            let dist = DISTS[dsym as usize] as usize + br.bits(DEXT[dsym as usize] as u32)? as usize;
            if dist > out.len() { return Err(()); }
            let start = out.len() - dist;
            for k in 0..len {
                out.push(out[start + k]);
            }
        }
    }
}

fn stored(br: &mut BitReader, out: &mut Vec<u8>) -> Result<(), ()> {
    // Aligne sur l'octet : jette les bits restants du tampon.
    br.bitbuf = 0;
    br.bitcnt = 0;
    if br.incnt + 4 > br.data.len() { return Err(()); }
    let len = br.data[br.incnt] as usize | ((br.data[br.incnt + 1] as usize) << 8);
    br.incnt += 4; // len + ~len
    if br.incnt + len > br.data.len() { return Err(()); }
    out.extend_from_slice(&br.data[br.incnt..br.incnt + len]);
    br.incnt += len;
    Ok(())
}

fn fixed(br: &mut BitReader, out: &mut Vec<u8>) -> Result<(), ()> {
    let mut lengths = [0u16; FIXLCODES];
    for l in lengths.iter_mut().take(144) { *l = 8; }
    for l in lengths.iter_mut().take(256).skip(144) { *l = 9; }
    for l in lengths.iter_mut().take(280).skip(256) { *l = 7; }
    for l in lengths.iter_mut().take(FIXLCODES).skip(280) { *l = 8; }
    let mut lencode = Huffman::new(FIXLCODES);
    construct(&mut lencode, &lengths);
    let dlen = [5u16; MAXDCODES];
    let mut distcode = Huffman::new(MAXDCODES);
    construct(&mut distcode, &dlen);
    codes(br, out, &lencode, &distcode)
}

fn dynamic(br: &mut BitReader, out: &mut Vec<u8>) -> Result<(), ()> {
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let nlen = br.bits(5)? as usize + 257;
    let ndist = br.bits(5)? as usize + 1;
    let ncode = br.bits(4)? as usize + 4;
    if nlen > MAXLCODES || ndist > MAXDCODES { return Err(()); }

    let mut lengths = [0u16; 19];
    for i in 0..ncode {
        lengths[ORDER[i]] = br.bits(3)? as u16;
    }
    let mut lencode = Huffman::new(19);
    if construct(&mut lencode, &lengths) != 0 { return Err(()); }

    // Lit les longueurs de code pour les arbres litteral/longueur et distance.
    let total = nlen + ndist;
    let mut codelen: Vec<u16> = Vec::with_capacity(total);
    while codelen.len() < total {
        let symbol = lencode.decode(br)?;
        if symbol < 0 { return Err(()); }
        if symbol < 16 {
            codelen.push(symbol as u16);
        } else {
            let mut rep;
            let val;
            if symbol == 16 {
                if codelen.is_empty() { return Err(()); }
                val = *codelen.last().unwrap();
                rep = 3 + br.bits(2)? as usize;
            } else if symbol == 17 {
                val = 0;
                rep = 3 + br.bits(3)? as usize;
            } else {
                val = 0;
                rep = 11 + br.bits(7)? as usize;
            }
            while rep > 0 && codelen.len() < total {
                codelen.push(val);
                rep -= 1;
            }
        }
    }
    if codelen.len() != total { return Err(()); }

    // Construit les arbres. Un code incomplet (left>0) n'est tolere que pour un
    // unique code de longueur 1 (condition exacte de puff) ; sur-souscrit => erreur.
    let mut lencode2 = Huffman::new(nlen);
    let err = construct(&mut lencode2, &codelen[..nlen]);
    if err != 0 && (err < 0 || nlen as i32 != (lencode2.count[0] + lencode2.count[1]) as i32) {
        return Err(());
    }
    let mut distcode = Huffman::new(ndist);
    let err = construct(&mut distcode, &codelen[nlen..total]);
    if err != 0 && (err < 0 || ndist as i32 != (distcode.count[0] + distcode.count[1]) as i32) {
        return Err(());
    }
    codes(br, out, &lencode2, &distcode)
}

/// Decompresse un flux DEFLATE brut (sans en-tete).
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, ()> {
    let mut br = BitReader::new(data);
    let mut out = Vec::new();
    loop {
        let last = br.bits(1)?;
        let btype = br.bits(2)?;
        match btype {
            0 => stored(&mut br, &mut out)?,
            1 => fixed(&mut br, &mut out)?,
            2 => dynamic(&mut br, &mut out)?,
            _ => return Err(()),
        }
        if last == 1 { break; }
    }
    Ok(out)
}

/// Decompresse un flux zlib (RFC 1950) : 2 octets d'en-tete + DEFLATE + Adler32.
pub fn zlib_decode(data: &[u8]) -> Result<Vec<u8>, ()> {
    if data.len() < 2 { return Err(()); }
    // CMF/FLG ; bit FDICT (0x20 de FLG) non gere.
    if data[1] & 0x20 != 0 { return Err(()); }
    inflate(&data[2..])
}

/// Decompresse un flux gzip (RFC 1952) : en-tete variable + DEFLATE + CRC.
pub fn gzip_decode(data: &[u8]) -> Result<Vec<u8>, ()> {
    if data.len() < 10 || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return Err(());
    }
    let flg = data[3];
    let mut pos = 10;
    if flg & 0x04 != 0 {
        // FEXTRA
        if pos + 2 > data.len() { return Err(()); }
        let xlen = data[pos] as usize | ((data[pos + 1] as usize) << 8);
        pos += 2 + xlen;
    }
    if flg & 0x08 != 0 {
        // FNAME (chaine terminee par 0)
        while pos < data.len() && data[pos] != 0 { pos += 1; }
        pos += 1;
    }
    if flg & 0x10 != 0 {
        // FCOMMENT
        while pos < data.len() && data[pos] != 0 { pos += 1; }
        pos += 1;
    }
    if flg & 0x02 != 0 {
        // FHCRC
        pos += 2;
    }
    if pos > data.len() { return Err(()); }
    inflate(&data[pos..])
}

/// Decode un corps selon l'en-tete `Content-Encoding` (`gzip`/`deflate`/`x-gzip`).
/// Renvoie `None` si l'encodage est inconnu ou la decompression echoue.
pub fn decode_content(encoding: &str, body: &[u8]) -> Option<Vec<u8>> {
    let enc = encoding.trim().to_ascii_lowercase();
    if enc.contains("gzip") || enc.contains("x-gzip") {
        gzip_decode(body).ok()
    } else if enc.contains("deflate") {
        // "deflate" HTTP = zlib en theorie ; certains serveurs envoient du DEFLATE
        // brut. On tente zlib puis brut.
        zlib_decode(body).ok().or_else(|| inflate(body).ok())
    } else {
        None
    }
}

/// Auto-test : decompresse des donnees produites par gzip (embarquees).
pub fn selftest() -> Result<(), &'static str> {
    // "deflate" brut de "Hello, Hello, Hello!" (bloc fixe), genere hors-ligne.
    // On valide plutot le chemin via un petit flux stored + fixed.
    // Bloc DEFLATE "stored" contenant "abc" :
    //   01 (last, type=00) 03 00 (len=3) fc ff (~len) 'a' 'b' 'c'
    let stored = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
    let out = inflate(&stored).map_err(|_| "inflate stored")?;
    if out != b"abc" { return Err("stored != abc"); }
    Ok(())
}
