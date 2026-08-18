//! Resource Core — accounting unifie CPU / memoire / GPU.
//!
//! Ce module ne possede pas les ressources : il donne une vue coherente de ce
//! que possedent le scheduler, le PMM/VMM, l'arene DMA et le GPU. L'invariant
//! important est que VSS et RSS sont deux metriques differentes : reserver
//! plusieurs TiB de VMA ne doit jamais apparaitre comme plusieurs TiB de RAM.

use crate::kernel::task::Process;
use crate::kernel::vmm::PAGE_SIZE;

#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryUsage {
    /// Espace virtuel reserve (union des VMA).
    pub vss: u64,
    /// Pages reellement presentes dans les tables utilisateur.
    pub rss: u64,
    pub anonymous: u64,
    pub file_private: u64,
    pub shared: u64,
    pub device: u64,
    pub untracked: u64,
}

pub fn memory_usage(process: &Process) -> MemoryUsage {
    let resident = process.space.resident_stats(&process.promesses);
    MemoryUsage {
        vss: crate::kernel::vma::octets_virtuels(&process.promesses),
        rss: resident.total_pages * PAGE_SIZE,
        anonymous: resident.anonymous_pages * PAGE_SIZE,
        file_private: resident.file_private_pages * PAGE_SIZE,
        shared: resident.shared_pages * PAGE_SIZE,
        device: resident.device_pages * PAGE_SIZE,
        untracked: resident.untracked_pages * PAGE_SIZE,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryPressure {
    Normal,
    Moderate,
    High,
    Critical,
}

impl MemoryPressure {
    pub fn label(self) -> &'static str {
        match self {
            MemoryPressure::Normal => "normal",
            MemoryPressure::Moderate => "moderate",
            MemoryPressure::High => "high",
            MemoryPressure::Critical => "critical",
        }
    }
}

/// Niveau de pression base sur les frames physiques libres.
///
/// Ce n'est pas encore un daemon de reclaim : c'est le contrat stable que le
/// futur reclaim/compression/swap pourra consommer.
pub fn memory_pressure() -> MemoryPressure {
    let stats = crate::kernel::vmm::frame_stats_detailed();
    if stats.total == 0 {
        return MemoryPressure::Normal;
    }
    let free_pct = stats.free.saturating_mul(100) / stats.total;
    match free_pct {
        0..=4 => MemoryPressure::Critical,
        5..=9 => MemoryPressure::High,
        10..=24 => MemoryPressure::Moderate,
        _ => MemoryPressure::Normal,
    }
}

/// Vue systeme compacte.
pub fn print_system() {
    let cpu = crate::arch::x86_64::cpu::accounting();
    let frames = crate::kernel::vmm::frame_stats_detailed();
    let dma = crate::kernel::memory::dma_stats();
    let gpu = crate::drivers::gpu::stats();
    let (heap_used, heap_free, heap_total) = crate::kernel::heap::stats();

    crate::println!("Resource Core v1");
    crate::println!(
        "cpu: total={} user={} kernel={} idle={} idle={}%",
        cpu.total_ticks,
        cpu.user_ticks,
        cpu.kernel_ticks,
        cpu.idle_ticks,
        cpu.idle_percent()
    );
    crate::println!(
        "pmm: used={} free={} total={} pages ({} MiB) high={} alloc={} free_ops={} failures={}",
        frames.used,
        frames.free,
        frames.total,
        frames.total * PAGE_SIZE / (1024 * 1024),
        frames.high_watermark,
        frames.allocations,
        frames.frees,
        frames.failures
    );
    crate::println!(
        "heap: used={} KiB free={} KiB total={} KiB",
        heap_used / 1024,
        heap_free / 1024,
        heap_total / 1024
    );
    crate::println!(
        "dma: used={} KiB free={} KiB total={} KiB alloc={} failures={}",
        dma.used / 1024,
        dma.free / 1024,
        dma.total / 1024,
        dma.allocations,
        dma.failures
    );
    crate::println!(
        "gpu: backend={} active={} {}x{}x{} presents={} transferred={} MiB",
        gpu.backend.label(),
        gpu.active,
        gpu.width,
        gpu.height,
        gpu.bpp,
        gpu.presents,
        gpu.bytes_presented / (1024 * 1024)
    );
    crate::println!("memory-pressure: {}", memory_pressure().label());
}

/// Tableau des processus : CPU sur temps mur, VSS et RSS distincts.
pub fn print_processes() {
    let (mesures, window_ticks) = crate::kernel::task::mesure_processus();
    crate::println!(
        "PID    CPU     RSS MiB    VSS MiB  THR  NAME"
    );
    for mesure in mesures {
        let cpu = if window_ticks == 0 {
            0
        } else {
            mesure.ticks.saturating_mul(100) / window_ticks
        };
        crate::println!(
            "{:<6} {:>3}% {:>10} {:>10} {:>4}  {}",
            mesure.pid,
            cpu.min(100),
            mesure.rss_octets / (1024 * 1024),
            mesure.vss_octets / (1024 * 1024),
            mesure.taches,
            mesure.nom
        );
    }
}

/// Invariants peu couteux, utilisables dans la CI bare-metal.
pub fn self_test() -> bool {
    let frames = crate::kernel::vmm::frame_stats_detailed();
    if frames.used > frames.total || frames.free > frames.total {
        return false;
    }
    if frames.used.saturating_add(frames.free) != frames.total {
        return false;
    }

    let cpu = crate::arch::x86_64::cpu::accounting();
    if cpu.user_ticks
        .saturating_add(cpu.kernel_ticks)
        .saturating_add(cpu.idle_ticks)
        != cpu.total_ticks
    {
        return false;
    }

    let dma = crate::kernel::memory::dma_stats();
    if dma.used > dma.total || dma.free > dma.total {
        return false;
    }

    crate::drivers::gpu::self_test()
}
