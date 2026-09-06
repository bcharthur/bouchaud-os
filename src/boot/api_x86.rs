//! Adaptateur bootloader_api 0.11 -> contrat de boot Bouchaud.
//!
//! Ce producteur est utilisé par le chemin UEFI. Il garde toute dépendance à
//! `bootloader_api` hors du coeur du noyau.

use bootloader_api::info::{
    MemoryRegionKind as ApiMemoryRegionKind,
    PixelFormat as ApiPixelFormat,
};
use bootloader_api::BootInfo as ApiBootInfo;

use crate::arch::api::Architecture;

use super::{
    BootInfo, FirmwareKind, FramebufferInfo, FramebufferPixelFormat,
    MemoryRegion, MemoryRegionKind, PlatformKind, RamdiskInfo,
};

const MAX_MEMORY_REGIONS: usize = 512;

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
    firmware: FirmwareKind::Uefi,
    memory_regions: &[],
    memory_regions_complete: false,
    framebuffer: None,
    ramdisk: None,
    physical_memory_offset: None,
    rsdp_address: None,
    device_tree: None,
};

fn framebuffer_info(api: &mut ApiBootInfo) -> Option<FramebufferInfo> {
    let framebuffer = api.framebuffer.as_mut()?;
    let info = framebuffer.info();

    if info.byte_len == 0
        || info.width == 0
        || info.height == 0
        || info.stride < info.width
        || info.bytes_per_pixel == 0
    {
        return None;
    }

    let width = u32::try_from(info.width).ok()?;
    let height = u32::try_from(info.height).ok()?;
    let stride = u32::try_from(info.stride).ok()?;
    let bytes_per_pixel = u8::try_from(info.bytes_per_pixel).ok()?;

    let pixel_format = match info.pixel_format {
        ApiPixelFormat::Rgb => FramebufferPixelFormat::Rgb,
        ApiPixelFormat::Bgr => FramebufferPixelFormat::Bgr,
        ApiPixelFormat::U8 => FramebufferPixelFormat::U8,
        _ => FramebufferPixelFormat::Unknown,
    };

    // `buffer_mut` retourne la vue VIRTUELLE réellement mappée par
    // bootloader_api. On ne suppose donc jamais que BAR0 du GPU est un LFB.
    let buffer = framebuffer.buffer_mut();
    if buffer.len() < info.byte_len {
        return None;
    }

    Some(FramebufferInfo {
        address: buffer.as_mut_ptr() as u64,
        byte_len: info.byte_len,
        width,
        height,
        stride,
        bytes_per_pixel,
        pixel_format,
    })
}

fn ramdisk_info(api: &ApiBootInfo) -> Option<RamdiskInfo> {
    let address = api.ramdisk_addr.as_ref().copied()?;
    let byte_len = usize::try_from(api.ramdisk_len).ok()?;
    if address == 0 || byte_len == 0 {
        return None;
    }
    // bootloader_api mappe le ramdisk selon mappings.ramdisk_memory puis place
    // cette adresse VIRTUELLE dans BootInfo::ramdisk_addr.
    Some(RamdiskInfo { address, byte_len })
}

/// Convertit le BootInfo UEFI en contrat Bouchaud avant d'entrer dans le
/// noyau générique.
pub fn from_bootloader_api(api: &'static mut ApiBootInfo) -> &'static BootInfo {
    let mut count = 0usize;
    let mut complete = true;

    for region in api.memory_regions.iter() {
        if region.end <= region.start {
            continue;
        }

        if count == MAX_MEMORY_REGIONS {
            complete = false;
            break;
        }

        let kind = match region.kind {
            ApiMemoryRegionKind::Usable => MemoryRegionKind::Usable,
            _ => MemoryRegionKind::Reserved,
        };

        unsafe {
            MEMORY_REGIONS[count] = MemoryRegion {
                start: region.start,
                len: region.end - region.start,
                kind,
            };
        }
        count += 1;
    }

    let physical_memory_offset = api.physical_memory_offset.as_ref().copied();
    let rsdp_address = api.rsdp_addr.as_ref().copied();
    let ramdisk = ramdisk_info(api);
    let framebuffer = framebuffer_info(api);

    unsafe {
        let regions = core::slice::from_raw_parts(
            core::ptr::addr_of!(MEMORY_REGIONS) as *const MemoryRegion,
            count,
        );

        NORMALIZED_BOOT_INFO = BootInfo {
            architecture: Architecture::X86_64,
            platform: PlatformKind::Pc,
            firmware: FirmwareKind::Uefi,
            memory_regions: regions,
            memory_regions_complete: complete,
            framebuffer,
            ramdisk,
            physical_memory_offset,
            rsdp_address,
            device_tree: None,
        };

        &*core::ptr::addr_of!(NORMALIZED_BOOT_INFO)
    }
}
