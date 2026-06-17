//! NIST P-256 (secp256r1) — ECDH et verification ECDSA.
//!
//! Arithmetique de corps via `bignum` (reutilise et teste). Coordonnees
//! jacobiennes pour eviter une inversion par operation. Objectif : verifier les
//! signatures ecdsa_secp256r1_sha256 (certificats et CertificateVerify) et
//! supporter l'echange de cles ECDHE P-256.

use super::bignum::BigUint;
use super::sha256::sha256;
use alloc::vec::Vec;
use core::cmp::Ordering;

fn p() -> BigUint {
    // p = ffffffff00000001000000000000000000000000ffffffffffffffffffffffff
    BigUint::from_bytes_be(&[
        0xff,0xff,0xff,0xff,0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    ])
}
fn n_order() -> BigUint {
    BigUint::from_bytes_be(&[
        0xff,0xff,0xff,0xff,0x00,0x00,0x00,0x00,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
        0xbc,0xe6,0xfa,0xad,0xa7,0x17,0x9e,0x84,0xf3,0xb9,0xca,0xc2,0xfc,0x63,0x25,0x51,
    ])
}
fn gx() -> BigUint {
    BigUint::from_bytes_be(&[
        0x6b,0x17,0xd1,0xf2,0xe1,0x2c,0x42,0x47,0xf8,0xbc,0xe6,0xe5,0x63,0xa4,0x40,0xf2,
        0x77,0x03,0x7d,0x81,0x2d,0xeb,0x33,0xa0,0xf4,0xa1,0x39,0x45,0xd8,0x98,0xc2,0x96,
    ])
}
fn gy() -> BigUint {
    BigUint::from_bytes_be(&[
        0x4f,0xe3,0x42,0xe2,0xfe,0x1a,0x7f,0x9b,0x8e,0xe7,0xeb,0x4a,0x7c,0x0f,0x9e,0x16,
        0x2b,0xce,0x33,0x57,0x6b,0x31,0x5e,0xce,0xcb,0xb6,0x40,0x68,0x37,0xbf,0x51,0xf5,
    ])
}
fn b_param() -> BigUint {
    BigUint::from_bytes_be(&[
        0x5a,0xc6,0x35,0xd8,0xaa,0x3a,0x93,0xe7,0xb3,0xeb,0xbd,0x55,0x76,0x98,0x86,0xbc,
        0x65,0x1d,0x06,0xb0,0xcc,0x53,0xb0,0xf6,0x3b,0xce,0x3c,0x3e,0x27,0xd2,0x60,0x4b,
    ])
}

// --- arithmetique modulaire dans le corps premier p ---
struct Field { p: BigUint }
impl Field {
    fn new() -> Field { Field { p: p() } }
    fn add(&self, a: &BigUint, b: &BigUint) -> BigUint {
        let s = a.add(b);
        if s.cmp_pub(&self.p) != Ordering::Less { s.sub(&self.p) } else { s }
    }
    fn sub(&self, a: &BigUint, b: &BigUint) -> BigUint {
        if a.cmp_pub(b) == Ordering::Less {
            a.add(&self.p).sub(b)
        } else {
            a.sub(b)
        }
    }
    fn mul(&self, a: &BigUint, b: &BigUint) -> BigUint {
        a.mul(b).rem(&self.p)
    }
    fn sqr(&self, a: &BigUint) -> BigUint { self.mul(a, a) }
    // Inverse via petit theoreme de Fermat : a^(p-2) mod p.
    fn inv(&self, a: &BigUint) -> BigUint {
        let two = BigUint::from_bytes_be(&[2]);
        let exp = self.p.sub(&two);
        a.modpow(&exp, &self.p)
    }
}

/// Point en coordonnees jacobiennes (X:Y:Z), infini si Z=0.
#[derive(Clone)]
pub struct Point { x: BigUint, y: BigUint, z: BigUint }

impl Point {
    fn infinity() -> Point {
        Point { x: BigUint::from_bytes_be(&[1]), y: BigUint::from_bytes_be(&[1]), z: BigUint::zero() }
    }
    fn is_infinity(&self) -> bool { self.z.is_zero() }
    fn affine(x: BigUint, y: BigUint) -> Point {
        Point { x, y, z: BigUint::from_bytes_be(&[1]) }
    }
}

fn point_double(f: &Field, pt: &Point) -> Point {
    if pt.is_infinity() || pt.y.is_zero() { return Point::infinity(); }
    // a = -3
    let z2 = f.sqr(&pt.z);
    let m = {
        let t1 = f.sub(&pt.x, &z2);     // X - Z^2
        let t2 = f.add(&pt.x, &z2);     // X + Z^2
        let prod = f.mul(&t1, &t2);
        let three = BigUint::from_bytes_be(&[3]);
        f.mul(&three, &prod)            // 3(X-Z^2)(X+Z^2)
    };
    let y2 = f.sqr(&pt.y);
    let s = {
        let four = BigUint::from_bytes_be(&[4]);
        let xy2 = f.mul(&pt.x, &y2);
        f.mul(&four, &xy2)              // 4*X*Y^2
    };
    let x3 = {
        let m2 = f.sqr(&m);
        let two_s = f.add(&s, &s);
        f.sub(&m2, &two_s)
    };
    let y3 = {
        let s_x3 = f.sub(&s, &x3);
        let t = f.mul(&m, &s_x3);
        let y4 = f.sqr(&y2);
        let eight = BigUint::from_bytes_be(&[8]);
        let ey4 = f.mul(&eight, &y4);
        f.sub(&t, &ey4)
    };
    let z3 = {
        let yz = f.mul(&pt.y, &pt.z);
        f.add(&yz, &yz)
    };
    Point { x: x3, y: y3, z: z3 }
}

fn point_add(f: &Field, p1: &Point, p2: &Point) -> Point {
    if p1.is_infinity() { return p2.clone(); }
    if p2.is_infinity() { return p1.clone(); }
    let z1z1 = f.sqr(&p1.z);
    let z2z2 = f.sqr(&p2.z);
    let u1 = f.mul(&p1.x, &z2z2);
    let u2 = f.mul(&p2.x, &z1z1);
    let s1 = f.mul(&p1.y, &f.mul(&z2z2, &p2.z));
    let s2 = f.mul(&p2.y, &f.mul(&z1z1, &p1.z));
    if u1.cmp_pub(&u2) == Ordering::Equal {
        if s1.cmp_pub(&s2) != Ordering::Equal {
            return Point::infinity();
        }
        return point_double(f, p1);
    }
    let h = f.sub(&u2, &u1);
    let r = f.sub(&s2, &s1);
    let h2 = f.sqr(&h);
    let h3 = f.mul(&h2, &h);
    let u1h2 = f.mul(&u1, &h2);
    let x3 = {
        let r2 = f.sqr(&r);
        let two_u1h2 = f.add(&u1h2, &u1h2);
        f.sub(&f.sub(&r2, &h3), &two_u1h2)
    };
    let y3 = {
        let t = f.sub(&u1h2, &x3);
        let rt = f.mul(&r, &t);
        let s1h3 = f.mul(&s1, &h3);
        f.sub(&rt, &s1h3)
    };
    let z3 = f.mul(&f.mul(&h, &p1.z), &p2.z);
    Point { x: x3, y: y3, z: z3 }
}

fn scalar_mul(f: &Field, k: &BigUint, pt: &Point) -> Point {
    let mut r = Point::infinity();
    let bits = k.bit_len();
    for i in (0..bits).rev() {
        r = point_double(f, &r);
        if k.get_bit(i) == 1 {
            r = point_add(f, &r, pt);
        }
    }
    r
}

fn to_affine_x(f: &Field, pt: &Point) -> Option<BigUint> {
    if pt.is_infinity() { return None; }
    let zinv = f.inv(&pt.z);
    let zinv2 = f.sqr(&zinv);
    Some(f.mul(&pt.x, &zinv2))
}

/// Decode une cle publique non compressee (0x04 || X(32) || Y(32)).
fn decode_pubkey(data: &[u8]) -> Option<Point> {
    if data.len() != 65 || data[0] != 0x04 { return None; }
    let x = BigUint::from_bytes_be(&data[1..33]);
    let y = BigUint::from_bytes_be(&data[33..65]);
    Some(Point::affine(x, y))
}

// Inverse modulaire mod n (ordre) via Fermat.
fn inv_mod(a: &BigUint, m: &BigUint) -> BigUint {
    let two = BigUint::from_bytes_be(&[2]);
    let exp = m.sub(&two);
    a.modpow(&exp, m)
}

/// Verifie une signature ECDSA P-256 (SHA-256).
/// `pubkey` : point non compresse (65 octets). `sig` : (r, s) bruts (32+32 ou r/s
/// de longueurs variables fournis separes).
pub fn verify_ecdsa_sha256(pubkey: &[u8], msg: &[u8], r: &[u8], s: &[u8]) -> bool {
    let n = n_order();
    let f = Field::new();
    let q = match decode_pubkey(pubkey) { Some(p) => p, None => return false };

    let r_i = BigUint::from_bytes_be(r);
    let s_i = BigUint::from_bytes_be(s);
    if r_i.is_zero() || s_i.is_zero() { return false; }
    if r_i.cmp_pub(&n) != Ordering::Less { return false; }
    if s_i.cmp_pub(&n) != Ordering::Less { return false; }

    let hash = sha256(msg);
    let e = BigUint::from_bytes_be(&hash).rem(&n);

    let w = inv_mod(&s_i, &n);
    let u1 = e.mul(&w).rem(&n);
    let u2 = r_i.mul(&w).rem(&n);

    let g = Point::affine(gx(), gy());
    let p1 = scalar_mul(&f, &u1, &g);
    let p2 = scalar_mul(&f, &u2, &q);
    let sum = point_add(&f, &p1, &p2);

    let x = match to_affine_x(&f, &sum) { Some(x) => x, None => return false };
    let x_mod_n = x.rem(&n);
    x_mod_n.cmp_pub(&r_i) == Ordering::Equal
}

/// ECDH P-256 : multiplie le point pair (65 oct non compresse) par le scalaire
/// prive (32 oct) ; renvoie la coordonnee X partagee (32 oct).
pub fn ecdh(private: &[u8; 32], peer_pubkey: &[u8]) -> Option<[u8; 32]> {
    let f = Field::new();
    let q = decode_pubkey(peer_pubkey)?;
    let k = BigUint::from_bytes_be(private);
    let shared = scalar_mul(&f, &k, &q);
    let x = to_affine_x(&f, &shared)?;
    let mut bytes = x.to_bytes_be();
    if bytes.len() > 32 { return None; }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    let _ = &mut bytes;
    Some(out)
}

/// Cle publique P-256 a partir d'un scalaire prive : 0x04||X||Y (65 oct).
pub fn derive_pubkey(private: &[u8; 32]) -> Vec<u8> {
    let f = Field::new();
    let g = Point::affine(gx(), gy());
    let k = BigUint::from_bytes_be(private);
    let pt = scalar_mul(&f, &k, &g);
    let zinv = f.inv(&pt.z);
    let zinv2 = f.sqr(&zinv);
    let zinv3 = f.mul(&zinv2, &zinv);
    let x = f.mul(&pt.x, &zinv2);
    let y = f.mul(&pt.y, &zinv3);
    let mut out = alloc::vec![0u8; 65];
    out[0] = 0x04;
    let xb = x.to_bytes_be();
    let yb = y.to_bytes_be();
    out[1 + (32 - xb.len())..33].copy_from_slice(&xb);
    out[33 + (32 - yb.len())..65].copy_from_slice(&yb);
    out
}

/// Auto-test : verifie que G est sur la courbe et un vecteur ECDSA connu.
pub fn selftest() -> Result<(), &'static str> {
    let f = Field::new();
    // y^2 == x^3 - 3x + b  (mod p) pour G
    let x = gx(); let y = gy();
    let y2 = f.sqr(&y);
    let x3 = f.mul(&f.sqr(&x), &x);
    let three = BigUint::from_bytes_be(&[3]);
    let three_x = f.mul(&three, &x);
    let rhs = f.add(&f.sub(&x3, &three_x), &b_param());
    if y2.cmp_pub(&rhs) != Ordering::Equal { return Err("G hors courbe"); }

    // 2G puis verif que c'est sur la courbe.
    let g = Point::affine(gx(), gy());
    let g2 = point_double(&f, &g);
    let x2 = to_affine_x(&f, &g2).ok_or("2G infini")?;
    // x(2G) connu (NIST) :
    let want = BigUint::from_bytes_be(&[
        0x7c,0xf2,0x7b,0x18,0x8d,0x03,0x4f,0x7e,0x8a,0x52,0x38,0x03,0x04,0xb5,0x1a,0xc3,
        0xc0,0x89,0x69,0xe2,0x77,0xf2,0x1b,0x35,0xa6,0x0b,0x48,0xfc,0x47,0x66,0x99,0x78,
    ]);
    if x2.cmp_pub(&want) != Ordering::Equal { return Err("2G incorrect"); }

    // Vecteur ECDSA P-256 / SHA-256 (FIPS 186-4 exemple).
    // Qx, Qy
    let mut pub_un = alloc::vec![0x04u8];
    pub_un.extend_from_slice(&hex(
        "60fed4ba255a9d31c961eb74c6356d68c049b8923b61fa6ce669622e60f29fb6"));
    pub_un.extend_from_slice(&hex(
        "7903fe1008b8bc99a41ae9e95628bc64f2f1b20c2d7e9f5177a3c294d4462299"));
    let msg = b"sample";
    let r = hex("efd48b2aacb6a8fd1140dd9cd45e81d69d2c877b56aaf991c34d0ea84eaf3716");
    let s = hex("f7cb1c942d657c41d436c7a1b6e29f65f3e900dbb9aff4064dc4ab2f843acda8");
    if !verify_ecdsa_sha256(&pub_un, msg, &r, &s) {
        return Err("ecdsa verify (vecteur connu)");
    }
    // Signature alteree doit echouer.
    let mut bad = s.clone(); bad[31] ^= 1;
    if verify_ecdsa_sha256(&pub_un, msg, &r, &bad) {
        return Err("ecdsa accepte signature fausse");
    }
    Ok(())
}

fn hex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i + 1 < b.len() {
        out.push((hv(b[i]) << 4) | hv(b[i + 1]));
        i += 2;
    }
    out
}
fn hv(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}
