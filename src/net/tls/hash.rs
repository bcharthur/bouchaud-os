//! Abstraction de hash/HMAC/HKDF pour les suites TLS 1.3 SHA-256 et SHA-384.

use super::{sha256, sha512};
use alloc::vec;
use alloc::vec::Vec;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HashAlg { Sha256, Sha384 }

impl HashAlg {
    pub fn len(self) -> usize { match self { HashAlg::Sha256 => 32, HashAlg::Sha384 => 48 } }
    fn block_len(self) -> usize { match self { HashAlg::Sha256 => 64, HashAlg::Sha384 => 128 } }
    pub fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            HashAlg::Sha256 => sha256::sha256(data).to_vec(),
            HashAlg::Sha384 => sha512::sha384(data).to_vec(),
        }
    }
}

pub fn hmac(alg: HashAlg, key: &[u8], msg: &[u8]) -> Vec<u8> {
    let bl = alg.block_len();
    let mut k = vec![0u8; bl];
    if key.len() > bl {
        let d = alg.digest(key);
        k[..d.len()].copy_from_slice(&d);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = vec![0x36u8; bl];
    let mut opad = vec![0x5cu8; bl];
    for i in 0..bl { ipad[i] ^= k[i]; opad[i] ^= k[i]; }
    let mut inner = ipad;
    inner.extend_from_slice(msg);
    let inner_digest = alg.digest(&inner);
    let mut outer = opad;
    outer.extend_from_slice(&inner_digest);
    alg.digest(&outer)
}

pub fn hkdf_extract(alg: HashAlg, salt: &[u8], ikm: &[u8]) -> Vec<u8> { hmac(alg, salt, ikm) }

pub fn hkdf_expand(alg: HashAlg, prk: &[u8], info: &[u8], length: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(length);
    let mut t: Vec<u8> = Vec::new();
    let mut counter: u8 = 1;
    while out.len() < length {
        let mut input = Vec::with_capacity(t.len() + info.len() + 1);
        input.extend_from_slice(&t);
        input.extend_from_slice(info);
        input.push(counter);
        let block = hmac(alg, prk, &input);
        t = block.clone();
        let need = (length - out.len()).min(block.len());
        out.extend_from_slice(&block[..need]);
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
