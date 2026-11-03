//! Ed25519 signature VERIFICATION and SHA-512 (BOUCHAUD_SIGNATURE_V1).
//!
//! `no_std`, no allocation, no dependency. Verification only: Bouchaud never
//! signs on the device. The field and group arithmetic is a port of
//! TweetNaCl's `crypto_sign_open` (public domain, Bernstein et al.), plus the
//! RFC 8032 check that S < L (TweetNaCl omits it: signature malleability).
//! Variable time is acceptable: every input is public (key, message,
//! signature). The SHA-512 constants were COMPUTED (cube / square roots of
//! the first primes), not copied.
//!
//! Proof: tools/update/test_signature.rs -- RFC 8032 vectors, signatures
//! made by OpenSSL at test time, every bit flip of key / message / signature
//! rejected, S >= L rejected.

// ---------------------------------------------------------------- SHA-512

const K: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
    0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
    0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
    0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
    0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817
];
const H0: [u64; 8] = [0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179];

pub struct Sha512 { h: [u64; 8], bloc: [u8; 128], n: usize, total: u128 }

impl Sha512 {
    pub const fn new() -> Self { Self { h: H0, bloc: [0; 128], n: 0, total: 0 } }

    fn compresse(h: &mut [u64; 8], b: &[u8]) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            let mut x = [0u8; 8];
            x.copy_from_slice(&b[8 * i..8 * i + 8]);
            w[i] = u64::from_be_bytes(x);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = *h;
        for i in 0..80 {
            let s1 = v[4].rotate_right(14) ^ v[4].rotate_right(18) ^ v[4].rotate_right(41);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(28) ^ v[0].rotate_right(34) ^ v[0].rotate_right(39);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6]; v[6] = v[5]; v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2]; v[2] = v[1]; v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 { h[i] = h[i].wrapping_add(v[i]); }
    }

    pub fn update(&mut self, mut d: &[u8]) {
        self.total += d.len() as u128;
        while !d.is_empty() {
            let n = (128 - self.n).min(d.len());
            self.bloc[self.n..self.n + n].copy_from_slice(&d[..n]);
            self.n += n;
            d = &d[n..];
            if self.n == 128 {
                let b = self.bloc;
                Self::compresse(&mut self.h, &b);
                self.n = 0;
            }
        }
    }

    pub fn finalize(mut self) -> [u8; 64] {
        let bits = self.total * 8;
        let mut pad = [0u8; 256];
        pad[0] = 0x80;
        let lpad = if self.n < 112 { 112 - self.n } else { 240 - self.n };
        self.update_sans_total(&pad[..lpad]);
        self.update_sans_total(&bits.to_be_bytes());
        let mut o = [0u8; 64];
        for i in 0..8 { o[8 * i..8 * i + 8].copy_from_slice(&self.h[i].to_be_bytes()); }
        o
    }

    fn update_sans_total(&mut self, d: &[u8]) {
        let t = self.total;
        self.update(d);
        self.total = t;
    }
}

pub fn sha512(d: &[u8]) -> [u8; 64] { let mut h = Sha512::new(); h.update(d); h.finalize() }

// ---------------------------------------------------------- GF(2^255 - 19)

type Gf = [i64; 16];
const GF0: Gf = [0; 16];
const GF1: Gf = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const D: Gf = [0x78a3, 0x1359, 0x4dca, 0x75eb, 0xd8ab, 0x4141, 0x0a4d, 0x0070,
               0xe898, 0x7779, 0x4079, 0x8cc7, 0xfe73, 0x2b6f, 0x6cee, 0x5203];
const D2: Gf = [0xf159, 0x26b2, 0x9b94, 0xebd6, 0xb156, 0x8283, 0x149a, 0x00e0,
                0xd130, 0xeef3, 0x80f2, 0x198e, 0xfce7, 0x56df, 0xd9dc, 0x2406];
const X: Gf = [0xd51a, 0x8f25, 0x2d60, 0xc956, 0xa7b2, 0x9525, 0xc760, 0x692c,
               0xdc5c, 0xfdd6, 0xe231, 0xc0a4, 0x53fe, 0xcd6e, 0x36d3, 0x2169];
const Y: Gf = [0x6658, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666,
               0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666];
const I: Gf = [0xa0b0, 0x4a0e, 0x1b27, 0xc4ee, 0xe478, 0xad2f, 0x1806, 0x2f43,
               0xd7a7, 0x3dfb, 0x0099, 0x2b4d, 0xdf0b, 0x4fc1, 0x2480, 0x2b83];

fn car(o: &mut Gf) {
    for i in 0..16 {
        o[i] += 1 << 16;
        let c = o[i] >> 16;
        if i < 15 { o[i + 1] += c - 1; } else { o[0] += 38 * (c - 1); }
        o[i] -= c << 16;
    }
}

fn sel(p: &mut Gf, q: &mut Gf, b: i64) {
    let c = !(b - 1);
    for i in 0..16 { let t = c & (p[i] ^ q[i]); p[i] ^= t; q[i] ^= t; }
}

fn pack25519(n: &Gf) -> [u8; 32] {
    let mut t = *n;
    car(&mut t); car(&mut t); car(&mut t);
    let mut m: Gf = [0; 16];
    for _ in 0..2 {
        m[0] = t[0] - 0xffed;
        for i in 1..15 {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        let b = (m[15] >> 16) & 1;
        m[14] &= 0xffff;
        sel(&mut t, &mut m, 1 - b);
    }
    let mut o = [0u8; 32];
    for i in 0..16 { o[2 * i] = (t[i] & 0xff) as u8; o[2 * i + 1] = (t[i] >> 8) as u8; }
    o
}

fn neq(a: &Gf, b: &Gf) -> bool { pack25519(a) != pack25519(b) }
fn par(a: &Gf) -> u8 { pack25519(a)[0] & 1 }

fn unpack25519(n: &[u8]) -> Gf {
    let mut o: Gf = [0; 16];
    for i in 0..16 { o[i] = n[2 * i] as i64 + ((n[2 * i + 1] as i64) << 8); }
    o[15] &= 0x7fff;
    o
}

fn fa(a: &Gf, b: &Gf) -> Gf { let mut o = [0; 16]; for i in 0..16 { o[i] = a[i] + b[i]; } o }
fn fz(a: &Gf, b: &Gf) -> Gf { let mut o = [0; 16]; for i in 0..16 { o[i] = a[i] - b[i]; } o }
fn fm(a: &Gf, b: &Gf) -> Gf {
    let mut t = [0i64; 31];
    for i in 0..16 { for j in 0..16 { t[i + j] += a[i] * b[j]; } }
    for i in 0..15 { t[i] += 38 * t[i + 16]; }
    let mut o = [0i64; 16];
    o.copy_from_slice(&t[..16]);
    car(&mut o); car(&mut o);
    o
}
fn fs(a: &Gf) -> Gf { fm(a, a) }

fn inv25519(i: &Gf) -> Gf {
    let mut c = *i;
    for a in (0..=253).rev() { c = fs(&c); if a != 2 && a != 4 { c = fm(&c, i); } }
    c
}

fn pow2523(i: &Gf) -> Gf {
    let mut c = *i;
    for a in (0..=250).rev() { c = fs(&c); if a != 1 { c = fm(&c, i); } }
    c
}

// ------------------------------------------------------------------ points

type Pt = [Gf; 4];

fn add(p: &mut Pt, q: &Pt) {
    let a = fm(&fz(&p[1], &p[0]), &fz(&q[1], &q[0]));
    let b = fm(&fa(&p[0], &p[1]), &fa(&q[0], &q[1]));
    let c = fm(&fm(&p[3], &q[3]), &D2);
    let d0 = fm(&p[2], &q[2]);
    let d = fa(&d0, &d0);
    let e = fz(&b, &a);
    let f = fz(&d, &c);
    let g = fa(&d, &c);
    let h = fa(&b, &a);
    p[0] = fm(&e, &f);
    p[1] = fm(&h, &g);
    p[2] = fm(&g, &f);
    p[3] = fm(&e, &h);
}

fn cswap(p: &mut Pt, q: &mut Pt, b: u8) {
    for i in 0..4 { sel(&mut p[i], &mut q[i], b as i64); }
}

fn pack(p: &Pt) -> [u8; 32] {
    let zi = inv25519(&p[2]);
    let tx = fm(&p[0], &zi);
    let ty = fm(&p[1], &zi);
    let mut r = pack25519(&ty);
    r[31] ^= par(&tx) << 7;
    r
}

fn scalarmult(q: &mut Pt, s: &[u8; 32]) -> Pt {
    let mut p: Pt = [GF0, GF1, GF1, GF0];
    for i in (0..256).rev() {
        let b = (s[i / 8] >> (i & 7)) & 1;
        cswap(&mut p, q, b);
        let pc = p;
        add(q, &pc);
        let pc = p;
        add(&mut p, &pc);
        cswap(&mut p, q, b);
    }
    p
}

fn scalarbase(s: &[u8; 32]) -> Pt {
    let mut q: Pt = [X, Y, GF1, fm(&X, &Y)];
    scalarmult(&mut q, s)
}

const L: [i64; 32] = [0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2,
                      0xde, 0xf9, 0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10];

fn mod_l(x: &mut [i64; 64]) -> [u8; 32] {
    for i in (32..64).rev() {
        let mut carry = 0i64;
        let mut j = i - 32;
        while j < i - 12 {
            x[j] += carry - 16 * x[i] * L[j - (i - 32)];
            carry = (x[j] + 128) >> 8;
            x[j] -= carry << 8;
            j += 1;
        }
        x[j] += carry;
        x[i] = 0;
    }
    let mut carry = 0i64;
    for j in 0..32 {
        x[j] += carry - (x[31] >> 4) * L[j];
        carry = x[j] >> 8;
        x[j] &= 255;
    }
    for j in 0..32 { x[j] -= carry * L[j]; }
    let mut r = [0u8; 32];
    for i in 0..32 { x[i + 1] += x[i] >> 8; r[i] = (x[i] & 255) as u8; }
    r
}

fn reduce(h: &[u8; 64]) -> [u8; 32] {
    let mut x = [0i64; 64];
    for i in 0..64 { x[i] = h[i] as i64; }
    mod_l(&mut x)
}

/// -A from its encoding, or None if it is not a point.
fn unpackneg(p: &[u8; 32]) -> Option<Pt> {
    let mut r: Pt = [GF0, GF0, GF1, GF0];
    r[1] = unpack25519(p);
    let mut num = fs(&r[1]);
    let mut den = fm(&num, &D);
    num = fz(&num, &r[2]);
    den = fa(&r[2], &den);
    let den2 = fs(&den);
    let den4 = fs(&den2);
    let den6 = fm(&den4, &den2);
    let mut t = fm(&fm(&den6, &num), &den);
    t = pow2523(&t);
    t = fm(&fm(&fm(&t, &num), &den), &den);
    r[0] = fm(&t, &den);
    let mut chk = fm(&fs(&r[0]), &den);
    if neq(&chk, &num) { r[0] = fm(&r[0], &I); }
    chk = fm(&fs(&r[0]), &den);
    if neq(&chk, &num) { return None; }
    if par(&r[0]) == (p[31] >> 7) { r[0] = fz(&GF0, &r[0]); }
    r[3] = fm(&r[0], &r[1]);
    Some(r)
}

/// S < L, little endian (RFC 8032 5.1.7: reject malleable signatures).
fn s_canonique(s: &[u8]) -> bool {
    for i in (0..32).rev() {
        let (a, b) = (s[i] as i64, L[i]);
        if a < b { return true; }
        if a > b { return false; }
    }
    false
}

/// RFC 8032 5.1.3: the encoded y (top bit cleared) must be < p. TweetNaCl's
/// unpack25519 silently reduces y mod p, which accepts e.g. y = 1 + p as the
/// identity (BOUCHAUD_ED25519_STRICT_V1, found by
/// tools/update/ed25519_adversarial.py; OpenSSL 3 behaves the same).
fn y_canonique(b: &[u8; 32]) -> bool {
    // p = 2^255 - 19 = ed ff .. ff 7f (little endian). y >= p iff the top
    // byte (sign masked) is 0x7f, the middle bytes are all 0xff, and the low
    // byte is >= 0xed.
    !((b[31] & 0x7f) == 0x7f && b[1..31].iter().all(|&x| x == 0xff) && b[0] >= 0xed)
}

/// RFC 8032 5.1.3: "if x = 0 and x_0 = 1, decoding fails" (negative zero).
fn pas_zero_negatif(b: &[u8; 32], pt: &Pt) -> bool {
    // Extended coordinates with Z != 0: x == 0 iff X == 0 (no inversion).
    b[31] >> 7 == 0 || pack25519(&pt[0]) != [0u8; 32]
}

/// Strict decoding: canonical y, no negative zero, on the curve. Returns -P.
fn decode_strict(b: &[u8; 32]) -> Option<Pt> {
    if !y_canonique(b) { return None; }
    let p = unpackneg(b)?;
    if !pas_zero_negatif(b, &p) { return None; }
    Some(p)
}

/// [8]P is the identity: P lies in the 8-torsion subgroup (small order,
/// identity included). Same test on -P. Three doublings, not a 256-bit
/// ladder, and no inversion: this runs inside the `begin` syscall that the
/// update agent's heartbeat waits on (BOUCHAUD_ED25519_PETIT_ORDRE_V2).
fn petit_ordre(p: &Pt) -> bool {
    let mut r = *p;
    for _ in 0..3 {
        let c = r;
        add(&mut r, &c);
    }
    // Identity in extended coordinates: X == 0 and Y == Z (no inversion).
    pack25519(&r[0]) == [0u8; 32] && pack25519(&fz(&r[1], &r[2])) == [0u8; 32]
}

/// RFC 8032 Ed25519 verification of `signature` (R || S) over `message`
/// under the public key `cle`, with Bouchaud's STRICT policy on top of the
/// cofactorless equation [S]B = R + [k]A:
///   * S < L (RFC 8032 5.1.7);
///   * A and R decode strictly (RFC 8032 5.1.3: y < p, no negative zero);
///   * A and R are not of small order (libsodium's rule; neither a release
///     key nor the R of an honest signature ever is). Without it, the
///     identity key with R = identity, S = 0 verifies ANY message.
pub fn verifie(cle: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    if !s_canonique(&signature[32..]) { return false; }
    let Some(mut q) = decode_strict(cle) else { return false };
    if petit_ordre(&q) { return false; }
    let mut r_enc = [0u8; 32];
    r_enc.copy_from_slice(&signature[..32]);
    let Some(r_pt) = decode_strict(&r_enc) else { return false };
    if petit_ordre(&r_pt) { return false; }
    let mut h = Sha512::new();
    h.update(&signature[..32]);
    h.update(cle);
    h.update(message);
    let k = reduce(&h.finalize());
    let mut p = scalarmult(&mut q, &k);
    let mut s = [0u8; 32];
    s.copy_from_slice(&signature[32..]);
    let sb = scalarbase(&s);
    add(&mut p, &sb);
    pack(&p) == signature[..32]
}
