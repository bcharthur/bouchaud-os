//! CSPRNG simple pour les aleas du handshake TLS.
//!
//! Source d'entropie : RDRAND si disponible, sinon TSC melange. Le flux est
//! genere par ChaCha-like via SHA-256 en mode compteur (HASH-DRBG minimal).
//!
//! Note : sans vraie source materielle auditee, la qualite depend de RDRAND.

use super::sha256::sha256;
use crate::arch::x86_64::cpu;
use core::arch::x86_64::__cpuid;

static mut STATE: [u8; 32] = [0u8; 32];
static mut COUNTER: u64 = 0;
static mut SEEDED: bool = false;

fn has_rdrand() -> bool {
    let leaf1 = __cpuid(1);
    leaf1.ecx & (1 << 30) != 0
}

fn rdrand64() -> Option<u64> {
    if !has_rdrand() { return None; }
    let mut val: u64;
    let ok: u8;
    unsafe {
        core::arch::asm!(
            "rdrand {0}",
            "setc {1}",
            out(reg) val,
            out(reg_byte) ok,
            options(nomem, nostack),
        );
    }
    if ok != 0 { Some(val) } else { None }
}

fn seed() {
    let mut s = [0u8; 64];
    // Plusieurs lectures TSC + RDRAND melangees.
    for i in 0..8 {
        let t = cpu::rdtsc() ^ (i as u64).wrapping_mul(0x9e3779b97f4a7c15);
        s[i * 8..i * 8 + 8].copy_from_slice(&t.to_le_bytes());
    }
    if let Some(r) = rdrand64() {
        for i in 0..8 { s[i] ^= (r >> (i * 8)) as u8; }
    }
    if let Some(r) = rdrand64() {
        for i in 0..8 { s[32 + i] ^= (r >> (i * 8)) as u8; }
    }
    let d = sha256(&s);
    unsafe {
        STATE = d;
        SEEDED = true;
        COUNTER = 0;
    }
}

/// Remplit `out` d'octets pseudo-aleatoires.
pub fn fill(out: &mut [u8]) {
    unsafe {
        if !SEEDED { seed(); }
        let mut i = 0;
        while i < out.len() {
            // bloc = SHA-256(STATE || counter)
            let mut input = [0u8; 40];
            input[..32].copy_from_slice(&STATE);
            input[32..40].copy_from_slice(&COUNTER.to_le_bytes());
            COUNTER = COUNTER.wrapping_add(1);
            // melange RDRAND a chaque bloc si dispo.
            if let Some(r) = rdrand64() {
                for k in 0..8 { input[k] ^= (r >> (k * 8)) as u8; }
            }
            let block = sha256(&input);
            let n = (out.len() - i).min(32);
            out[i..i + n].copy_from_slice(&block[..n]);
            i += n;
        }
    }
}

/// Renvoie 32 octets aleatoires.
pub fn random32() -> [u8; 32] {
    let mut out = [0u8; 32];
    fill(&mut out);
    out
}
