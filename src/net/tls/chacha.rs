//! ChaCha20-Poly1305 AEAD (RFC 8439), pour la suite TLS_CHACHA20_POLY1305_SHA256.
//!
//! Poly1305 utilise l'arithmetique modulaire de `bignum` (mod 2^130-5) : simple
//! et verifiable, suffisant car cette suite n'est choisie que si le serveur la
//! prefere a AES-GCM.

use super::bignum::BigUint;
use alloc::vec::Vec;

// --- ChaCha20 ---

fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(7);
}

fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut s = [0u32; 16];
    s[0] = 0x61707865; s[1] = 0x3320646e; s[2] = 0x79622d32; s[3] = 0x6b206574;
    for i in 0..8 {
        s[4 + i] = u32::from_le_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
    }
    s[12] = counter;
    for i in 0..3 {
        s[13 + i] = u32::from_le_bytes([nonce[4 * i], nonce[4 * i + 1], nonce[4 * i + 2], nonce[4 * i + 3]]);
    }
    let mut w = s;
    for _ in 0..10 {
        quarter_round(&mut w, 0, 4, 8, 12);
        quarter_round(&mut w, 1, 5, 9, 13);
        quarter_round(&mut w, 2, 6, 10, 14);
        quarter_round(&mut w, 3, 7, 11, 15);
        quarter_round(&mut w, 0, 5, 10, 15);
        quarter_round(&mut w, 1, 6, 11, 12);
        quarter_round(&mut w, 2, 7, 8, 13);
        quarter_round(&mut w, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        let v = w[i].wrapping_add(s[i]);
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

// Chiffre/dechiffre `data` en place (XOR keystream), compteur initial `counter`.
fn chacha20_xor(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    let mut block_counter = counter;
    let mut i = 0;
    while i < data.len() {
        let ks = chacha20_block(key, block_counter, nonce);
        let n = (data.len() - i).min(64);
        for j in 0..n { data[i + j] ^= ks[j]; }
        block_counter = block_counter.wrapping_add(1);
        i += 64;
    }
}

// --- Poly1305 (mod 2^130 - 5) ---

fn poly1305_prime() -> BigUint {
    // 2^130 - 5
    let mut bytes = [0u8; 17];
    bytes[0] = 0x03; // 2^130 = 0x4_0000...(17 octets) ; -5 -> 0x3ff...fb
    for b in bytes.iter_mut().skip(1) { *b = 0xff; }
    bytes[16] = 0xfb;
    BigUint::from_bytes_be(&bytes)
}

fn le_to_big(bytes: &[u8]) -> BigUint {
    let mut be: Vec<u8> = bytes.to_vec();
    be.reverse();
    BigUint::from_bytes_be(&be)
}

fn big_to_le16(v: &BigUint) -> [u8; 16] {
    let be = v.to_bytes_be();
    let mut out = [0u8; 16];
    // be est big-endian ; on prend les 16 octets de poids faible.
    let take = be.len().min(16);
    for i in 0..take {
        out[i] = be[be.len() - 1 - i];
    }
    out
}

fn poly1305_mac(msg: &[u8], key: &[u8; 32]) -> [u8; 16] {
    // r = key[0..16] clampe ; s = key[16..32].
    let mut r_bytes = [0u8; 16];
    r_bytes.copy_from_slice(&key[0..16]);
    r_bytes[3] &= 15; r_bytes[7] &= 15; r_bytes[11] &= 15; r_bytes[15] &= 15;
    r_bytes[4] &= 252; r_bytes[8] &= 252; r_bytes[12] &= 252;
    let r = le_to_big(&r_bytes);
    let s = le_to_big(&key[16..32]);
    let p = poly1305_prime();

    let mut acc = BigUint::zero();
    let mut i = 0;
    while i < msg.len() {
        let n = (msg.len() - i).min(16);
        // bloc en little-endian avec un octet 0x01 ajoute en tete (poids fort).
        let mut blk = Vec::with_capacity(n + 1);
        blk.extend_from_slice(&msg[i..i + n]);
        blk.push(0x01);
        let m = le_to_big(&blk);
        acc = acc.add(&m).mul(&r).rem(&p);
        i += 16;
    }
    let tag_big = acc.add(&s);
    big_to_le16(&tag_big)
}

fn pad16(len: usize) -> usize { (16 - (len % 16)) % 16 }

/// Genere la cle Poly1305 a usage unique (premier bloc keystream, compteur 0).
fn poly_key(key: &[u8; 32], nonce: &[u8; 12]) -> [u8; 32] {
    let block = chacha20_block(key, 0, nonce);
    let mut otk = [0u8; 32];
    otk.copy_from_slice(&block[..32]);
    otk
}

fn mac_data(aad: &[u8], cipher: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend_from_slice(aad);
    m.extend(core::iter::repeat(0u8).take(pad16(aad.len())));
    m.extend_from_slice(cipher);
    m.extend(core::iter::repeat(0u8).take(pad16(cipher.len())));
    m.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    m.extend_from_slice(&(cipher.len() as u64).to_le_bytes());
    m
}

/// Chiffre `buf` en place et renvoie le tag 16 octets (IV 96 bits).
pub fn seal(key: &[u8], iv: &[u8; 12], aad: &[u8], buf: &mut Vec<u8>) -> [u8; 16] {
    let mut k = [0u8; 32];
    k.copy_from_slice(&key[..32]);
    let otk = poly_key(&k, iv);
    chacha20_xor(&k, 1, iv, buf);
    let md = mac_data(aad, buf);
    poly1305_mac(&md, &otk)
}

/// Dechiffre `buf` en place apres verification du tag. Err si auth echoue.
pub fn open(key: &[u8], iv: &[u8; 12], aad: &[u8], buf: &mut Vec<u8>, tag: &[u8; 16]) -> Result<(), ()> {
    let mut k = [0u8; 32];
    k.copy_from_slice(&key[..32]);
    let otk = poly_key(&k, iv);
    let md = mac_data(aad, buf);
    let expected = poly1305_mac(&md, &otk);
    let mut diff = 0u8;
    for i in 0..16 { diff |= expected[i] ^ tag[i]; }
    if diff != 0 { return Err(()); }
    chacha20_xor(&k, 1, iv, buf);
    Ok(())
}

/// Auto-test : vecteur AEAD de la RFC 8439 section 2.8.2.
pub fn selftest() -> Result<(), &'static str> {
    let key: [u8; 32] = [
        0x80,0x81,0x82,0x83,0x84,0x85,0x86,0x87,0x88,0x89,0x8a,0x8b,0x8c,0x8d,0x8e,0x8f,
        0x90,0x91,0x92,0x93,0x94,0x95,0x96,0x97,0x98,0x99,0x9a,0x9b,0x9c,0x9d,0x9e,0x9f,
    ];
    let nonce: [u8; 12] = [0x07,0x00,0x00,0x00,0x40,0x41,0x42,0x43,0x44,0x45,0x46,0x47];
    let aad: [u8; 12] = [0x50,0x51,0x52,0x53,0xc0,0xc1,0xc2,0xc3,0xc4,0xc5,0xc6,0xc7];
    let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

    let mut buf: Vec<u8> = pt.to_vec();
    let tag = seal(&key, &nonce, &aad, &mut buf);
    let want_tag = [0x1a,0xe1,0x0b,0x59,0x4f,0x09,0xe2,0x6a,0x7e,0x90,0x2e,0xcb,0xd0,0x60,0x06,0x91];
    if tag != want_tag { return Err("chacha20poly1305 tag"); }
    // premiers octets du ciphertext attendu
    let want_ct_head = [0xd3,0x1a,0x8d,0x34,0x64,0x8e,0x60,0xdb];
    if buf[..8] != want_ct_head { return Err("chacha20 ciphertext"); }

    // round-trip
    let mut dec = buf.clone();
    if open(&key, &nonce, &aad, &mut dec, &tag).is_err() { return Err("chacha open auth"); }
    if dec != pt { return Err("chacha open pt"); }
    let mut bad = tag; bad[0] ^= 1;
    let mut dec2 = buf.clone();
    if open(&key, &nonce, &aad, &mut dec2, &bad).is_ok() { return Err("chacha tag falsifie accepte"); }
    Ok(())
}
