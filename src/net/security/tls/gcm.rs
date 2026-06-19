//! AES-GCM (mode AEAD), conforme a NIST SP 800-38D, pour IV de 96 bits.
//!
//! Utilise pour AES_128_GCM_SHA256 et AES_256_GCM_SHA384(... ici SHA256) de TLS 1.3.

use super::aes::Aes;
use alloc::vec::Vec;

/// Multiplication dans GF(2^128) selon la convention GCM (polynome x^128+x^7+x^2+x+1).
fn ghash_mul(y: &mut [u8; 16], h: &[u8; 16]) {
    let mut z = [0u8; 16];
    let mut v = *h;
    for i in 0..128 {
        let bit = (y[i / 8] >> (7 - (i % 8))) & 1;
        if bit == 1 {
            for j in 0..16 { z[j] ^= v[j]; }
        }
        let lsb = v[15] & 1;
        for j in (1..16).rev() {
            v[j] = (v[j] >> 1) | (v[j - 1] << 7);
        }
        v[0] >>= 1;
        if lsb == 1 { v[0] ^= 0xe1; }
    }
    *y = z;
}

/// GHASH(H, A, C) : authentifie AAD puis ciphertext, avec bloc de longueurs final.
fn ghash(h: &[u8; 16], aad: &[u8], cipher: &[u8]) -> [u8; 16] {
    let mut y = [0u8; 16];
    let feed = |data: &[u8], y: &mut [u8; 16]| {
        let mut i = 0;
        while i < data.len() {
            let mut block = [0u8; 16];
            let n = (data.len() - i).min(16);
            block[..n].copy_from_slice(&data[i..i + n]);
            for j in 0..16 { y[j] ^= block[j]; }
            ghash_mul(y, h);
            i += 16;
        }
    };
    feed(aad, &mut y);
    feed(cipher, &mut y);
    // bloc final : len(A) en bits (64) || len(C) en bits (64), big-endian.
    let mut lenblock = [0u8; 16];
    let alen = (aad.len() as u64) * 8;
    let clen = (cipher.len() as u64) * 8;
    lenblock[..8].copy_from_slice(&alen.to_be_bytes());
    lenblock[8..].copy_from_slice(&clen.to_be_bytes());
    for j in 0..16 { y[j] ^= lenblock[j]; }
    ghash_mul(&mut y, h);
    y
}

fn inc32(counter: &mut [u8; 16]) {
    let mut c = u32::from_be_bytes([counter[12], counter[13], counter[14], counter[15]]);
    c = c.wrapping_add(1);
    counter[12..].copy_from_slice(&c.to_be_bytes());
}

// Flux CTR : XOR le contenu de `data` avec le keystream a partir de `counter`.
fn gctr(aes: &Aes, counter: &mut [u8; 16], data: &mut [u8]) {
    let mut i = 0;
    while i < data.len() {
        let mut ks = *counter;
        aes.encrypt_block(&mut ks);
        let n = (data.len() - i).min(16);
        for j in 0..n { data[i + j] ^= ks[j]; }
        inc32(counter);
        i += 16;
    }
}

/// Chiffre `plaintext` (modifie en place vers ciphertext) avec IV 96 bits, renvoie le tag 16 octets.
pub fn seal(key: &[u8], iv: &[u8; 12], aad: &[u8], buf: &mut Vec<u8>) -> [u8; 16] {
    let aes = Aes::new(key);
    let mut h = [0u8; 16];
    aes.encrypt_block(&mut h);

    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(iv);
    j0[15] = 1;

    // E(J0) pour le tag.
    let mut ej0 = j0;
    aes.encrypt_block(&mut ej0);

    // CTR demarre a inc32(J0).
    let mut ctr = j0;
    inc32(&mut ctr);
    gctr(&aes, &mut ctr, buf);

    let s = ghash(&h, aad, buf);
    let mut tag = [0u8; 16];
    for j in 0..16 { tag[j] = s[j] ^ ej0[j]; }
    tag
}

/// Dechiffre `buf` (ciphertext sans tag) avec verification du tag. Renvoie Err si auth echoue.
pub fn open(key: &[u8], iv: &[u8; 12], aad: &[u8], buf: &mut Vec<u8>, tag: &[u8; 16]) -> Result<(), ()> {
    let aes = Aes::new(key);
    let mut h = [0u8; 16];
    aes.encrypt_block(&mut h);

    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(iv);
    j0[15] = 1;
    let mut ej0 = j0;
    aes.encrypt_block(&mut ej0);

    // Authentifie le ciphertext AVANT de dechiffrer.
    let s = ghash(&h, aad, buf);
    let mut expected = [0u8; 16];
    for j in 0..16 { expected[j] = s[j] ^ ej0[j]; }
    let mut diff = 0u8;
    for j in 0..16 { diff |= expected[j] ^ tag[j]; }
    if diff != 0 { return Err(()); }

    let mut ctr = j0;
    inc32(&mut ctr);
    gctr(&aes, &mut ctr, buf);
    Ok(())
}

/// Auto-test : vecteur de test NIST GCM (AES-128, test case 3).
pub fn selftest() -> Result<(), &'static str> {
    let key = [
        0xfe, 0xff, 0xe9, 0x92, 0x86, 0x65, 0x73, 0x1c, 0x6d, 0x6a, 0x8f, 0x94, 0x67, 0x30, 0x83, 0x08,
    ];
    let iv = [0xca, 0xfe, 0xba, 0xbe, 0xfa, 0xce, 0xdb, 0xad, 0xde, 0xca, 0xf8, 0x88];
    let pt = [
        0xd9, 0x31, 0x32, 0x25, 0xf8, 0x84, 0x06, 0xe5, 0xa5, 0x59, 0x09, 0xc5, 0xaf, 0xf5, 0x26, 0x9a,
        0x86, 0xa7, 0xa9, 0x53, 0x15, 0x34, 0xf7, 0xda, 0x2e, 0x4c, 0x30, 0x3d, 0x8a, 0x31, 0x8a, 0x72,
        0x1c, 0x3c, 0x0c, 0x95, 0x95, 0x68, 0x09, 0x53, 0x2f, 0xcf, 0x0e, 0x24, 0x49, 0xa6, 0xb5, 0x25,
        0xb1, 0x6a, 0xed, 0xf5, 0xaa, 0x0d, 0xe6, 0x57, 0xba, 0x63, 0x7b, 0x39, 0x1a, 0xaf, 0xd2, 0x55,
    ];
    let mut buf: Vec<u8> = pt.to_vec();
    let tag = seal(&key, &iv, &[], &mut buf);
    let want_ct = [
        0x42, 0x83, 0x1e, 0xc2, 0x21, 0x77, 0x74, 0x24, 0x4b, 0x72, 0x21, 0xb7, 0x84, 0xd0, 0xd4, 0x9c,
        0xe3, 0xaa, 0x21, 0x2f, 0x2c, 0x02, 0xa4, 0xe0, 0x35, 0xc1, 0x7e, 0x23, 0x29, 0xac, 0xa1, 0x2e,
        0x21, 0xd5, 0x14, 0xb2, 0x54, 0x66, 0x93, 0x1c, 0x7d, 0x8f, 0x6a, 0x5a, 0xac, 0x84, 0xaa, 0x05,
        0x1b, 0xa3, 0x0b, 0x39, 0x6a, 0x0a, 0xac, 0x97, 0x3d, 0x58, 0xe0, 0x91, 0x47, 0x3f, 0x59, 0x85,
    ];
    let want_tag = [
        0x4d, 0x5c, 0x2a, 0xf3, 0x27, 0xcd, 0x64, 0xa6, 0x2c, 0xf3, 0x5a, 0xbd, 0x2b, 0xa6, 0xfa, 0xb4,
    ];
    if buf != want_ct { return Err("gcm seal ct"); }
    if tag != want_tag { return Err("gcm seal tag"); }

    // open round-trip
    let mut dec = buf.clone();
    if open(&key, &iv, &[], &mut dec, &tag).is_err() { return Err("gcm open auth"); }
    if dec != pt { return Err("gcm open pt"); }
    // tag corrompu doit echouer
    let mut bad = tag; bad[0] ^= 1;
    let mut dec2 = want_ct.to_vec();
    if open(&key, &iv, &[], &mut dec2, &bad).is_ok() { return Err("gcm bad tag accepte"); }
    Ok(())
}
