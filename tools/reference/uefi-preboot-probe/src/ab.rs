//! BOUCHAUD_LOADER_AB_V1 -- the Bouchaud Loader's A/B decision.
//!
//! Order (§9 of the Atomic Platform mission), each step before the next:
//!
//!   read both META copies -> validate (CRC, structure, active slot Good)
//!   -> newest valid copy -> choose the slot (update-core)
//!   -> if a candidate attempt is consumed: WRITE the new record to the older
//!      copy, FLUSH, READ IT BACK and decode it
//!   -> only then load the second-stage loader FROM THE SLOT'S PARTITION.
//!
//! A candidate is never booted unless its consumed attempt is durable: if the
//! write, the flush or the read-back fails, the known-good active slot boots
//! instead. Two invalid copies never boot a guess: RECOVERY_REQUIRED.
//!
//! Partitions are found by Bouchaud GPT type on the disk this shim booted from
//! (same device path up to the partition node), never by number.

use alloc::vec;
use alloc::vec::Vec;

use bouchaud_update_core::passage::{self, Passage, Raison};
use bouchaud_update_core::{self as uc, disque, Etat, Slot};
use uefi::proto::device_path::{DevicePath, DeviceType};
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::block::BlockIO;
use uefi::proto::media::partition::PartitionInfo;
use uefi::{boot, Guid, Handle};

pub enum Decision {
    /// No META partition on this disk: the pre-A/B layout, unchanged.
    Legacy,
    /// `passage` is what the kernel receives (BOUCHAUD_BOOT_HANDOFF_V1).
    Boot { slot: Slot, partition: Handle, raison: &'static str, passage: Passage },
    RecoveryRequired(&'static str),
}

type Prefixe = Vec<(u8, u8, Vec<u8>)>;

/// The device path of a partition handle without its MEDIA nodes: the disk.
fn disque_de(handle: Handle) -> Option<Prefixe> {
    let dp = boot::open_protocol_exclusive::<DevicePath>(handle).ok()?;
    let mut out = Vec::new();
    for node in dp.node_iter() {
        if node.device_type() == DeviceType::MEDIA { break; }
        out.push((node.device_type().0, node.sub_type().0, node.data().to_vec()));
    }
    Some(out)
}

fn type_gpt(handle: Handle) -> Option<Guid> {
    let info = boot::open_protocol_exclusive::<PartitionInfo>(handle).ok()?;
    info.gpt_partition_entry().map(|e| e.partition_type_guid.0)
}

/// Unique partition GUID, in GPT on-disk byte order.
fn unique_gpt(handle: Handle) -> [u8; 16] {
    let Ok(info) = boot::open_protocol_exclusive::<PartitionInfo>(handle) else { return [0; 16] };
    info.gpt_partition_entry().map(|e| e.unique_partition_guid.to_bytes()).unwrap_or([0; 16])
}

fn guid(t: disque::TypeGuid) -> Option<Guid> { Guid::try_parse(t.texte).ok() }

struct Partitions { meta: Option<Handle>, a: Option<Handle>, b: Option<Handle> }

fn trouve_partitions() -> Option<Partitions> {
    let moi = boot::open_protocol_exclusive::<LoadedImage>(boot::image_handle()).ok()?;
    let disque = disque_de(moi.device()?)?;
    drop(moi);
    let (gm, ga, gb) = (guid(disque::TYPE_META)?, guid(disque::TYPE_SLOT_A)?, guid(disque::TYPE_SLOT_B)?);
    let mut p = Partitions { meta: None, a: None, b: None };
    for h in boot::find_handles::<PartitionInfo>().ok()? {
        if disque_de(h).as_ref() != Some(&disque) { continue; }
        match type_gpt(h) {
            Some(t) if t == gm => p.meta = Some(h),
            Some(t) if t == ga => p.a = Some(h),
            Some(t) if t == gb => p.b = Some(h),
            _ => {}
        }
    }
    Some(p)
}

/// Reads the two copies: (block size, copy0 block, copy1 block).
fn lit_copies(meta: Handle) -> Option<(usize, Vec<u8>, Vec<u8>)> {
    let bio = boot::open_protocol_exclusive::<BlockIO>(meta).ok()?;
    let media = bio.media();
    let taille = media.block_size() as usize;
    if taille < uc::OCTETS || media.last_block() < 1 { return None; }
    let mut c0 = vec![0u8; taille];
    let mut c1 = vec![0u8; taille];
    bio.read_blocks(media.media_id(), 0, &mut c0).ok()?;
    bio.read_blocks(media.media_id(), 1, &mut c1).ok()?;
    Some((taille, c0, c1))
}

/// Persist `e` into the older copy, flush, read back. True only if the
/// re-read record is exactly `e`.
fn persiste(meta: Handle, e: &uc::Enregistrement) -> bool {
    let Some((_, c0, c1)) = lit_copies(meta) else { return false };
    let i = uc::copie_a_ecrire(&c0[..uc::OCTETS], &c1[..uc::OCTETS]);
    let mut bloc = if i == 0 { c0 } else { c1 };
    bloc[..uc::OCTETS].copy_from_slice(&uc::encode(e));
    {
        let Ok(mut bio) = boot::open_protocol_exclusive::<BlockIO>(meta) else { return false };
        let media_id = bio.media().media_id();
        if bio.write_blocks(media_id, i as u64, &bloc).is_err() { return false; }
        if bio.flush_blocks().is_err() { return false; }
    }
    let Some((_, r0, r1)) = lit_copies(meta) else { return false };
    let relu = if i == 0 { r0 } else { r1 };
    uc::decode(&relu[..uc::OCTETS]).as_ref() == Some(e)
}

pub fn decide(journal: &mut alloc::string::String) -> Decision {
    use core::fmt::Write as _;
    let Some(p) = trouve_partitions() else { return Decision::Legacy };
    let Some(meta) = p.meta else { return Decision::Legacy };
    let handle_de = |s: Slot| match s { Slot::A => p.a, Slot::B => p.b };
    let Some((_, c0, c1)) = lit_copies(meta) else { return Decision::RecoveryRequired("meta_illisible") };
    let Some(e) = uc::selectionne(&c0[..uc::OCTETS], &c1[..uc::OCTETS]) else {
        return Decision::RecoveryRequired("deux_copies_invalides");
    };
    let _ = writeln!(journal, "ab.lu sequence={} actif={:?} a={:?} b={:?}",
                     e.sequence, e.actif, e.slot(Slot::A).etat, e.slot(Slot::B).etat);
    let (slot, apres) = e.choisit_boot();
    let mut raison = Raison::Actif;
    let mut choisi = slot;
    let mut sur_disque = e;
    if apres != e {
        if persiste(meta, &apres) {
            sur_disque = apres;
            raison = if slot != e.actif { Raison::CandidatEssaiConsomme } else { Raison::RollbackCandidatEpuise };
        } else if slot != e.actif {
            // Never boot a candidate whose consumed attempt is not durable.
            choisi = e.actif;
            raison = Raison::PersistanceImpossibleReplieActif;
        } else {
            raison = Raison::RollbackNonPersiste;
        }
    }
    let essais_restants = match sur_disque.slot(choisi).etat { Etat::Candidat { essais } => essais, _ => 0 };
    // Per-boot identity, not a secret: TSC mixed with the record sequence.
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    let boot_id = tsc ^ sur_disque.sequence.rotate_left(32) ^ 0xB0AB_0000_0000_0000;
    let passage = Passage { slot: choisi, raison, essais_restants, sequence_meta: sur_disque.sequence,
                            boot_id, meta_unique: unique_gpt(meta) };
    match handle_de(choisi) {
        Some(h) => Decision::Boot { slot: choisi, partition: h, raison: raison.texte(), passage },
        None => Decision::RecoveryRequired("partition_du_slot_absente"),
    }
}

/// Leaves the handoff in one page of OS-loader memory (passage::TYPE_MEMOIRE).
/// The page survives ExitBootServices (it is not boot-services memory) and is
/// reported to the kernel as an unknown UEFI region. Best effort: a kernel
/// that finds no handoff simply never commits health (fail closed).
pub fn depose_passage(p: &Passage) -> bool {
    let Ok(page) = boot::allocate_pages(
        boot::AllocateType::AnyPages,
        uefi::boot::MemoryType::custom(passage::TYPE_MEMOIRE),
        1,
    ) else { return false };
    let o = passage::encode(p);
    unsafe {
        core::ptr::write_bytes(page.as_ptr(), 0, 4096);
        core::ptr::copy_nonoverlapping(o.as_ptr(), page.as_ptr(), o.len());
    }
    true
}
