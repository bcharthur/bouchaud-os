//! Bring-up sûr du futur matériel de référence.
//!
//! Ce mode est une étape de diagnostic, pas un mode produit. Il arrête
//! volontairement le boot avant les pilotes à effets de bord.

use crate::boot::{BootInfo, FirmwareKind};

/// Activé uniquement par `--features reference-bringup`.
pub const fn enabled() -> bool {
    cfg!(feature = "reference-bringup")
}

fn firmware_label(firmware: FirmwareKind) -> &'static str {
    match firmware {
        FirmwareKind::LegacyBios => "legacy-bios",
        FirmwareKind::Uefi => "uefi",
        FirmwareKind::Unknown => "unknown",
    }
}

pub fn announce(boot: &BootInfo) {
    crate::serial_println!(
        "[BRINGUP] mode=reference-stage1 firmware={} regions={} map_complete={}",
        firmware_label(boot.firmware),
        boot.memory_regions.len(),
        boot.memory_regions_complete as u8,
    );
    crate::serial_println!("[BRINGUP] smp=off");
    crate::serial_println!("[BRINGUP] storage-write=off");
    crate::serial_println!("[BRINGUP] network=off");
    crate::serial_println!("[BRINGUP] audio=off");
}

/// Termine le lot foundation sans atteindre clavier, ATA, persistance, réseau
/// ou audio.
///
/// Le marqueur est volontairement LEGACY. Il ne prouve ni UEFI ni GOP.
fn framebuffer_format_label(format: crate::boot::FramebufferPixelFormat) -> &'static str {
    match format {
        crate::boot::FramebufferPixelFormat::Rgb => "rgb",
        crate::boot::FramebufferPixelFormat::Bgr => "bgr",
        crate::boot::FramebufferPixelFormat::U8 => "u8",
        crate::boot::FramebufferPixelFormat::Unknown => "unknown",
    }
}

/// Preuve UEFI Lot 2A. Elle exige un framebuffer firmware valide mais ne
/// dessine pas encore dedans : l'ecriture GOP sera le Lot 2B.
pub fn complete_uefi_bootinfo_and_halt(boot: &BootInfo) -> ! {
    if boot.firmware != FirmwareKind::Uefi {
        panic!("bringup UEFI appele sur un firmware non UEFI");
    }
    if !boot.memory_regions_complete {
        panic!("bringup UEFI: carte memoire tronquee");
    }
    if boot.physical_memory_offset.is_none() {
        panic!("bringup UEFI: mapping physique absent");
    }

    let framebuffer = boot
        .framebuffer
        .expect("bringup UEFI: framebuffer GOP absent ou invalide");

    crate::serial_println!(
        "[BRINGUP] uefi phys_offset={:#x} rsdp={}",
        boot.physical_memory_offset.unwrap_or(0),
        boot.rsdp_address.is_some() as u8,
    );
    crate::serial_println!(
        "[BRINGUP] uefi framebuffer=1 addr={:#x} bytes={} {}x{} stride={} bpp={} format={}",
        framebuffer.address,
        framebuffer.byte_len,
        framebuffer.width,
        framebuffer.height,
        framebuffer.stride,
        framebuffer.bytes_per_pixel,
        framebuffer_format_label(framebuffer.pixel_format),
    );
    crate::serial_println!("BOUCHAUD_UEFI_BOOTINFO_OK");

    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}

pub fn complete_legacy_foundation_and_halt(boot: &BootInfo) -> ! {
    crate::serial_println!(
        "[BRINGUP] phys_offset={:#x} rsdp={} framebuffer={}",
        boot.physical_memory_offset.unwrap_or(0),
        boot.rsdp_address.is_some() as u8,
        boot.framebuffer.is_some() as u8,
    );
    crate::serial_println!("BOUCHAUD_REFERENCE_BRINGUP_LEGACY_OK");

    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}
