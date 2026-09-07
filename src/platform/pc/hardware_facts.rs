//! Faits materiels physiques stables et non trompeurs.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use crate::boot::{BootInfo, FramebufferInfo, MemoryRegionKind};

static REPORTED_CPUS: AtomicUsize = AtomicUsize::new(1);
static USABLE_RAM_BYTES: AtomicU64 = AtomicU64::new(0);
static FB_WIDTH: AtomicU32 = AtomicU32::new(0);
static FB_HEIGHT: AtomicU32 = AtomicU32::new(0);
static TIMER_PROGRESS: AtomicBool = AtomicBool::new(false);
static HPET_PRESENT: AtomicBool = AtomicBool::new(false);
static XHCI_PRESENT: AtomicBool = AtomicBool::new(false);
static XHCI_ACTIVE: AtomicBool = AtomicBool::new(false);
static XHCI_CONNECTED_PORTS: AtomicUsize = AtomicUsize::new(0);

pub fn install_base(boot: &BootInfo, framebuffer: FramebufferInfo, reported_cpus: u32, timer_progress: bool, hpet_present: bool) {
    let usable = boot.memory_regions.iter()
        .filter(|r| r.kind == MemoryRegionKind::Usable)
        .fold(0u64, |acc, r| acc.saturating_add(r.len));
    REPORTED_CPUS.store((reported_cpus.max(1)) as usize, Ordering::Release);
    USABLE_RAM_BYTES.store(usable, Ordering::Release);
    FB_WIDTH.store(framebuffer.width, Ordering::Release);
    FB_HEIGHT.store(framebuffer.height, Ordering::Release);
    TIMER_PROGRESS.store(timer_progress, Ordering::Release);
    HPET_PRESENT.store(hpet_present, Ordering::Release);
}

pub fn install_xhci(present: bool, active: bool, connected_ports: usize) {
    XHCI_PRESENT.store(present, Ordering::Release);
    XHCI_ACTIVE.store(active, Ordering::Release);
    XHCI_CONNECTED_PORTS.store(connected_ports, Ordering::Release);
}

pub fn reported_cpus() -> usize { REPORTED_CPUS.load(Ordering::Acquire).max(1) }
pub fn usable_ram_bytes() -> u64 { USABLE_RAM_BYTES.load(Ordering::Acquire) }
pub fn framebuffer_resolution() -> (u32, u32) { (FB_WIDTH.load(Ordering::Acquire), FB_HEIGHT.load(Ordering::Acquire)) }
pub fn timer_progress() -> bool { TIMER_PROGRESS.load(Ordering::Acquire) }
pub fn hpet_present() -> bool { HPET_PRESENT.load(Ordering::Acquire) }
pub fn xhci_present() -> bool { XHCI_PRESENT.load(Ordering::Acquire) }
pub fn xhci_active() -> bool { XHCI_ACTIVE.load(Ordering::Acquire) }
pub fn xhci_connected_ports() -> usize { XHCI_CONNECTED_PORTS.load(Ordering::Acquire) }
