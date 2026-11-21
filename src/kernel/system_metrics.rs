//! Shared, rate-limited system snapshots for user space and the desktop.

use crate::kernel::{sync::SpinLock, task, timer};
use alloc::{format, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) const PROC_PATH: &str = "/proc/bouchaud/metrics";

const PERIOD_NS: u64 = 1_000_000_000;
const PAGE_BYTES: u64 = 4096;

#[derive(Clone, Copy, Default)]
pub(crate) struct Sample {
    pub time_ns: u64,
    pub cpu_online: usize,
    pub cpu_user_ns: u64,
    pub cpu_system_ns: u64,
    pub cpu_idle_ns: u64,
    pub cpu_busy_pct: Option<u64>,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub ata_read_bytes: u64,
    pub ata_read_ops: u64,
    pub ata_read_ns: u64,
    pub ata_read_max_ns: u64,
    pub ata_read_bytes_per_sec: Option<u64>,
    pub barrier_ns: u64,
    pub barrier_max_ns: u64,
    pub scheduler_wait_max_ns: u64,
    pub desktop_frames: u64,
    pub desktop_present_buckets_us: [Option<u64>; 3],
    pub nvme_present: bool,
    pub nvme_reads: u64,
    pub nvme_writes: u64,
    pub nvme_flushes: u64,
    pub virtio_presents: u64,
    pub virtio_errors: u64,
    pub virtio_present_ns: u64,
    pub virtio_present_us: Option<u64>,
    pub virgl_ready: bool,
    pub virgl_frames: u64,
    pub virgl_uploaded_pixels: u64,
}

static LAST: SpinLock<Option<Sample>> = SpinLock::new(None);
static REFRESHING: AtomicBool = AtomicBool::new(false);

fn collect(time_ns: u64, old: Option<Sample>) -> Sample {
    let cpu = task::proc_cpu_publie();
    let (used, total) = crate::kernel::vmm::frame_stats_relaxed();
    let (ata_read_ops, ata_read_bytes, ata_read_ns, ata_read_max_ns) =
        crate::fs::backing::disk_read_timing();
    let (nvme_reads, nvme_writes, nvme_flushes, _, _) = crate::drivers::nvme::stats();
    let (virtio_presents, virtio_errors, _, virtio_present_ns) =
        crate::drivers::virtio_gpu::compteurs();
    let (virgl_frames, _, _, virgl_uploaded_pixels, _, _, virgl_ready) =
        crate::drivers::virtio_gpu::compteurs_composition_gpu();
    let (barrier_ns, barrier_max_ns) = crate::fs::persistance::barrier_timing();
    let (_, scheduler_wait_max_ns, _) = task::compteurs_veille();
    let (desktop_frames, desktop_present_buckets_us) = crate::gui::composition::presentation_stats();
    let mut new = Sample {
        time_ns,
        cpu_online: cpu.online,
        cpu_user_ns: cpu.user_ns,
        cpu_system_ns: cpu.system_ns,
        cpu_idle_ns: cpu.idle_ns,
        ram_used_bytes: used.min(total) * PAGE_BYTES,
        ram_total_bytes: total * PAGE_BYTES,
        ata_read_ops,
        ata_read_bytes,
        ata_read_ns,
        ata_read_max_ns,
        barrier_ns,
        barrier_max_ns,
        scheduler_wait_max_ns,
        desktop_frames,
        desktop_present_buckets_us,
        nvme_present: crate::drivers::nvme::present(),
        nvme_reads,
        nvme_writes,
        nvme_flushes,
        virtio_presents,
        virtio_errors,
        virtio_present_ns,
        virgl_ready,
        virgl_frames,
        virgl_uploaded_pixels,
        ..Sample::default()
    };
    if let Some(old) = old {
        if old.cpu_online == new.cpu_online {
            new.cpu_busy_pct = (|| {
                let user = new.cpu_user_ns.checked_sub(old.cpu_user_ns)?;
                let system = new.cpu_system_ns.checked_sub(old.cpu_system_ns)?;
                let idle = new.cpu_idle_ns.checked_sub(old.cpu_idle_ns)?;
                let busy = user.saturating_add(system);
                busy.saturating_mul(100).checked_div(busy.saturating_add(idle))
            })();
        }
        new.ata_read_bytes_per_sec = new.ata_read_bytes.checked_sub(old.ata_read_bytes)
            .and_then(|bytes| bytes.saturating_mul(PERIOD_NS).checked_div(time_ns - old.time_ns));
        new.virtio_present_us = (|| {
            let frames = new.virtio_presents.checked_sub(old.virtio_presents)?;
            let ns = new.virtio_present_ns.checked_sub(old.virtio_present_ns)?;
            Some(ns.checked_div(frames)? / 1000)
        })();
    }
    new
}

/// None is possible only when the first refresh is already running elsewhere.
pub(crate) fn current() -> Option<Sample> {
    let time_ns = timer::monotonic_ns();
    let old = *LAST.lock();
    if old.is_some_and(|s| time_ns.saturating_sub(s.time_ns) < PERIOD_NS) {
        return old;
    }
    if REFRESHING.swap(true, Ordering::Acquire) {
        return old;
    }
    // A different caller may have finished a refresh before we acquired it.
    let old = *LAST.lock();
    if old.is_some_and(|s| time_ns.saturating_sub(s.time_ns) < PERIOD_NS) {
        REFRESHING.store(false, Ordering::Release);
        return old;
    }
    let sample = collect(time_ns, old);
    *LAST.lock() = Some(sample);
    REFRESHING.store(false, Ordering::Release);
    Some(sample)
}

pub(crate) fn value(value: Option<u64>) -> alloc::string::String {
    value.map_or_else(|| "N/A".into(), |n| format!("{n}"))
}

pub(crate) fn proc_snapshot() -> Vec<u8> {
    let Some(s) = current() else {
        return b"SYSTEM_METRICS version=1 state=refreshing\n".to_vec();
    };
    format!(
        "SYSTEM_METRICS version=1 monotonic_ns={} cpu_online={} cpu_busy_pct={} cpu_user_ns={} cpu_system_ns={} cpu_idle_ns={} ram_used_bytes={} ram_free_bytes={} ram_total_bytes={}\n\
         DISK_METRICS ata_backing_read_ops={} ata_backing_read_bytes={} ata_backing_read_ns={} ata_backing_read_max_ns={} ata_backing_read_bytes_per_sec={} nvme_present={} nvme_reads={} nvme_writes={} nvme_flushes={} persist_barrier_ns={} persist_barrier_max_ns={}\n\
         GPU_METRICS utilization_pct=N/A virgl_ready={} virgl_frames={} virgl_uploaded_pixels={} virtio_presents={} virtio_errors={} virtio_present_ns={} virtio_present_mean_us={} desktop_frames={} desktop_present_p50_bucket_us={} desktop_present_p95_bucket_us={} desktop_present_p99_bucket_us={}\n\
         SCHED_METRICS max_observed_runnable_wait_ns={}\n",
        s.time_ns, s.cpu_online, value(s.cpu_busy_pct), s.cpu_user_ns, s.cpu_system_ns,
        s.cpu_idle_ns, s.ram_used_bytes, s.ram_total_bytes - s.ram_used_bytes, s.ram_total_bytes,
        s.ata_read_ops, s.ata_read_bytes, s.ata_read_ns, s.ata_read_max_ns,
        value(s.ata_read_bytes_per_sec), u8::from(s.nvme_present), s.nvme_reads, s.nvme_writes,
        s.nvme_flushes, s.barrier_ns, s.barrier_max_ns, u8::from(s.virgl_ready), s.virgl_frames, s.virgl_uploaded_pixels,
        s.virtio_presents, s.virtio_errors, s.virtio_present_ns, value(s.virtio_present_us), s.desktop_frames,
        value(s.desktop_present_buckets_us[0]), value(s.desktop_present_buckets_us[1]),
        value(s.desktop_present_buckets_us[2]), s.scheduler_wait_max_ns,
    ).into_bytes()
}
