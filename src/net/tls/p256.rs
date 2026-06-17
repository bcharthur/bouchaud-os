//! NIST P-256 (secp256r1) — ECDH et verification ECDSA.
//!
//! Simple instance de la courbe de Weierstrass courte generique (`ec`) avec les
//! parametres secp256r1. Le hachage associe est SHA-256. Objectif : verifier les
//! signatures ecdsa_secp256r1_sha256 (certificats et CertificateVerify) et
//! supporter l'echange de cles ECDHE P-256.

use super::ec::Curve;
use super::sha256::sha256;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// Parametres secp256r1 (coordonnees sur 32 octets).
fn curve() -> Curve {
    Curve::new(
        // p = ffffffff00000001000000000000000000000000ffffffffffffffffffffffff
        &[
            0xff,0xff,0xff,0xff,0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
            0x00,0x00,0x00,0x00,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
        ],
        // n (ordre)
        &[
            0xff,0xff,0xff,0xff,0x00,0x00,0x00,0x00,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
            0xbc,0xe6,0xfa,0xad,0xa7,0x17,0x9e,0x84,0xf3,0xb9,0xca,0xc2,0xfc,0x63,0x25,0x51,
        ],
        // Gx
        &[
            0x6b,0x17,0xd1,0xf2,0xe1,0x2c,0x42,0x47,0xf8,0xbc,0xe6,0xe5,0x63,0xa4,0x40,0xf2,
            0x77,0x03,0x7d,0x81,0x2d,0xeb,0x33,0xa0,0xf4,0xa1,0x39,0x45,0xd8,0x98,0xc2,0x96,
        ],
        // Gy
        &[
            0x4f,0xe3,0x42,0xe2,0xfe,0x1a,0x7f,0x9b,0x8e,0xe7,0xeb,0x4a,0x7c,0x0f,0x9e,0x16,
            0x2b,0xce,0x33,0x57,0x6b,0x31,0x5e,0xce,0xcb,0xb6,0x40,0x68,0x37,0xbf,0x51,0xf5,
        ],
        // b
        &[
            0x5a,0xc6,0x35,0xd8,0xaa,0x3a,0x93,0xe7,0xb3,0xeb,0xbd,0x55,0x76,0x98,0x86,0xbc,
            0x65,0x1d,0x06,0xb0,0xcc,0x53,0xb0,0xf6,0x3b,0xce,0x3c,0x3e,0x27,0xd2,0x60,0x4b,
        ],
        32,
    )
}

/// Verifie une signature ECDSA P-256 (SHA-256).
/// `pubkey` : point non compresse (65 octets). `r`/`s` : entiers bruts.
pub fn verify_ecdsa_sha256(pubkey: &[u8], msg: &[u8], r: &[u8], s: &[u8]) -> bool {
    let hash = sha256(msg);
    curve().verify_ecdsa(pubkey, &hash, r, s)
}

/// ECDH P-256 : multiplie le point pair (65 oct non compresse) par le scalaire
/// prive (32 oct) ; renvoie la coordonnee X partagee (32 oct).
pub fn ecdh(private: &[u8; 32], peer_pubkey: &[u8]) -> Option<[u8; 32]> {
    let shared = curve().ecdh(private, peer_pubkey)?;
    if shared.len() != 32 { return None; }
    let mut out = [0u8; 32];
    out.copy_from_slice(&shared);
    Some(out)
}

/// Cle publique P-256 a partir d'un scalaire prive : 0x04||X||Y (65 oct).
pub fn derive_pubkey(private: &[u8; 32]) -> Vec<u8> {
    curve().derive_pubkey(private)
}

/// Auto-test : verifie que G est sur la courbe et un vecteur ECDSA connu.
pub fn selftest() -> Result<(), &'static str> {
    let c = curve();
    // y^2 == x^3 - 3x + b (mod p) pour G.
    if !c.generator_on_curve() { return Err("G hors courbe"); }

    // x(2G) connu (NIST).
    let x2 = c.double_g_x().ok_or("2G infini")?;
    let want = super::bignum::BigUint::from_bytes_be(&[
        0x7c,0xf2,0x7b,0x18,0x8d,0x03,0x4f,0x7e,0x8a,0x52,0x38,0x03,0x04,0xb5,0x1a,0xc3,
        0xc0,0x89,0x69,0xe2,0x77,0xf2,0x1b,0x35,0xa6,0x0b,0x48,0xfc,0x47,0x66,0x99,0x78,
    ]);
    if x2.cmp_pub(&want) != Ordering::Equal { return Err("2G incorrect"); }

    // Vecteur ECDSA P-256 / SHA-256 (FIPS 186-4 exemple "sample").
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
