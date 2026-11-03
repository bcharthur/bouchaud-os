//! Loader -> kernel boot handoff (BOUCHAUD_BOOT_HANDOFF_V1).
//!
//! The kernel must know WHICH slot the loader booted, and from WHICH metadata
//! record. Deducing it from the metadata alone is wrong in exactly the case
//! that matters: when the loader could not persist a consumed attempt it boots
//! the active slot while the record still says "candidate pending" -- a kernel
//! that guessed "candidate" would then commit an image that never ran.
//!
//! The loader writes this 64-byte record into one page of UEFI memory of the
//! OS-loader type [`TYPE_MEMOIRE`] (0x8000_0000..0xFFFF_FFFF is reserved by the
//! UEFI spec for OS loaders). The second-stage loader reports that range to
//! the kernel as an unknown UEFI region; the kernel reads it once at boot.
//!
//! Layout (little endian):
//!   0..8   magic "BOHO0001"
//!   8      slot booted (0 = A, 1 = B)
//!   9      reason code ([`Raison`])
//!   10     attempts left on the booted slot (0 when it is not a candidate)
//!   11..16 zero
//!   16..24 sequence of the metadata record on disk at handoff
//!   24..32 boot id (per boot, not cryptographic)
//!   32..48 unique GUID of the META partition the loader read (GPT byte
//!          order): the kernel commits on THAT partition, never on another
//!          disk that happens to carry a Bouchaud layout
//!   48..60 zero
//!   60..64 CRC32 of bytes 0..60
use crate::ab::crc32;
use crate::{Enregistrement, Etat, Slot};

pub const OCTETS: usize = 64;
const MAGIC: &[u8; 8] = b"BOHO0001";
/// UEFI memory type of the handoff page (OS-loader range).
pub const TYPE_MEMOIRE: u32 = 0x8000_B0AB;

/// Why the loader booted this slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raison {
    /// The active (known good) slot, nothing pending.
    Actif = 1,
    /// A candidate, its attempt consumed and durable before the handoff.
    CandidatEssaiConsomme = 2,
    /// The candidate ran out of attempts: rolled back to the active slot.
    RollbackCandidatEpuise = 3,
    /// Rollback chosen but the record could not be written (still boots active).
    RollbackNonPersiste = 4,
    /// A candidate was due but its attempt could not be made durable: active.
    PersistanceImpossibleReplieActif = 5,
    /// RECOVERY booted: no valid metadata (both copies invalid, or a slot
    /// partition missing). `slot` is meaningless.
    RecoveryRequise = 6,
    /// RECOVERY booted: the operator asked for it at the boot prompt.
    RecoveryDemandee = 7,
    /// RECOVERY booted: the active slot failed to confirm health
    /// `echecs::SEUIL` boots in a row.
    RecoveryEchecsRepetes = 8,
}

impl Raison {
    pub const fn texte(self) -> &'static str {
        match self {
            Raison::Actif => "actif",
            Raison::CandidatEssaiConsomme => "candidat_essai_consomme",
            Raison::RollbackCandidatEpuise => "rollback_candidat_epuise",
            Raison::RollbackNonPersiste => "rollback_non_persiste",
            Raison::PersistanceImpossibleReplieActif => "persistance_essai_impossible_repli_actif",
            Raison::RecoveryRequise => "recovery_requise",
            Raison::RecoveryDemandee => "recovery_demandee",
            Raison::RecoveryEchecsRepetes => "recovery_echecs_repetes",
        }
    }
    pub const fn est_recovery(self) -> bool {
        matches!(self, Raison::RecoveryRequise | Raison::RecoveryDemandee | Raison::RecoveryEchecsRepetes)
    }
    const fn depuis(c: u8) -> Option<Raison> {
        Some(match c {
            1 => Raison::Actif,
            2 => Raison::CandidatEssaiConsomme,
            3 => Raison::RollbackCandidatEpuise,
            4 => Raison::RollbackNonPersiste,
            5 => Raison::PersistanceImpossibleReplieActif,
            6 => Raison::RecoveryRequise,
            7 => Raison::RecoveryDemandee,
            8 => Raison::RecoveryEchecsRepetes,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Passage {
    pub slot: Slot,
    pub raison: Raison,
    pub essais_restants: u8,
    pub sequence_meta: u64,
    pub boot_id: u64,
    pub meta_unique: [u8; 16],
}

pub fn encode(p: &Passage) -> [u8; OCTETS] {
    let mut o = [0u8; OCTETS];
    o[0..8].copy_from_slice(MAGIC);
    o[8] = p.slot as u8;
    o[9] = p.raison as u8;
    o[10] = p.essais_restants;
    o[16..24].copy_from_slice(&p.sequence_meta.to_le_bytes());
    o[24..32].copy_from_slice(&p.boot_id.to_le_bytes());
    o[32..48].copy_from_slice(&p.meta_unique);
    let c = crc32(&o[0..60]);
    o[60..64].copy_from_slice(&c.to_le_bytes());
    o
}

pub fn decode(o: &[u8]) -> Option<Passage> {
    if o.len() < OCTETS || &o[0..8] != MAGIC { return None; }
    if u32::from_le_bytes([o[60], o[61], o[62], o[63]]) != crc32(&o[0..60]) { return None; }
    if o[11..16].iter().chain(o[48..60].iter()).any(|&b| b != 0) { return None; }
    let slot = match o[8] { 0 => Slot::A, 1 => Slot::B, _ => return None };
    let raison = Raison::depuis(o[9])?;
    let mut s = [0u8; 8];
    s.copy_from_slice(&o[16..24]);
    let mut b = [0u8; 8];
    b.copy_from_slice(&o[24..32]);
    let mut m = [0u8; 16];
    m.copy_from_slice(&o[32..48]);
    Some(Passage { slot, raison, essais_restants: o[10],
                   sequence_meta: u64::from_le_bytes(s), boot_id: u64::from_le_bytes(b), meta_unique: m })
}

/// Why a health commit is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusCommit {
    /// The record on disk is not the one the loader booted from: something
    /// wrote the metadata since. Never commit on a record we did not boot.
    MetaChangee,
    /// The booted slot is neither a pending candidate nor the good active
    /// slot (e.g. it was marked Bad meanwhile).
    SlotNiCandidatNiActif,
}

/// What a health confirmation does to the metadata, given what the loader
/// handed over and the record now on disk.
///
///   Ok(Some(n)) : the booted candidate becomes active and Good; persist `n`.
///   Ok(None)    : the booted slot is already the good active slot: nothing to
///                 write (this includes the fallback case where a candidate
///                 was pending but the ACTIVE slot ran -- it stays pending --
///                 and an install into the other slot started meanwhile).
///   Err(_)      : refuse, write nothing.
pub fn commit_sante(p: &Passage, e: &Enregistrement) -> Result<Option<Enregistrement>, RefusCommit> {
    // Recovery is not a system: nothing it runs may confirm a slot.
    if p.raison.est_recovery() { return Err(RefusCommit::SlotNiCandidatNiActif); }
    // Already the good active slot: confirming it writes nothing, whatever
    // changed in the record since the boot (an install into the OTHER slot
    // rewrites META while this system runs -- found by the QEMU update bench).
    if e.actif == p.slot && e.slot(p.slot).etat == Etat::Bon { return Ok(None); }
    // Promoting a candidate is only valid on the very record it booted from.
    if e.sequence != p.sequence_meta { return Err(RefusCommit::MetaChangee); }
    match e.slot(p.slot).etat {
        Etat::Candidat { .. } if p.raison == Raison::CandidatEssaiConsomme => Ok(Some(e.sante_ok(p.slot))),
        _ => Err(RefusCommit::SlotNiCandidatNiActif),
    }
}
