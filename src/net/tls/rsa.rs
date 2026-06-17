//! RSA — verification de signature uniquement (PKCS#1 v1.5 et RSA-PSS, SHA-256).
//!
//! Sert a verifier les signatures des certificats X.509 (sha256WithRSAEncryption)
//! et le message CertificateVerify de TLS 1.3 (rsa_pss_rsae_sha256).

use super::bignum::BigUint;
use super::sha256::{sha256, Sha256, HASH_LEN};
use alloc::vec::Vec;

/// Cle publique RSA.
pub struct RsaPubKey {
    pub n: BigUint,
    pub e: BigUint,
    pub n_bytes: usize, // taille du module en octets
}

impl RsaPubKey {
    pub fn new(modulus: &[u8], exponent: &[u8]) -> RsaPubKey {
        // Retire un eventuel octet de signe ASN.1 (0x00 de tete).
        let m = strip_leading_zeros(modulus);
        RsaPubKey {
            n: BigUint::from_bytes_be(m),
            e: BigUint::from_bytes_be(exponent),
            n_bytes: m.len(),
        }
    }

    // Operation publique : sig^e mod n, renvoyee zero-paddee a n_bytes.
    fn raw(&self, sig: &[u8]) -> Vec<u8> {
        let s = BigUint::from_bytes_be(sig);
        let m = s.modpow(&self.e, &self.n);
        let mut out = m.to_bytes_be();
        if out.len() < self.n_bytes {
            let mut padded = alloc::vec![0u8; self.n_bytes - out.len()];
            padded.extend_from_slice(&out);
            out = padded;
        }
        out
    }
}

fn strip_leading_zeros(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i + 1 < b.len() && b[i] == 0 { i += 1; }
    &b[i..]
}

// DigestInfo DER pour SHA-256.
const SHA256_PREFIX: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
    0x05, 0x00, 0x04, 0x20,
];

/// Verifie une signature PKCS#1 v1.5 (SHA-256) sur le message `msg`.
pub fn verify_pkcs1_sha256(key: &RsaPubKey, msg: &[u8], sig: &[u8]) -> bool {
    if sig.len() != key.n_bytes { return false; }
    let em = key.raw(sig);
    let hash = sha256(msg);
    // EM attendu : 00 01 FF..FF 00 || prefix || hash
    let t_len = SHA256_PREFIX.len() + HASH_LEN;
    if em.len() < t_len + 11 { return false; }
    let ps_len = em.len() - t_len - 3;
    if em[0] != 0x00 || em[1] != 0x01 { return false; }
    for i in 0..ps_len {
        if em[2 + i] != 0xff { return false; }
    }
    if em[2 + ps_len] != 0x00 { return false; }
    let t = &em[3 + ps_len..];
    if t[..SHA256_PREFIX.len()] != SHA256_PREFIX { return false; }
    t[SHA256_PREFIX.len()..] == hash
}

// MGF1 avec SHA-256.
fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut counter: u32 = 0;
    while out.len() < len {
        let mut h = Sha256::new();
        h.update(seed);
        h.update(&counter.to_be_bytes());
        let block = h.finalize();
        let need = (len - out.len()).min(HASH_LEN);
        out.extend_from_slice(&block[..need]);
        counter += 1;
    }
    out
}

/// Verifie une signature RSA-PSS (SHA-256, MGF1-SHA256, salt = 32) sur `msg`.
pub fn verify_pss_sha256(key: &RsaPubKey, msg: &[u8], sig: &[u8]) -> bool {
    if sig.len() != key.n_bytes { return false; }
    let mhash = sha256(msg);
    let em = key.raw(sig);
    let em_len = key.n_bytes;
    let h_len = HASH_LEN;
    let s_len = HASH_LEN; // salt length = hash length (rsa_pss_rsae_sha256)

    // modBits = bit_len(n) ; emBits = modBits - 1.
    let mod_bits = key.n.bit_len();
    let em_bits = mod_bits - 1;
    // emLen attendu = ceil(emBits/8).
    let expected_em_len = (em_bits + 7) / 8;
    // raw() rend n_bytes octets ; ajuste si emLen < n_bytes (bit de poids fort nul).
    let em = if expected_em_len < em_len {
        em[em_len - expected_em_len..].to_vec()
    } else {
        em
    };
    let em_len = expected_em_len;

    if em_len < h_len + s_len + 2 { return false; }
    if em[em_len - 1] != 0xbc { return false; }

    let db_len = em_len - h_len - 1;
    let masked_db = &em[..db_len];
    let h = &em[db_len..db_len + h_len];

    let db_mask = mgf1(h, db_len);
    let mut db = alloc::vec![0u8; db_len];
    for i in 0..db_len {
        db[i] = masked_db[i] ^ db_mask[i];
    }
    // Met a zero les bits de tete (8*emLen - emBits).
    let zero_bits = 8 * em_len - em_bits;
    if zero_bits > 0 {
        db[0] &= 0xff >> zero_bits;
    }
    // DB = PS(0x00) || 0x01 || salt
    let ps_len = db_len - s_len - 1;
    for i in 0..ps_len {
        if db[i] != 0x00 { return false; }
    }
    if db[ps_len] != 0x01 { return false; }
    let salt = &db[ps_len + 1..];

    // M' = (0x00 * 8) || mHash || salt ; H' = SHA256(M')
    let mut m_prime = Vec::with_capacity(8 + h_len + s_len);
    m_prime.extend_from_slice(&[0u8; 8]);
    m_prime.extend_from_slice(&mhash);
    m_prime.extend_from_slice(salt);
    let h_prime = sha256(&m_prime);
    h_prime.as_slice() == h
}

fn hex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i + 1 < b.len() {
        let h = |c: u8| match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => 0,
        };
        out.push((h(b[i]) << 4) | h(b[i + 1]));
        i += 2;
    }
    out
}

// Vecteur de reference : cle RSA-2048 et signatures (PKCS#1 v1.5 et PSS) du
// message "hello bouchaud tls", produites par OpenSSL.
const N_HEX: &str = "af48a409c6500d6e39c8073985fbf85ead91723cd4152c85b99fdb7be0a5bbd123b6ccdad57ea01a1ecff8a3917a0030eb52d4f2f83dd61d83c408a98c94e7528aeabc66a4f7b9a2ef5d3d4ddb512f44d949f44853c2e28fbdf65524639fba185981709dd04a82c35297eac428b70657a18a4de734de37ac9d22b5eaa271ff78bbfd9845e25137368516a007577a8bfa36fdaedb11ad0fc4552df0914c2c5619f406c18cfa5825cff2c743d3bc4490abb830d5a6a087a5aabe4ea8c0511077c7fb9e8853168f5149adbcafb3f4725b2387d6b7b4f1ce50b0327906e6c572f122dd455e4e9921adf89a19d7c951c1040233a774501bc208cd6cc94e190dda4337";
const SIG_PKCS1_HEX: &str = "ad1ec136311b21bed1114abd2e8bb3f72d96d6df1b3a321588f7d14dc992d350dbdd3197fce693ae0bb303227d05396cd0437672b6f3b57f89cec21c9a8a9865bae3495b6d636d693df896bf6f036b73d1aab2218897e6ff2c53181f3a5bb2f59b6480651a4d5e6c99639eac8049c16d81b029fe3379cae322069066cc1a9cf473528031a8bf5dc5de5224d2bae94dc1baca1ee9914b2c74f93d66fb4eb115a4167675117027309decdd95bcfd5359d8b082ec1978cf9fc13395d4602308e2992d9eaa747bd36be071c6133f3be7a1ad0564a2923c7f67292598e6d060155a0b164f6346ce6a602cd03e927fc90058aa5b3f7e7f3244028a6a0ae10514df5c00";
const SIG_PSS_HEX: &str = "8924549c8c5b4cb81a616eb462d8de267c279396d72c50698069bf94d1f9f4165f7b7ea94445ba7a57c483fdf368cb2e08ab14965922a9d3d5b9c805608a07b3e2f264e54daa018d2298db70090353ca92973bc34041586a89ed77756fc5b72d46de2c5982472c37dd0efffbfca00d53dd40b0e1423bfa713e65dbd260608358843cdb66831029a87e366662949131b13095143d08e934d808e3b7c5cd27a8c2102f64c91151449ba2fa4dd2f928a4cd1acb7ec250bf7dc4dc61b85c18296b53e25acca97568bfe33082bef1f402aff20ca63af9add000259626aafc58517ed2fab94f4e4dac3c6ab861ac481cce25c9cb289661690d5e315c5fcc5e8f22c4a6";

/// Auto-test : verification PKCS#1 v1.5 et PSS contre des signatures OpenSSL.
pub fn selftest() -> Result<(), &'static str> {
    let n = hex(N_HEX);
    let e = [0x01, 0x00, 0x01];
    let key = RsaPubKey::new(&n, &e);
    let msg = b"hello bouchaud tls";

    let sig1 = hex(SIG_PKCS1_HEX);
    if !verify_pkcs1_sha256(&key, msg, &sig1) { return Err("pkcs1 v1.5"); }
    let mut bad = sig1.clone();
    bad[100] ^= 1;
    if verify_pkcs1_sha256(&key, msg, &bad) { return Err("pkcs1 accepte sig fausse"); }

    let sigp = hex(SIG_PSS_HEX);
    if !verify_pss_sha256(&key, msg, &sigp) { return Err("pss"); }
    let mut badm = msg.to_vec();
    badm[0] ^= 1;
    if verify_pss_sha256(&key, &badm, &sigp) { return Err("pss accepte mauvais msg"); }
    Ok(())
}
