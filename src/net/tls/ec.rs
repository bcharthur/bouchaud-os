//! Courbe de Weierstrass courte generique : `y^2 = x^3 - 3x + b` sur un corps
//! premier `F_p`. Le coefficient `a = -3` est commun a toutes les courbes NIST
//! P-r (secp*r1), il suffit donc de faire varier (p, n, G, b) et la taille des
//! coordonnees pour couvrir P-256, P-384, etc.
//!
//! Arithmetique de corps via `bignum` (reutilise et teste). Coordonnees
//! jacobiennes pour eviter une inversion par operation. Sert a verifier les
//! signatures ECDSA (certificats et CertificateVerify) et a l'echange ECDHE.
//!
//! Ce module ne connait aucune fonction de hachage : `verify_ecdsa` recoit le
//! condensat deja calcule (SHA-256 pour P-256, SHA-384 pour P-384), ce qui le
//! garde agnostique de la suite cryptographique.

use super::bignum::BigUint;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// Parametres d'une courbe NIST sous forme `y^2 = x^3 - 3x + b`.
pub struct Curve {
    p: BigUint,       // module du corps premier
    n: BigUint,       // ordre du sous-groupe genere par G
    gx: BigUint,      // coordonnee X du generateur
    gy: BigUint,      // coordonnee Y du generateur
    b: BigUint,       // coefficient b
    coord_len: usize, // taille d'une coordonnee en octets (32 P-256, 48 P-384)
}

/// Point en coordonnees jacobiennes (X:Y:Z), infini si Z=0.
#[derive(Clone)]
pub struct Point {
    x: BigUint,
    y: BigUint,
    z: BigUint,
}

impl Point {
    fn infinity() -> Point {
        Point { x: BigUint::from_bytes_be(&[1]), y: BigUint::from_bytes_be(&[1]), z: BigUint::zero() }
    }
    fn is_infinity(&self) -> bool { self.z.is_zero() }
    fn affine(x: BigUint, y: BigUint) -> Point {
        Point { x, y, z: BigUint::from_bytes_be(&[1]) }
    }
}

impl Curve {
    /// Construit une courbe depuis ses parametres bruts (big-endian).
    pub fn new(p: &[u8], n: &[u8], gx: &[u8], gy: &[u8], b: &[u8], coord_len: usize) -> Curve {
        Curve {
            p: BigUint::from_bytes_be(p),
            n: BigUint::from_bytes_be(n),
            gx: BigUint::from_bytes_be(gx),
            gy: BigUint::from_bytes_be(gy),
            b: BigUint::from_bytes_be(b),
            coord_len,
        }
    }

    // --- arithmetique modulaire dans le corps premier p ---

    fn f_add(&self, a: &BigUint, b: &BigUint) -> BigUint {
        let s = a.add(b);
        if s.cmp_pub(&self.p) != Ordering::Less { s.sub(&self.p) } else { s }
    }
    fn f_sub(&self, a: &BigUint, b: &BigUint) -> BigUint {
        if a.cmp_pub(b) == Ordering::Less {
            a.add(&self.p).sub(b)
        } else {
            a.sub(b)
        }
    }
    fn f_mul(&self, a: &BigUint, b: &BigUint) -> BigUint {
        a.mul(b).rem(&self.p)
    }
    fn f_sqr(&self, a: &BigUint) -> BigUint { self.f_mul(a, a) }
    // Inverse via petit theoreme de Fermat : a^(p-2) mod p.
    fn f_inv(&self, a: &BigUint) -> BigUint {
        let two = BigUint::from_bytes_be(&[2]);
        let exp = self.p.sub(&two);
        a.modpow(&exp, &self.p)
    }

    // --- arithmetique de points (jacobiennes, a = -3) ---

    fn point_double(&self, pt: &Point) -> Point {
        if pt.is_infinity() || pt.y.is_zero() { return Point::infinity(); }
        // a = -3
        let z2 = self.f_sqr(&pt.z);
        let m = {
            let t1 = self.f_sub(&pt.x, &z2);    // X - Z^2
            let t2 = self.f_add(&pt.x, &z2);    // X + Z^2
            let prod = self.f_mul(&t1, &t2);
            let three = BigUint::from_bytes_be(&[3]);
            self.f_mul(&three, &prod)           // 3(X-Z^2)(X+Z^2)
        };
        let y2 = self.f_sqr(&pt.y);
        let s = {
            let four = BigUint::from_bytes_be(&[4]);
            let xy2 = self.f_mul(&pt.x, &y2);
            self.f_mul(&four, &xy2)             // 4*X*Y^2
        };
        let x3 = {
            let m2 = self.f_sqr(&m);
            let two_s = self.f_add(&s, &s);
            self.f_sub(&m2, &two_s)
        };
        let y3 = {
            let s_x3 = self.f_sub(&s, &x3);
            let t = self.f_mul(&m, &s_x3);
            let y4 = self.f_sqr(&y2);
            let eight = BigUint::from_bytes_be(&[8]);
            let ey4 = self.f_mul(&eight, &y4);
            self.f_sub(&t, &ey4)
        };
        let z3 = {
            let yz = self.f_mul(&pt.y, &pt.z);
            self.f_add(&yz, &yz)
        };
        Point { x: x3, y: y3, z: z3 }
    }

    fn point_add(&self, p1: &Point, p2: &Point) -> Point {
        if p1.is_infinity() { return p2.clone(); }
        if p2.is_infinity() { return p1.clone(); }
        let z1z1 = self.f_sqr(&p1.z);
        let z2z2 = self.f_sqr(&p2.z);
        let u1 = self.f_mul(&p1.x, &z2z2);
        let u2 = self.f_mul(&p2.x, &z1z1);
        let s1 = self.f_mul(&p1.y, &self.f_mul(&z2z2, &p2.z));
        let s2 = self.f_mul(&p2.y, &self.f_mul(&z1z1, &p1.z));
        if u1.cmp_pub(&u2) == Ordering::Equal {
            if s1.cmp_pub(&s2) != Ordering::Equal {
                return Point::infinity();
            }
            return self.point_double(p1);
        }
        let h = self.f_sub(&u2, &u1);
        let r = self.f_sub(&s2, &s1);
        let h2 = self.f_sqr(&h);
        let h3 = self.f_mul(&h2, &h);
        let u1h2 = self.f_mul(&u1, &h2);
        let x3 = {
            let r2 = self.f_sqr(&r);
            let two_u1h2 = self.f_add(&u1h2, &u1h2);
            self.f_sub(&self.f_sub(&r2, &h3), &two_u1h2)
        };
        let y3 = {
            let t = self.f_sub(&u1h2, &x3);
            let rt = self.f_mul(&r, &t);
            let s1h3 = self.f_mul(&s1, &h3);
            self.f_sub(&rt, &s1h3)
        };
        let z3 = self.f_mul(&self.f_mul(&h, &p1.z), &p2.z);
        Point { x: x3, y: y3, z: z3 }
    }

    fn scalar_mul(&self, k: &BigUint, pt: &Point) -> Point {
        let mut r = Point::infinity();
        let bits = k.bit_len();
        for i in (0..bits).rev() {
            r = self.point_double(&r);
            if k.get_bit(i) == 1 {
                r = self.point_add(&r, pt);
            }
        }
        r
    }

    fn to_affine_x(&self, pt: &Point) -> Option<BigUint> {
        if pt.is_infinity() { return None; }
        let zinv = self.f_inv(&pt.z);
        let zinv2 = self.f_sqr(&zinv);
        Some(self.f_mul(&pt.x, &zinv2))
    }

    fn generator(&self) -> Point { Point::affine(self.gx.clone(), self.gy.clone()) }

    /// Decode une cle publique non compressee (0x04 || X || Y), chaque
    /// coordonnee sur `coord_len` octets.
    fn decode_pubkey(&self, data: &[u8]) -> Option<Point> {
        let total = 1 + 2 * self.coord_len;
        if data.len() != total || data[0] != 0x04 { return None; }
        let x = BigUint::from_bytes_be(&data[1..1 + self.coord_len]);
        let y = BigUint::from_bytes_be(&data[1 + self.coord_len..total]);
        Some(Point::affine(x, y))
    }

    // Inverse modulaire mod n (ordre) via Fermat.
    fn inv_mod_n(&self, a: &BigUint) -> BigUint {
        let two = BigUint::from_bytes_be(&[2]);
        let exp = self.n.sub(&two);
        a.modpow(&exp, &self.n)
    }

    // Encode une coordonnee de corps sur exactement `coord_len` octets (None si
    // elle deborde, ce qui ne devrait jamais arriver pour un point valide).
    fn encode_coord(&self, v: &BigUint) -> Option<Vec<u8>> {
        let bytes = v.to_bytes_be();
        if bytes.len() > self.coord_len { return None; }
        let mut out = alloc::vec![0u8; self.coord_len];
        out[self.coord_len - bytes.len()..].copy_from_slice(&bytes);
        Some(out)
    }

    /// Verifie une signature ECDSA. `digest` est le condensat du message deja
    /// calcule avec la fonction de hachage associee a la courbe. `r` et `s` sont
    /// les entiers de la signature en big-endian.
    pub fn verify_ecdsa(&self, pubkey: &[u8], digest: &[u8], r: &[u8], s: &[u8]) -> bool {
        let q = match self.decode_pubkey(pubkey) { Some(p) => p, None => return false };

        let r_i = BigUint::from_bytes_be(r);
        let s_i = BigUint::from_bytes_be(s);
        if r_i.is_zero() || s_i.is_zero() { return false; }
        if r_i.cmp_pub(&self.n) != Ordering::Less { return false; }
        if s_i.cmp_pub(&self.n) != Ordering::Less { return false; }

        let e = BigUint::from_bytes_be(digest).rem(&self.n);
        let w = self.inv_mod_n(&s_i);
        let u1 = e.mul(&w).rem(&self.n);
        let u2 = r_i.mul(&w).rem(&self.n);

        let p1 = self.scalar_mul(&u1, &self.generator());
        let p2 = self.scalar_mul(&u2, &q);
        let sum = self.point_add(&p1, &p2);

        let x = match self.to_affine_x(&sum) { Some(x) => x, None => return false };
        x.rem(&self.n).cmp_pub(&r_i) == Ordering::Equal
    }

    /// ECDH : multiplie la cle publique du pair (non compressee) par le scalaire
    /// prive ; renvoie la coordonnee X partagee sur `coord_len` octets.
    pub fn ecdh(&self, private: &[u8], peer_pubkey: &[u8]) -> Option<Vec<u8>> {
        let q = self.decode_pubkey(peer_pubkey)?;
        let k = BigUint::from_bytes_be(private);
        let shared = self.scalar_mul(&k, &q);
        let x = self.to_affine_x(&shared)?;
        self.encode_coord(&x)
    }

    /// Cle publique non compressee 0x04 || X || Y a partir d'un scalaire prive.
    pub fn derive_pubkey(&self, private: &[u8]) -> Vec<u8> {
        let k = BigUint::from_bytes_be(private);
        let pt = self.scalar_mul(&k, &self.generator());
        let zinv = self.f_inv(&pt.z);
        let zinv2 = self.f_sqr(&zinv);
        let zinv3 = self.f_mul(&zinv2, &zinv);
        let x = self.f_mul(&pt.x, &zinv2);
        let y = self.f_mul(&pt.y, &zinv3);
        let mut out = alloc::vec![0u8; 1 + 2 * self.coord_len];
        out[0] = 0x04;
        if let Some(xb) = self.encode_coord(&x) {
            out[1..1 + self.coord_len].copy_from_slice(&xb);
        }
        if let Some(yb) = self.encode_coord(&y) {
            out[1 + self.coord_len..].copy_from_slice(&yb);
        }
        out
    }

    // --- briques d'auto-test (exposees aux modules de courbe concrets) ---

    /// Verifie que G satisfait `y^2 = x^3 - 3x + b`.
    pub fn generator_on_curve(&self) -> bool {
        let y2 = self.f_sqr(&self.gy);
        let x3 = self.f_mul(&self.f_sqr(&self.gx), &self.gx);
        let three = BigUint::from_bytes_be(&[3]);
        let three_x = self.f_mul(&three, &self.gx);
        let rhs = self.f_add(&self.f_sub(&x3, &three_x), &self.b);
        y2.cmp_pub(&rhs) == Ordering::Equal
    }

    /// Coordonnee X affine de 2G (pour comparaison a un vecteur connu).
    pub fn double_g_x(&self) -> Option<BigUint> {
        self.to_affine_x(&self.point_double(&self.generator()))
    }

    /// Verifie que n*G = O (ordre du generateur correct).
    pub fn order_is_valid(&self) -> bool {
        self.scalar_mul(&self.n, &self.generator()).is_infinity()
    }
}
