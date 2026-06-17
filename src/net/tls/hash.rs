//! Hash/HMAC/HKDF generiques pour les suites TLS 1.3 SHA-256 et SHA-384.

use super::{sha256, sha512};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HashAlg {
    Sha256,
    Sha384,
}

impl HashAlg {
    pub fn len(self) -> usize {
        match self {
            HashAlg::Sha256 => 32,
            HashAlg::Sha384 => 48,
        }
    }

    fn block_len(self) -> usize {
        match self {
            HashAlg::Sha256 => 64,
            HashAlg::Sha384 => 128,
        }
    }
}

pub fn digest(alg: HashAlg, data: &[u8]) -> Vec<u8> {
    match alg {
        HashAlg::Sha256 => sha256::sha256(data).to_vec(),
        HashAlg::Sha384 => sha512::sha384(data).to_vec(),
    }
}

pub fn hmac(alg: HashAlg, key: &[u8], msg: &[u8]) -> Vec<u8> {
    let block_len = alg.block_len();
    let hash_len = alg.len();
    let mut k = Vec::new();
    k.resize(block_len, 0);

    if key.len() > block_len {
        let d = digest(alg, key);
        k[..hash_len].copy_from_slice(&d[..hash_len]);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = Vec::new();
    ipad.resize(block_len, 0x36);
    let mut opad = Vec::new();
    opad.resize(block_len, 0x5c);
    for i in 0..block_len {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Vec::with_capacity(block_len + msg.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(msg);
    let inner_digest = digest(alg, &inner);

    let mut outer = Vec::with_capacity(block_len + inner_digest.len());
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_digest);
    digest(alg, &outer)
}

pub fn hkdf_extract(alg: HashAlg, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
    hmac(alg, salt, ikm)
}

pub fn hkdf_expand(alg: HashAlg, prk: &[u8], info: &[u8], length: usize) -> Vec<u8> {
    let hash_len = alg.len();
    let mut out = Vec::with_capacity(length);
    let mut t: Vec<u8> = Vec::new();
    let mut counter: u8 = 1;
    while out.len() < length {
        let mut input = Vec::with_capacity(t.len() + info.len() + 1);
        input.extend_from_slice(&t);
        input.extend_from_slice(info);
        input.push(counter);
        t = hmac(alg, prk, &input);
        let need = (length - out.len()).min(hash_len);
        out.extend_from_slice(&t[..need]);
        counter = counter.wrapping_add(1);
    }
    out
}

pub fn hkdf_expand_label(alg: HashAlg, secret: &[u8], label: &[u8], context: &[u8], length: usize) -> Vec<u8> {
    let mut full_label = Vec::with_capacity(6 + label.len());
    full_label.extend_from_slice(b"tls13 ");
    full_label.extend_from_slice(label);

    let mut info = Vec::with_capacity(2 + 1 + full_label.len() + 1 + context.len());
    info.extend_from_slice(&(length as u16).to_be_bytes());
    info.push(full_label.len() as u8);
    info.extend_from_slice(&full_label);
    info.push(context.len() as u8);
    info.extend_from_slice(context);
    hkdf_expand(alg, secret, &info, length)
}

pub fn derive_secret(alg: HashAlg, secret: &[u8], label: &[u8], transcript_hash: &[u8]) -> Vec<u8> {
    hkdf_expand_label(alg, secret, label, transcript_hash, alg.len())
}

pub fn empty_hash(alg: HashAlg) -> Vec<u8> {
    digest(alg, b"")
}

pub fn zeros(alg: HashAlg) -> Vec<u8> {
    let mut z = Vec::new();
    z.resize(alg.len(), 0);
    z
}

pub fn selftest() -> Result<(), &'static str> {
    let h = hmac(HashAlg::Sha256, b"key", b"The quick brown fox jumps over the lazy dog");
    let want = [
        0xf7,0xbc,0x83,0xf4,0x30,0x53,0x84,0x24,0xb1,0x32,0x98,0xe6,0xaa,0x6f,0xb1,0x43,
        0xef,0x4d,0x59,0xa1,0x49,0x46,0x17,0x59,0x97,0x47,0x9d,0xbc,0x2d,0x1a,0x3c,0xd8,
    ];
    if h.as_slice() != &want[..] { return Err("hmac-sha256"); }
    let h384 = hmac(HashAlg::Sha384, b"key", b"The quick brown fox jumps over the lazy dog");
    let want384 = [
        0xd7,0xf4,0x72,0x7e,0x2c,0x0b,0x39,0xae,0x0f,0x1e,0x40,0xcc,0x96,0xf6,0x02,0x42,
        0xd5,0xb7,0x80,0x18,0x41,0xce,0xa6,0xfc,0x59,0x2c,0x5d,0x3e,0x1a,0xe5,0x07,0x00,
        0x58,0x2a,0x96,0xcf,0x35,0xe1,0xe5,0x54,0x99,0x5f,0xe4,0xe0,0x33,0x81,0xc2,0x37,
    ];
    if h384.as_slice() != &want384[..] { return Err("hmac-sha384"); }
    Ok(())
}
