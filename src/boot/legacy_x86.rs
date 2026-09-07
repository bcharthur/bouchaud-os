//! Adaptateur temporaire entre `bootloader` 0.9 et le contrat Bouchaud.
//!
//! Il n'alloue rien : il s'exécute avant que le tas normal soit disponible.

use bootloader::bootinfo::MemoryRegionType;
use bootloader::BootInfo as LegacyBootInfo;

use crate::arch::api::Architecture;

use super::{
    BootInfo, FirmwareKind, MemoryRegion, MemoryRegionKind, PlatformKind,
};

const MAX_MEMORY_REGIONS: usize = 256;

const EMPTY_REGION: MemoryRegion = MemoryRegion {
    start: 0,
    len: 0,
    kind: MemoryRegionKind::Reserved,
};

static mut MEMORY_REGIONS: [MemoryRegion; MAX_MEMORY_REGIONS] =
    [EMPTY_REGION; MAX_MEMORY_REGIONS];

static mut NORMALIZED_BOOT_INFO: BootInfo = BootInfo {
    architecture: Architecture::X86_64,
    platform: PlatformKind::Pc,
    firmware: FirmwareKind::LegacyBios,
    memory_regions: &[],
    memory_regions_complete: false,
    framebuffer: None,
    ramdisk: None,
    physical_memory_offset: None,
    rsdp_address: None,
    device_tree: None,
};

/// Normalise le BootInfo de `bootloader` 0.9 sans allocation.
///
/// Seules les régions explicitement `Usable` deviennent candidates aux
/// allocateurs. Tout le reste reste réservé.
pub fn from_bootloader_09(legacy: &'static LegacyBootInfo) -> &'static BootInfo {
    let mut count = 0usize;
    let mut complete = true;

    for region in legacy.memory_map.iter() {
        let start = region.range.start_addr();
        let end = region.range.end_addr();

        if end <= start {
            continue;
        }

        if count == MAX_MEMORY_REGIONS {
            complete = false;
            break;
        }

        let kind = if region.region_type == MemoryRegionType::Usable {
            MemoryRegionKind::Usable
        } else {
            MemoryRegionKind::Reserved
        };

        unsafe {
            MEMORY_REGIONS[count] = MemoryRegion {
                start,
                len: end - start,
                kind,
            };
        }

        count += 1;
    }

    unsafe {
        let regions = core::slice::from_raw_parts(
            core::ptr::addr_of!(MEMORY_REGIONS) as *const MemoryRegion,
            count,
        );

        NORMALIZED_BOOT_INFO = BootInfo {
            architecture: Architecture::X86_64,
            platform: PlatformKind::Pc,
            firmware: FirmwareKind::LegacyBios,
            memory_regions: regions,
            memory_regions_complete: complete,
            framebuffer: None,
            ramdisk: None,
            physical_memory_offset: Some(legacy.physical_memory_offset),
            rsdp_address: None,
            device_tree: None,
        };

        &*core::ptr::addr_of!(NORMALIZED_BOOT_INFO)
    }
}
