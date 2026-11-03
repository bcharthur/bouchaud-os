//! On-disk identities of the A/B layout (BOUCHAUD_DISPOSITION_AB_V1).
//!
//! The loader finds META and the slots by GPT partition TYPE, on the disk it
//! booted from -- never by partition number, never on another disk. One source
//! for the loader, the image builder and the kernel.

/// A GPT type GUID: textual form (gpt tools) and fields (UEFI `Guid`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeGuid {
    pub texte: &'static str,
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

const fn bouchaud(n: u16, texte: &'static str) -> TypeGuid {
    TypeGuid { texte, d1: 0xB0DC_8A5D, d2: n, d3: 0x4B0D,
               d4: [0x9E, 0x21, 0x42, 0x4F, 0x55, 0x43, 0x48, 0x44] }
}

/// Persistence (STATE): the existing TYPE_SYSTEME_BOUCHAUD of src/fs/gpt.rs.
pub const TYPE_ETAT: TypeGuid = bouchaud(0x0001, "B0DC8A5D-0001-4B0D-9E21-424F55434844");
/// Raw boot metadata: copy 0 at block 0, copy 1 at block 1.
pub const TYPE_META: TypeGuid = bouchaud(0x0002, "B0DC8A5D-0002-4B0D-9E21-424F55434844");
/// System slots: FAT, each a complete bootable system.
pub const TYPE_SLOT_A: TypeGuid = bouchaud(0x0003, "B0DC8A5D-0003-4B0D-9E21-424F55434844");
pub const TYPE_SLOT_B: TypeGuid = bouchaud(0x0004, "B0DC8A5D-0004-4B0D-9E21-424F55434844");

/// The second-stage loader, inside each slot partition.
pub const CHARGEUR_DANS_SLOT: &str = "\\EFI\\BOOT\\BOUCHAUD-LOADER.EFI";

pub const fn type_du_slot(slot: crate::Slot) -> TypeGuid {
    match slot { crate::Slot::A => TYPE_SLOT_A, crate::Slot::B => TYPE_SLOT_B }
}
