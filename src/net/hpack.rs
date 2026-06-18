//! HPACK (RFC 7541) : compression d'en-tetes HTTP/2.
//!
//! Cote client on n'a besoin que :
//!   - d'un *encodeur* minimal pour la requete (litteraux sans Huffman, toujours
//!     valides) ;
//!   - d'un *decodeur* complet pour la reponse du serveur : table statique,
//!     table dynamique avec eviction, entiers a prefixe et chaines Huffman
//!     (les serveurs encodent presque toujours les valeurs en Huffman).
//!
//! Table de Huffman : RFC 7541 Annexe B. Table statique : Annexe A.

use alloc::string::String;
use alloc::vec::Vec;

/// Table statique HPACK (RFC 7541 Annexe A), index 1..=61.
const STATIC_TABLE: &[(&str, &str)] = &[
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

// Codes de Huffman HPACK (RFC 7541 Annexe B), symboles 0..=255.
const HUFF_CODE: [u32; 256] = [
    0x1ff8, 0x7fffd8, 0xfffffe2, 0xfffffe3, 0xfffffe4, 0xfffffe5, 0xfffffe6, 0xfffffe7,
    0xfffffe8, 0xffffea, 0x3ffffffc, 0xfffffe9, 0xfffffea, 0x3ffffffd, 0xfffffeb, 0xfffffec,
    0xfffffed, 0xfffffee, 0xfffffef, 0xffffff0, 0xffffff1, 0xffffff2, 0x3ffffffe, 0xffffff3,
    0xffffff4, 0xffffff5, 0xffffff6, 0xffffff7, 0xffffff8, 0xffffff9, 0xffffffa, 0xffffffb,
    0x14, 0x3f8, 0x3f9, 0xffa, 0x1ff9, 0x15, 0xf8, 0x7fa,
    0x3fa, 0x3fb, 0xf9, 0x7fb, 0xfa, 0x16, 0x17, 0x18,
    0x0, 0x1, 0x2, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
    0x1e, 0x1f, 0x5c, 0xfb, 0x7ffc, 0x20, 0xffb, 0x3fc,
    0x1ffa, 0x21, 0x5d, 0x5e, 0x5f, 0x60, 0x61, 0x62,
    0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a,
    0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72,
    0xfc, 0x73, 0xfd, 0x1ffb, 0x7fff0, 0x1ffc, 0x3ffc, 0x22,
    0x7ffd, 0x3, 0x23, 0x4, 0x24, 0x5, 0x25, 0x26,
    0x27, 0x6, 0x74, 0x75, 0x28, 0x29, 0x2a, 0x7,
    0x2b, 0x76, 0x2c, 0x8, 0x9, 0x2d, 0x77, 0x78,
    0x79, 0x7a, 0x7b, 0x7ffe, 0x7fc, 0x3ffd, 0x1ffd, 0xffffffc,
    0xfffe6, 0x3fffd2, 0xfffe7, 0xfffe8, 0x3fffd3, 0x3fffd4, 0x3fffd5, 0x7fffd9,
    0x3fffd6, 0x7fffda, 0x7fffdb, 0x7fffdc, 0x7fffdd, 0x7fffde, 0xffffeb, 0x7fffdf,
    0xffffec, 0xffffed, 0x3fffd7, 0x7fffe0, 0xffffee, 0x7fffe1, 0x7fffe2, 0x7fffe3,
    0x7fffe4, 0x1fffdc, 0x3fffd8, 0x7fffe5, 0x3fffd9, 0x7fffe6, 0x7fffe7, 0xffffef,
    0x3fffda, 0x1fffdd, 0xfffe9, 0x3fffdb, 0x3fffdc, 0x7fffe8, 0x7fffe9, 0x1fffde,
    0x7fffea, 0x3fffdd, 0x3fffde, 0xfffff0, 0x1fffdf, 0x3fffdf, 0x7fffeb, 0x7fffec,
    0x1fffe0, 0x1fffe1, 0x3fffe0, 0x1fffe2, 0x7fffed, 0x3fffe1, 0x7fffee, 0x7fffef,
    0xfffea, 0x3fffe2, 0x3fffe3, 0x3fffe4, 0x7ffff0, 0x3fffe5, 0x3fffe6, 0x7ffff1,
    0x3ffffe0, 0x3ffffe1, 0xfffeb, 0x7fff1, 0x3fffe7, 0x7ffff2, 0x3fffe8, 0x1ffffec,
    0x3ffffe2, 0x3ffffe3, 0x3ffffe4, 0x7ffffde, 0x7ffffdf, 0x3ffffe5, 0xfffff1, 0x1ffffed,
    0x7fff2, 0x1fffe3, 0x3ffffe6, 0x7ffffe0, 0x7ffffe1, 0x3ffffe7, 0x7ffffe2, 0xfffff2,
    0x1fffe4, 0x1fffe5, 0x3ffffe8, 0x3ffffe9, 0xffffffd, 0x7ffffe3, 0x7ffffe4, 0x7ffffe5,
    0xfffec, 0xfffff3, 0xfffed, 0x1fffe6, 0x3fffe9, 0x1fffe7, 0x1fffe8, 0x7ffff3,
    0x3fffea, 0x3fffeb, 0x1ffffee, 0x1ffffef, 0xfffff4, 0xfffff5, 0x3ffffea, 0x7ffff4,
    0x3ffffeb, 0x7ffffe6, 0x3ffffec, 0x3ffffed, 0x7ffffe7, 0x7ffffe8, 0x7ffffe9, 0x7ffffea,
    0x7ffffeb, 0xffffffe, 0x7ffffec, 0x7ffffed, 0x7ffffee, 0x7ffffef, 0x7fffff0, 0x3ffffee,
];

const HUFF_LEN: [u8; 256] = [
    13, 23, 28, 28, 28, 28, 28, 28, 28, 24, 30, 28, 28, 30, 28, 28,
    28, 28, 28, 28, 28, 28, 30, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    6, 10, 10, 12, 13, 6, 8, 11, 10, 10, 8, 11, 8, 6, 6, 6,
    5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 7, 8, 15, 6, 12, 10,
    13, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 8, 7, 8, 13, 19, 13, 14, 6,
    15, 5, 6, 5, 6, 5, 6, 6, 6, 5, 7, 7, 6, 6, 6, 5,
    6, 7, 6, 5, 5, 6, 7, 7, 7, 7, 7, 15, 11, 14, 13, 28,
    20, 22, 20, 20, 22, 22, 22, 23, 22, 23, 23, 23, 23, 23, 24, 23,
    24, 24, 22, 23, 24, 23, 23, 23, 23, 21, 22, 23, 22, 23, 23, 24,
    22, 21, 20, 22, 22, 23, 23, 21, 23, 22, 22, 24, 21, 22, 23, 23,
    21, 21, 22, 21, 23, 22, 23, 23, 20, 22, 22, 22, 23, 22, 22, 23,
    26, 26, 20, 19, 22, 23, 22, 25, 26, 26, 26, 27, 27, 26, 24, 25,
    19, 21, 26, 27, 27, 26, 27, 24, 21, 21, 26, 26, 28, 27, 27, 27,
    20, 24, 20, 21, 22, 21, 21, 23, 22, 22, 25, 25, 24, 24, 26, 23,
    26, 27, 26, 26, 27, 27, 27, 27, 27, 28, 27, 27, 27, 27, 27, 26,
];

// Decode un symbole Huffman a partir d'un code (longueur, valeur) si exact.
fn huff_lookup(nbits: u32, code: u32) -> Option<u8> {
    for sym in 0..256usize {
        if HUFF_LEN[sym] as u32 == nbits && HUFF_CODE[sym] == code {
            return Some(sym as u8);
        }
    }
    None
}

/// Decode une chaine encodee en Huffman HPACK.
pub fn huffman_decode(data: &[u8]) -> Option<String> {
    let mut out: Vec<u8> = Vec::new();
    let mut cur: u32 = 0;
    let mut nbits: u32 = 0;
    for &byte in data {
        for k in (0..8).rev() {
            let bit = (byte >> k) & 1;
            cur = (cur << 1) | bit as u32;
            nbits += 1;
            if nbits > 30 { return None; }
            if let Some(sym) = huff_lookup(nbits, cur) {
                out.push(sym);
                cur = 0;
                nbits = 0;
            }
        }
    }
    // Le rembourrage final est constitue des bits de poids fort de EOS (des 1),
    // sur strictement moins de 8 bits.
    if nbits >= 8 { return None; }
    if nbits > 0 {
        let mask = (1u32 << nbits) - 1;
        if cur & mask != mask { return None; }
    }
    String::from_utf8(out).ok()
}

// Decode un entier HPACK a prefixe de `prefix` bits. Renvoie (valeur, pos suivante).
fn decode_int(buf: &[u8], pos: usize, prefix: u32) -> Option<(usize, usize)> {
    if pos >= buf.len() { return None; }
    let mask = (1usize << prefix) - 1;
    let mut value = (buf[pos] as usize) & mask;
    let mut p = pos + 1;
    if value < mask {
        return Some((value, p));
    }
    let mut shift = 0u32;
    loop {
        if p >= buf.len() { return None; }
        let b = buf[p];
        p += 1;
        value += ((b & 0x7f) as usize) << shift;
        shift += 7;
        if b & 0x80 == 0 { break; }
        if shift > 28 { return None; }
    }
    Some((value, p))
}

// Decode une chaine HPACK (octet H + longueur + donnees). Renvoie (chaine, pos).
fn decode_string(buf: &[u8], pos: usize) -> Option<(String, usize)> {
    if pos >= buf.len() { return None; }
    let huff = buf[pos] & 0x80 != 0;
    let (len, p) = decode_int(buf, pos, 7)?;
    if p + len > buf.len() { return None; }
    let raw = &buf[p..p + len];
    let s = if huff {
        huffman_decode(raw)?
    } else {
        String::from(core::str::from_utf8(raw).ok()?)
    };
    Some((s, p + len))
}

/// Decodeur HPACK avec table dynamique persistante sur la connexion.
pub struct Decoder {
    dynamic: Vec<(String, String)>, // index 0 = entree la plus recente
    max_size: usize,
    size: usize,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder { dynamic: Vec::new(), max_size: 4096, size: 0 }
    }

    fn entry_size(name: &str, value: &str) -> usize {
        name.len() + value.len() + 32
    }

    fn evict_to_fit(&mut self) {
        while self.size > self.max_size {
            if let Some((n, v)) = self.dynamic.pop() {
                self.size -= Self::entry_size(&n, &v);
            } else {
                break;
            }
        }
    }

    fn insert(&mut self, name: String, value: String) {
        let sz = Self::entry_size(&name, &value);
        // Une entree plus grande que la table entiere vide la table (RFC 7541 §4.4).
        if sz > self.max_size {
            self.dynamic.clear();
            self.size = 0;
            return;
        }
        self.dynamic.insert(0, (name, value));
        self.size += sz;
        self.evict_to_fit();
    }

    fn set_max_size(&mut self, new_max: usize) {
        self.max_size = new_max;
        self.evict_to_fit();
    }

    // Resout un index (1-base) en (name, value).
    fn lookup(&self, index: usize) -> Option<(String, String)> {
        if index == 0 { return None; }
        if index <= STATIC_TABLE.len() {
            let (n, v) = STATIC_TABLE[index - 1];
            return Some((String::from(n), String::from(v)));
        }
        let di = index - STATIC_TABLE.len() - 1;
        self.dynamic.get(di).cloned()
    }

    /// Decode un bloc d'en-tetes complet en liste (nom, valeur).
    pub fn decode(&mut self, buf: &[u8]) -> Option<Vec<(String, String)>> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut pos = 0usize;
        while pos < buf.len() {
            let b = buf[pos];
            if b & 0x80 != 0 {
                // Indexed Header Field.
                let (idx, p) = decode_int(buf, pos, 7)?;
                let (n, v) = self.lookup(idx)?;
                out.push((n, v));
                pos = p;
            } else if b & 0x40 != 0 {
                // Literal Header Field with Incremental Indexing.
                let (idx, p) = decode_int(buf, pos, 6)?;
                let (name, p) = if idx == 0 {
                    decode_string(buf, p)?
                } else {
                    (self.lookup(idx)?.0, p)
                };
                let (value, p) = decode_string(buf, p)?;
                self.insert(name.clone(), value.clone());
                out.push((name, value));
                pos = p;
            } else if b & 0x20 != 0 {
                // Dynamic Table Size Update.
                let (new_max, p) = decode_int(buf, pos, 5)?;
                self.set_max_size(new_max);
                pos = p;
            } else {
                // Literal sans indexation (0x00) ou jamais indexe (0x10) : prefixe 4 bits.
                let (idx, p) = decode_int(buf, pos, 4)?;
                let (name, p) = if idx == 0 {
                    decode_string(buf, p)?
                } else {
                    (self.lookup(idx)?.0, p)
                };
                let (value, p) = decode_string(buf, p)?;
                out.push((name, value));
                pos = p;
            }
        }
        Some(out)
    }
}

// Encode un entier HPACK a prefixe (avec bits de drapeau dans l'octet de tete).
fn encode_int(out: &mut Vec<u8>, value: usize, prefix: u32, flags: u8) {
    let mask = (1usize << prefix) - 1;
    if value < mask {
        out.push(flags | value as u8);
        return;
    }
    out.push(flags | mask as u8);
    let mut v = value - mask;
    while v >= 128 {
        out.push(((v & 0x7f) as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

// Encode une chaine litterale sans Huffman (H=0).
fn encode_string(out: &mut Vec<u8>, s: &str) {
    encode_int(out, s.len(), 7, 0x00);
    out.extend_from_slice(s.as_bytes());
}

/// Encode un bloc d'en-tetes de requete en litteraux sans indexation (0x00) et
/// sans Huffman : toujours valide et accepte par tous les serveurs.
pub fn encode_request(headers: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in headers {
        out.push(0x00); // Literal Header Field without Indexing, new name.
        encode_string(&mut out, name);
        encode_string(&mut out, value);
    }
    out
}

/// Auto-test : vecteurs de l'Annexe C de la RFC 7541.
pub fn selftest() -> Result<(), &'static str> {
    // Huffman : "www.example.com" (C.4.1).
    let enc = [0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab, 0x90, 0xf4, 0xff];
    match huffman_decode(&enc) {
        Some(s) if s == "www.example.com" => {}
        _ => return Err("huffman www.example.com"),
    }

    // Entier a prefixe 5 : 1337 = 0x1f 0x9a 0x0a (RFC 7541 C.1.3).
    let (v, p) = decode_int(&[0x1f, 0x9a, 0x0a], 0, 5).ok_or("entier 1337")?;
    if v != 1337 || p != 3 { return Err("entier 1337 valeur"); }

    // Bloc complet C.4.1 : GET http / www.example.com avec Huffman.
    let block = [
        0x82, 0x86, 0x84, 0x41, 0x8c, 0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b,
        0xa0, 0xab, 0x90, 0xf4, 0xff,
    ];
    let mut dec = Decoder::new();
    let hs = dec.decode(&block).ok_or("decode bloc C.4.1")?;
    let want: &[(&str, &str)] = &[
        (":method", "GET"),
        (":scheme", "http"),
        (":path", "/"),
        (":authority", "www.example.com"),
    ];
    if hs.len() != want.len() { return Err("bloc C.4.1 nb en-tetes"); }
    for (got, exp) in hs.iter().zip(want) {
        if got.0 != exp.0 || got.1 != exp.1 { return Err("bloc C.4.1 contenu"); }
    }

    // Round-trip encodeur -> decodeur.
    let req = encode_request(&[(":method", "GET"), (":path", "/test"), ("x-a", "b")]);
    let mut dec2 = Decoder::new();
    let back = dec2.decode(&req).ok_or("round-trip decode")?;
    if back.len() != 3 || back[1].0 != ":path" || back[1].1 != "/test" {
        return Err("round-trip contenu");
    }
    Ok(())
}
