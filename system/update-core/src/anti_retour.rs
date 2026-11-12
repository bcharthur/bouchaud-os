//! Anti-rollback: the floor below which no release may be INSTALLED
//! (BOUCHAUD_ANTI_RETOUR_V1).
//!
//! # Two different things
//!
//! - **Local rollback** (a candidate fails, or `rollback --confirme` in
//!   recovery): the A/B record switches back to the slot that is already on
//!   the disk. It needs no network and no new signature, and this module does
//!   NOT touch it.
//! - **Hostile downgrade**: someone hands the machine an OLD release that was
//!   correctly signed at the time (and may carry a fixed vulnerability). The
//!   signature proves who published it, not that it is recent. This module
//!   refuses it: a release whose signed `sequence` is below the floor is
//!   never installed.
//!
//! # Where it lives
//!
//! META blocks 3 and 4, two copies (sequence + CRC, the most recent valid one
//! wins), APART from the A/B record (blocks 0 and 1) whose format the UEFI
//! loader on the ESP reads: an update rewrites the slots, never the loader,
//! so the A/B format must not change. The loader ignores these blocks.
//!
//! Layout (first 48 bytes of each block): "BOAR0001", u64 record sequence,
//! u64 floor, u64 release sequence of slot A, u64 of slot B, u32 reserved (0),
//! u32 CRC32 of bytes 0..44.
//!
//! # Reading
//!
//! - both blocks entirely zero: a fresh install, floor 0;
//! - at least one valid copy: the most recent valid one;
//! - otherwise (garbage, torn by something other than our two-copy write):
//!   UNREADABLE -- installs are refused. Never silently reset to 0, which
//!   would re-open every old release.
//!
//! # Policy
//!
//! - `begin`: refused if the manifest sequence is below the floor, and a v1
//!   manifest (sequence 0) is refused as soon as a floor exists;
//! - `finish`: the target slot's release sequence is recorded (before the
//!   candidate is marked in the A/B record);
//! - health commit of the running slot: floor = max(floor, its sequence).
//!   Idempotent, so a power cut between the A/B commit and this write is
//!   repaired by the next confirmed boot. The floor never goes down.

use crate::ab::{crc32, Slot};

pub const OCTETS: usize = 48;
/// META blocks holding the two copies.
pub const BLOCS: [u64; 2] = [3, 4];
const MAGIC: [u8; 8] = *b"BOAR0001";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registre {
    /// Increases at every write; picks the most recent copy.
    pub sequence: u64,
    /// No release below this one is installed.
    pub plancher: u64,
    /// Release sequence installed in each slot (0: unknown).
    pub slots: [u64; 2],
}

impl Registre {
    pub const NEUF: Registre = Registre { sequence: 0, plancher: 0, slots: [0, 0] };

    fn suivant(&self) -> Registre {
        Registre { sequence: self.sequence.wrapping_add(1), ..*self }
    }

    /// `finish`: the target slot now holds release `sequence`.
    pub fn installe(&self, cible: Slot, sequence: u64) -> Registre {
        let mut n = self.suivant();
        n.slots[cible as usize] = sequence;
        n
    }

    /// Health commit of `slot`: the floor rises to its release, never down.
    /// `None` if nothing changes (no write needed).
    pub fn confirme(&self, slot: Slot) -> Option<Registre> {
        let s = self.slots[slot as usize];
        if s <= self.plancher { return None; }
        let mut n = self.suivant();
        n.plancher = s;
        Some(n)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lecture {
    Neuf,
    Valide(Registre),
    Illisible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    /// The release is older than the floor.
    Retrograde,
    /// A v1 manifest (no sequence) while a floor exists.
    SansSequence,
    /// The record cannot be read: refuse rather than guess 0.
    RegistreIllisible,
}

pub fn encode(r: &Registre) -> [u8; OCTETS] {
    let mut o = [0u8; OCTETS];
    o[0..8].copy_from_slice(&MAGIC);
    o[8..16].copy_from_slice(&r.sequence.to_le_bytes());
    o[16..24].copy_from_slice(&r.plancher.to_le_bytes());
    o[24..32].copy_from_slice(&r.slots[0].to_le_bytes());
    o[32..40].copy_from_slice(&r.slots[1].to_le_bytes());
    let c = crc32(&o[0..44]);
    o[44..48].copy_from_slice(&c.to_le_bytes());
    o
}

fn u64_le(o: &[u8]) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&o[..8]);
    u64::from_le_bytes(b)
}

pub fn decode(o: &[u8]) -> Option<Registre> {
    if o.len() < OCTETS || o[0..8] != MAGIC || o[40..44] != [0; 4] { return None; }
    if u32::from_le_bytes([o[44], o[45], o[46], o[47]]) != crc32(&o[0..44]) { return None; }
    Some(Registre {
        sequence: u64_le(&o[8..]),
        plancher: u64_le(&o[16..]),
        slots: [u64_le(&o[24..]), u64_le(&o[32..])],
    })
}

/// Reads the two copies (whole blocks).
pub fn lit(bloc0: &[u8], bloc1: &[u8]) -> Lecture {
    match (decode(bloc0), decode(bloc1)) {
        (Some(x), Some(y)) => Lecture::Valide(if y.sequence > x.sequence { y } else { x }),
        (Some(x), None) => Lecture::Valide(x),
        (None, Some(y)) => Lecture::Valide(y),
        (None, None) if bloc0.iter().all(|&b| b == 0) && bloc1.iter().all(|&b| b == 0) => Lecture::Neuf,
        (None, None) => Lecture::Illisible,
    }
}

/// Which copy to overwrite: never the most recent valid one.
pub fn copie_a_ecrire(bloc0: &[u8], bloc1: &[u8]) -> usize {
    match (decode(bloc0), decode(bloc1)) {
        (Some(x), Some(y)) => if y.sequence > x.sequence { 0 } else { 1 },
        (Some(_), None) => 1,
        _ => 0,
    }
}

/// `begin`: may a release with this signed sequence be installed?
pub fn autorise(lecture: Lecture, sequence: u64) -> Result<Registre, Refus> {
    let r = match lecture {
        Lecture::Neuf => Registre::NEUF,
        Lecture::Valide(r) => r,
        Lecture::Illisible => return Err(Refus::RegistreIllisible),
    };
    if r.plancher > 0 && sequence == 0 { return Err(Refus::SansSequence); }
    if sequence < r.plancher { return Err(Refus::Retrograde); }
    Ok(r)
}
