//! Unconfirmed boots of the ACTIVE slot (BOUCHAUD_ECHECS_REPETES_V1).
//!
//! The A/B record bounds a CANDIDATE's attempts. Nothing bounded a broken
//! ACTIVE slot: it would be booted forever. This counter lives in META block
//! 2, apart from the A/B record (whose format does not change):
//!
//!   * the loader increments it, durably, BEFORE booting the active slot
//!     (raison Actif); a rollback resets it (the old system starts afresh);
//!   * the kernel resets it when the system supervisor confirms health;
//!   * at SEUIL consecutive unconfirmed boots the loader stops booting the
//!     active slot and boots RECOVERY instead.
//!
//! Layout (block 2, first 16 bytes): "BOEC" magic, u32 counter, u32
//! reserved (0), u32 CRC32 of bytes 0..12. Anything else reads as 0: an
//! unreadable counter must never, by itself, send a machine to recovery.

use crate::ab::crc32;

pub const OCTETS: usize = 16;
/// META block holding the counter (blocks 0 and 1 are the A/B copies).
pub const BLOC: u64 = 2;
/// Consecutive unconfirmed active boots before recovery.
pub const SEUIL: u32 = 3;

pub fn encode(n: u32) -> [u8; OCTETS] {
    let mut o = [0u8; OCTETS];
    o[0..4].copy_from_slice(b"BOEC");
    o[4..8].copy_from_slice(&n.to_le_bytes());
    let c = crc32(&o[0..12]);
    o[12..16].copy_from_slice(&c.to_le_bytes());
    o
}

pub fn decode(o: &[u8]) -> u32 {
    if o.len() < OCTETS || &o[0..4] != b"BOEC" || o[8..12] != [0; 4] { return 0; }
    if u32::from_le_bytes([o[12], o[13], o[14], o[15]]) != crc32(&o[0..12]) { return 0; }
    u32::from_le_bytes([o[4], o[5], o[6], o[7]])
}

/// What the loader does with the counter, given the A/B decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Avant {
    /// Write this value (durably) and boot as decided.
    Ecrire(u32),
    /// Too many unconfirmed boots of the active slot: boot recovery.
    Recovery,
}

/// `actif_normal`: the loader is about to boot the active slot with nothing
/// pending (reason Actif). `rollback`: it is rolling a candidate back.
pub fn avant_boot(compteur: u32, actif_normal: bool, rollback: bool) -> Avant {
    if rollback { return Avant::Ecrire(0); }
    if !actif_normal { return Avant::Ecrire(compteur); }
    if compteur >= SEUIL { return Avant::Recovery; }
    Avant::Ecrire(compteur + 1)
}
