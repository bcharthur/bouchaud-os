//! NIST P-384 (secp384r1) — verification ECDSA.
//!
//! Simple instance de la courbe de Weierstrass courte generique (`ec`) avec les
//! parametres secp384r1. Le hachage associe est SHA-384. Objectif : verifier les
//! signatures ecdsa_secp384r1_sha384 (certificats et CertificateVerify), ce qui
//! couvre la majorite des sites ECDSA modernes (dont Google).

use super::ec::Curve;
use super::sha512::sha384;
use alloc::vec::Vec;

/// Parametres secp384r1 (coordonnees sur 48 octets).
fn curve() -> Curve {
    Curve::new(
        &hex("fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff"),
        &hex("ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973"),
        &hex("aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7"),
        &hex("3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f"),
        &hex("b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef"),
        48,
    )
}

/// Verifie une signature ECDSA P-384 (SHA-384).
/// `pubkey` : point non compresse (97 octets). `r`/`s` : entiers bruts.
pub fn verify_ecdsa_sha384(pubkey: &[u8], msg: &[u8], r: &[u8], s: &[u8]) -> bool {
    let hash = sha384(msg);
    curve().verify_ecdsa(pubkey, &hash, r, s)
}

/// Auto-test : verifie que G est sur la courbe et que n*G vaut l'infini.
pub fn selftest() -> Result<(), &'static str> {
    let c = curve();
    if !c.generator_on_curve() { return Err("G hors courbe"); }
    if !c.order_is_valid() { return Err("ordre incorrect"); }
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
