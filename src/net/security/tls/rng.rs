//! CSPRNG simple pour les aleas du handshake TLS.
//!
//! Source d'entropie : RDRAND si disponible, sinon TSC melange. Le flux est
//! genere par ChaCha-like via SHA-256 en mode compteur (HASH-DRBG minimal).
//!
//! Note : sans vraie source materielle auditee, la qualite depend de RDRAND.

use super::sha256::sha256;
use crate::arch::x86_64::cpu;
use core::arch::x86_64::__cpuid;

/// L'etat du generateur.
///
/// Il vivait dans trois `static mut` sans verrou, et le gros verrou etait tout
/// ce qui les serialisait : `read` et le handshake TLS s'executaient sous BKL,
/// donc jamais deux a la fois.
///
/// Ce n'etait pas seulement une course de donnees. `COUNTER` est ce qui rend
/// deux blocs differents : deux CPU qui le lisent avant que l'un ne l'ait
/// incremente calculent `SHA-256(STATE || meme_compteur)` et obtiennent LE MEME
/// bloc. Pour des aleas de handshake TLS, c'est une repetition d'alea, pas une
/// approximation de compteur. `SEEDED` a le meme defaut a l'amorce : deux
/// entrees simultanees peuvent semer deux fois, ou lire un etat non seme.
///
/// L'etat porte donc son verrou, et ne depend plus de qui l'appelle.
struct Generateur {
    etat: [u8; 32],
    compteur: u64,
    seme: bool,
}

static GENERATEUR: crate::kernel::sync::SpinLock<Generateur> =
    crate::kernel::sync::SpinLock::new(Generateur {
        etat: [0u8; 32],
        compteur: 0,
        seme: false,
    });

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

/// Seme `generateur`. L'appelant tient deja le verrou.
fn seed(generateur: &mut Generateur) {
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
    generateur.etat = sha256(&s);
    generateur.seme = true;
    generateur.compteur = 0;
}

/// Remplit `out` d'octets pseudo-aleatoires.
pub fn fill(out: &mut [u8]) {
    // Le verrou couvre la LECTURE et l'INCREMENT du compteur : c'est leur
    // atomicite conjointe qui garantit qu'aucun bloc ne sort deux fois. Le
    // tenir pendant les SHA-256 n'ajoute pas d'attente notable -- le calcul
    // est borne, sans allocation, et le seul autre porteur possible fait la
    // meme chose.
    let mut generateur = GENERATEUR.lock();
    if !generateur.seme {
        seed(&mut generateur);
    }
    let mut i = 0;
    while i < out.len() {
        // bloc = SHA-256(etat || compteur)
        let mut input = [0u8; 40];
        input[..32].copy_from_slice(&generateur.etat);
        input[32..40].copy_from_slice(&generateur.compteur.to_le_bytes());
        generateur.compteur = generateur.compteur.wrapping_add(1);
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

/// Renvoie 32 octets aleatoires.
pub fn random32() -> [u8; 32] {
    let mut out = [0u8; 32];
    fill(&mut out);
    out
}
