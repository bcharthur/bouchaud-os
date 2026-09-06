//! Telemetrie read-only du dashboard de bring-up de reference.
//!
//! Important: ce module ne sonde AUCUN bus et n'initialise AUCUN pilote.
//! Les informations materiel viennent uniquement de CPUID et du BootInfo UEFI.
//! Le stockage reste volontairement non sonde pendant le Stage 1.

use alloc::string::{String, ToString};
use core::arch::x86_64::__cpuid;

use crate::boot::{BootInfo, FramebufferInfo, MemoryRegionKind};

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub cpu_brand: String,
    pub cpu_vendor: String,
    pub logical_cpus_reported: u32,
    pub tsc_mhz: Option<u64>,
    pub uptime_ms: u64,

    pub usable_memory_bytes: u64,

    pub heap_used_bytes: usize,
    pub heap_free_bytes: usize,
    pub heap_total_bytes: usize,

    pub frame_used: u64,
    pub frame_total: u64,

    pub framebuffer_bytes: usize,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_bpp: u8,

    pub storage_state: &'static str,
    pub storage_write_state: &'static str,
    pub network_state: &'static str,
    pub smp_state: &'static str,
    pub cpu_load_state: &'static str,
}

fn trim_ascii(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.trim_matches('\0').trim().to_string()
}

fn cpu_vendor() -> String {
    let leaf = unsafe { __cpuid(0) };
    let mut bytes = [0u8; 12];
    bytes[0..4].copy_from_slice(&leaf.ebx.to_le_bytes());
    bytes[4..8].copy_from_slice(&leaf.edx.to_le_bytes());
    bytes[8..12].copy_from_slice(&leaf.ecx.to_le_bytes());
    trim_ascii(&bytes)
}

fn cpu_brand() -> String {
    let max = unsafe { __cpuid(0x8000_0000) }.eax;
    if max < 0x8000_0004 {
        return cpu_vendor();
    }

    let mut bytes = [0u8; 48];
    for (index, leaf_id) in (0x8000_0002..=0x8000_0004).enumerate() {
        let leaf = unsafe { __cpuid(leaf_id) };
        let out = &mut bytes[index * 16..index * 16 + 16];
        out[0..4].copy_from_slice(&leaf.eax.to_le_bytes());
        out[4..8].copy_from_slice(&leaf.ebx.to_le_bytes());
        out[8..12].copy_from_slice(&leaf.ecx.to_le_bytes());
        out[12..16].copy_from_slice(&leaf.edx.to_le_bytes());
    }
    let brand = trim_ascii(&bytes);
    if brand.is_empty() { cpu_vendor() } else { brand }
}

fn logical_cpus_reported() -> u32 {
    let max = unsafe { __cpuid(0) }.eax;
    if max < 1 {
        return 1;
    }
    let leaf = unsafe { __cpuid(1) };
    ((leaf.ebx >> 16) & 0xff).max(1)
}

fn usable_memory_bytes(boot: &BootInfo) -> u64 {
    boot.memory_regions
        .iter()
        .filter(|region| region.kind == MemoryRegionKind::Usable)
        .fold(0u64, |acc, region| acc.saturating_add(region.len))
}

pub fn snapshot(boot: &BootInfo, framebuffer: FramebufferInfo) -> Snapshot {
    let (heap_used, heap_free, heap_total) = crate::kernel::heap::stats();
    let (frame_used, frame_total) = crate::kernel::vmm::frame_stats_relaxed();

    Snapshot {
        cpu_brand: cpu_brand(),
        cpu_vendor: cpu_vendor(),
        logical_cpus_reported: logical_cpus_reported(),
        tsc_mhz: crate::kernel::timer::tsc_hz().map(|hz| hz / 1_000_000),
        uptime_ms: crate::kernel::timer::monotonic_ms(),

        usable_memory_bytes: usable_memory_bytes(boot),

        heap_used_bytes: heap_used,
        heap_free_bytes: heap_free,
        heap_total_bytes: heap_total,

        frame_used,
        frame_total,

        framebuffer_bytes: framebuffer.byte_len,
        framebuffer_width: framebuffer.width,
        framebuffer_height: framebuffer.height,
        framebuffer_bpp: framebuffer.bytes_per_pixel,

        // Stage 1 reste strictement sans sonde disque/NVMe/ATA.
        storage_state: "Not probed - safe Stage 1",
        storage_write_state: "OFF",
        network_state: "OFF",
        smp_state: "BSP only",
        // La charge CPU systeme devient significative quand scheduler + IRQ
        // sont actifs. Afficher un pourcentage maintenant serait trompeur.
        cpu_load_state: "N/A before scheduler",
    }
}
