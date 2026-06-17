//! Couche record TLS 1.3 : protection AEAD et schedule de cles.
//! Support stable : AES-128-GCM/SHA-256 et AES-256-GCM/SHA-384.

use super::{gcm, hash};
use alloc::vec::Vec;

pub const CT_CHANGE_CIPHER_SPEC: u8 = 20;
pub const CT_ALERT: u8 = 21;
pub const CT_HANDSHAKE: u8 = 22;
pub const CT_APPLICATION_DATA: u8 = 23;

pub const TLS_AES_128_GCM_SHA256_ID: u16 = 0x1301;
pub const TLS_AES_256_GCM_SHA384_ID: u16 = 0x1302;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CipherSuite {
    TlsAes128GcmSha256,
    TlsAes256GcmSha384,
}

impl CipherSuite {
    pub fn from_id(id: u16) -> Option<Self> {
        match id {
            TLS_AES_128_GCM_SHA256_ID => Some(CipherSuite::TlsAes128GcmSha256),
            TLS_AES_256_GCM_SHA384_ID => Some(CipherSuite::TlsAes256GcmSha384),
            _ => None,
        }
    }

    pub fn id(self) -> u16 {
        match self {
            CipherSuite::TlsAes128GcmSha256 => TLS_AES_128_GCM_SHA256_ID,
            CipherSuite::TlsAes256GcmSha384 => TLS_AES_256_GCM_SHA384_ID,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CipherSuite::TlsAes128GcmSha256 => "TLS_AES_128_GCM_SHA256",
            CipherSuite::TlsAes256GcmSha384 => "TLS_AES_256_GCM_SHA384",
        }
    }

    pub fn hash_alg(self) -> hash::HashAlg {
        match self {
            CipherSuite::TlsAes128GcmSha256 => hash::HashAlg::Sha256,
            CipherSuite::TlsAes256GcmSha384 => hash::HashAlg::Sha384,
        }
    }

    pub fn key_len(self) -> usize {
        match self {
            CipherSuite::TlsAes128GcmSha256 => 16,
            CipherSuite::TlsAes256GcmSha384 => 32,
        }
    }

    pub fn hash_len(self) -> usize {
        self.hash_alg().len()
    }
}

/// Cles directionnelles (un sens du flux) + numero de sequence.
pub struct DirKeys {
    suite: CipherSuite,
    key: Vec<u8>,
    iv: [u8; 12],
    seq: u64,
}

impl DirKeys {
    /// Derive cle+iv depuis un traffic secret.
    pub fn new(suite: CipherSuite, secret: &[u8]) -> DirKeys {
        let alg = suite.hash_alg();
        let key = hash::hkdf_expand_label(alg, secret, b"key", &[], suite.key_len());
        let iv_v = hash::hkdf_expand_label(alg, secret, b"iv", &[], 12);
        let mut iv = [0u8; 12];
        iv.copy_from_slice(&iv_v);
        DirKeys { suite, key, iv, seq: 0 }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = self.iv;
        let s = self.seq.to_be_bytes();
        for i in 0..8 {
            n[4 + i] ^= s[i];
        }
        n
    }

    /// Chiffre un message (type interne `inner_type`) en un record TLS complet.
    pub fn encrypt(&mut self, inner_type: u8, data: &[u8]) -> Vec<u8> {
        let mut inner = Vec::with_capacity(data.len() + 1);
        inner.extend_from_slice(data);
        inner.push(inner_type);
        let total = inner.len() + 16;
        let header = [
            CT_APPLICATION_DATA,
            0x03, 0x03,
            (total >> 8) as u8,
            total as u8,
        ];
        let nonce = self.nonce();
        let tag = match self.suite {
            CipherSuite::TlsAes128GcmSha256 | CipherSuite::TlsAes256GcmSha384 => {
                gcm::seal(&self.key, &nonce, &header, &mut inner)
            }
        };
        self.seq += 1;
        let mut out = Vec::with_capacity(5 + total);
        out.extend_from_slice(&header);
        out.extend_from_slice(&inner);
        out.extend_from_slice(&tag);
        out
    }

    /// Dechiffre le corps d'un record (ciphertext+tag), avec son en-tete (AAD).
    /// Renvoie (type interne, plaintext).
    pub fn decrypt(&mut self, header: &[u8; 5], body: &[u8]) -> Option<(u8, Vec<u8>)> {
        if body.len() < 16 { return None; }
        let split = body.len() - 16;
        let mut buf = body[..split].to_vec();
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&body[split..]);
        let nonce = self.nonce();
        let ok = match self.suite {
            CipherSuite::TlsAes128GcmSha256 | CipherSuite::TlsAes256GcmSha384 => {
                gcm::open(&self.key, &nonce, header, &mut buf, &tag).is_ok()
            }
        };
        if !ok { return None; }
        self.seq += 1;
        // Retire le padding (zeros) ; le dernier octet non nul = type interne.
        while let Some(&0) = buf.last() {
            buf.pop();
        }
        let inner_type = buf.pop()?;
        Some((inner_type, buf))
    }
}

/// Schedule de cles TLS 1.3.
pub struct KeySchedule {
    suite: CipherSuite,
    handshake_secret: Vec<u8>,
    pub client_hs: Vec<u8>,
    pub server_hs: Vec<u8>,
}

impl KeySchedule {
    /// Calcule les secrets de handshake a partir du secret ECDHE partage et du
    /// hash de transcript (ClientHello..ServerHello).
    pub fn derive_handshake(suite: CipherSuite, shared: &[u8], transcript_ch_sh: &[u8]) -> KeySchedule {
        let alg = suite.hash_alg();
        let zeros = hash::zeros(alg);
        let early = hash::hkdf_extract(alg, &zeros, &zeros);
        let empty_hash = hash::empty_hash(alg);
        let derived = hash::derive_secret(alg, &early, b"derived", &empty_hash);
        let handshake_secret = hash::hkdf_extract(alg, &derived, shared);
        let client_hs = hash::derive_secret(alg, &handshake_secret, b"c hs traffic", transcript_ch_sh);
        let server_hs = hash::derive_secret(alg, &handshake_secret, b"s hs traffic", transcript_ch_sh);
        KeySchedule { suite, handshake_secret, client_hs, server_hs }
    }

    /// Calcule les secrets applicatifs a partir du transcript jusqu'a (server) Finished.
    pub fn derive_application(&self, transcript_through_sf: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let alg = self.suite.hash_alg();
        let empty_hash = hash::empty_hash(alg);
        let derived = hash::derive_secret(alg, &self.handshake_secret, b"derived", &empty_hash);
        let zeros = hash::zeros(alg);
        let master = hash::hkdf_extract(alg, &derived, &zeros);
        let client_ap = hash::derive_secret(alg, &master, b"c ap traffic", transcript_through_sf);
        let server_ap = hash::derive_secret(alg, &master, b"s ap traffic", transcript_through_sf);
        (client_ap, server_ap)
    }
}

/// Cle de Finished derivee d'un traffic secret.
pub fn finished_key(suite: CipherSuite, traffic_secret: &[u8]) -> Vec<u8> {
    let alg = suite.hash_alg();
    hash::hkdf_expand_label(alg, traffic_secret, b"finished", &[], suite.hash_len())
}

/// verify_data = HMAC(finished_key, transcript_hash).
pub fn finished_verify(suite: CipherSuite, traffic_secret: &[u8], transcript_hash: &[u8]) -> Vec<u8> {
    let fk = finished_key(suite, traffic_secret);
    hash::hmac(suite.hash_alg(), &fk, transcript_hash)
}
