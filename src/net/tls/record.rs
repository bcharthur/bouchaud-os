//! Couche record TLS 1.3 : protection AEAD (AES-128-GCM) et schedule de cles.

use super::sha256::{self, hkdf_expand_label, hkdf_extract, derive_secret, hmac, HASH_LEN};
use super::gcm;
use alloc::vec::Vec;

pub const CT_CHANGE_CIPHER_SPEC: u8 = 20;
pub const CT_ALERT: u8 = 21;
pub const CT_HANDSHAKE: u8 = 22;
pub const CT_APPLICATION_DATA: u8 = 23;

const KEY_LEN: usize = 16; // AES-128-GCM

/// Cles directionnelles (un sens du flux) + numero de sequence.
pub struct DirKeys {
    key: Vec<u8>,
    iv: [u8; 12],
    seq: u64,
}

impl DirKeys {
    /// Derive cle+iv depuis un traffic secret.
    pub fn new(secret: &[u8]) -> DirKeys {
        let key = hkdf_expand_label(secret, b"key", &[], KEY_LEN);
        let iv_v = hkdf_expand_label(secret, b"iv", &[], 12);
        let mut iv = [0u8; 12];
        iv.copy_from_slice(&iv_v);
        DirKeys { key, iv, seq: 0 }
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
        let tag = gcm::seal(&self.key, &nonce, &header, &mut inner);
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
        gcm::open(&self.key, &nonce, header, &mut buf, &tag).ok()?;
        self.seq += 1;
        // Retire le padding (zeros) ; le dernier octet non nul = type interne.
        while let Some(&0) = buf.last() {
            buf.pop();
        }
        let inner_type = buf.pop()?;
        Some((inner_type, buf))
    }
}

/// Schedule de cles TLS 1.3 (suite *_SHA256).
pub struct KeySchedule {
    handshake_secret: [u8; HASH_LEN],
    pub client_hs: [u8; HASH_LEN],
    pub server_hs: [u8; HASH_LEN],
}

fn zeros() -> [u8; HASH_LEN] { [0u8; HASH_LEN] }

impl KeySchedule {
    /// Calcule les secrets de handshake a partir du secret ECDHE partage et du
    /// hash de transcript (ClientHello..ServerHello).
    pub fn derive_handshake(shared: &[u8], transcript_ch_sh: &[u8]) -> KeySchedule {
        let early = hkdf_extract(&zeros(), &zeros());
        let empty_hash = sha256::sha256(b"");
        let derived = derive_secret(&early, b"derived", &empty_hash);
        let handshake_secret = hkdf_extract(&derived, shared);
        let client_hs = derive_secret(&handshake_secret, b"c hs traffic", transcript_ch_sh);
        let server_hs = derive_secret(&handshake_secret, b"s hs traffic", transcript_ch_sh);
        KeySchedule { handshake_secret, client_hs, server_hs }
    }

    /// Calcule les secrets applicatifs a partir du transcript jusqu'a (server) Finished.
    pub fn derive_application(&self, transcript_through_sf: &[u8]) -> ([u8; HASH_LEN], [u8; HASH_LEN]) {
        let empty_hash = sha256::sha256(b"");
        let derived = derive_secret(&self.handshake_secret, b"derived", &empty_hash);
        let master = hkdf_extract(&derived, &zeros());
        let client_ap = derive_secret(&master, b"c ap traffic", transcript_through_sf);
        let server_ap = derive_secret(&master, b"s ap traffic", transcript_through_sf);
        (client_ap, server_ap)
    }
}

/// Cle de Finished derivee d'un traffic secret.
pub fn finished_key(traffic_secret: &[u8]) -> Vec<u8> {
    hkdf_expand_label(traffic_secret, b"finished", &[], HASH_LEN)
}

/// verify_data = HMAC(finished_key, transcript_hash).
pub fn finished_verify(traffic_secret: &[u8], transcript_hash: &[u8]) -> [u8; HASH_LEN] {
    let fk = finished_key(traffic_secret);
    hmac(&fk, transcript_hash)
}
