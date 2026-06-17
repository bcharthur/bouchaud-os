//! NIST P-384 (secp384r1) — verification ECDSA.
//!
//! Arithmetique de corps via `bignum` (reutilise et teste). Coordonnees
//! jacobiennes pour eviter une inversion par operation. Objectif : verifier les
//! signatures ecdsa_secp384r1_sha384 (certificats et CertificateVerify).

use super::bignum::BigUint;
use super::sha512::sha384;
use alloc::vec::Vec;
use core::cmp::Ordering;

fn p() -> BigUint {
    BigUint::from_bytes_be(&hex("fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff"))
}
fn n_order() -> BigUint {
    BigUint::from_bytes_be(&hex("ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973"))
}
fn gx() -> BigUint {
    BigUint::from_bytes_be(&hex("aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7"))
}
fn gy() -> BigUint {
    BigUint::from_bytes_be(&hex("3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f"))
}
fn b_param() -> BigUint {
    BigUint::from_bytes_be(&hex("b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef"))
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

/// Decode une cle publique non compressee (0x04 || X(48) || Y(48)).
fn decode_pubkey(data: &[u8]) -> Option<Point> {
    if data.len() != 97 || data[0] != 0x04 { return None; }
    let x = BigUint::from_bytes_be(&data[1..49]);
    let y = BigUint::from_bytes_be(&data[49..97]);
    Some(Point::affine(x, y))
}

// Inverse modulaire mod n (ordre) via Fermat.
fn inv_mod(a: &BigUint, m: &BigUint) -> BigUint {
    let two = BigUint::from_bytes_be(&[2]);
    let exp = m.sub(&two);
    a.modpow(&exp, m)
}

/// Verifie une signature ECDSA P-384 (SHA-384).
/// `pubkey` : point non compresse (65 octets). `sig` : (r, s) bruts (32+32 ou r/s
/// de longueurs variables fournis separes).
pub fn verify_ecdsa_sha384(pubkey: &[u8], msg: &[u8], r: &[u8], s: &[u8]) -> bool {
    let n = n_order();
    let f = Field::new();
    let q = match decode_pubkey(pubkey) { Some(p) => p, None => return false };

    let r_i = BigUint::from_bytes_be(r);
    let s_i = BigUint::from_bytes_be(s);
    if r_i.is_zero() || s_i.is_zero() { return false; }
    if r_i.cmp_pub(&n) != Ordering::Less { return false; }
    if s_i.cmp_pub(&n) != Ordering::Less { return false; }

    let hash = sha384(msg);
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

/// Auto-test : verifie que G est sur la courbe et que n*G vaut l'infini.
pub fn selftest() -> Result<(), &'static str> {
    let f = Field::new();
    let x = gx(); let y = gy();
    let y2 = f.sqr(&y);
    let x3 = f.mul(&f.sqr(&x), &x);
    let three = BigUint::from_bytes_be(&[3]);
    let three_x = f.mul(&three, &x);
    let rhs = f.add(&f.sub(&x3, &three_x), &b_param());
    if y2.cmp_pub(&rhs) != Ordering::Equal { return Err("G hors courbe"); }
    let g = Point::affine(gx(), gy());
    if !scalar_mul(&f, &n_order(), &g).is_infinity() { return Err("ordre incorrect"); }
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
