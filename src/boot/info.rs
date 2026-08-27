//! Informations de démarrage indépendantes de l'architecture et du firmware.

use crate::arch::api::Architecture;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformKind {
    Pc,
    QemuVirt,
    RaspberryPi4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryRegionKind {
    Usable,
    Reserved,
    Mmio,
    Firmware,
}

#[derive(Clone, Copy, Debug)]
pub struct MemoryRegion {
    pub start: u64,
    pub len: u64,
    pub kind: MemoryRegionKind,
}

#[derive(Clone, Copy, Debug)]
pub struct FramebufferInfo {
    pub address: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bytes_per_pixel: u8,
}

pub struct BootInfo {
    pub architecture: Architecture,
    pub platform: PlatformKind,
    pub memory_regions: &'static [MemoryRegion],
    pub framebuffer: Option<FramebufferInfo>,
    pub physical_memory_offset: Option<u64>,
    pub device_tree: Option<usize>,
}
