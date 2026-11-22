//! Shared, rate-limited system snapshots for user space and the desktop.

use crate::kernel::{sync::SpinLock, task, timer};
use alloc::{format, vec::Vec};
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub(crate) const PROC_PATH: &str = "/proc/bouchaud/metrics";

const PERIOD_NS: u64 = 1_000_000_000;
const SERIAL_PERIOD_NS: u64 = 5_000_000_000;
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
    pub ata_present: bool,
    pub ata_read_bytes: u64,
    pub ata_read_ops: u64,
    pub ata_read_ns: u64,
    pub ata_read_max_ns: u64,
    pub ata_read_bytes_per_sec: Option<u64>,
    pub nvme_read_bytes: u64,
    pub nvme_write_bytes: u64,
    pub nvme_read_bytes_per_sec: Option<u64>,
    pub nvme_write_bytes_per_sec: Option<u64>,
    pub persist_payload_bytes: u64,
    pub persist_payload_bytes_per_sec: Option<u64>,
    pub persist_commits: u64,
    pub persist_barriers: u64,
    pub persist_barriers_unavailable: u64,
    pub persist_available: bool,
    pub desktop_backend: &'static str,
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
    let (nvme_read_bytes, nvme_write_bytes) = crate::drivers::nvme::octets_transferes();
    let (persist_payload_bytes, persist_commits, persist_available) = crate::fs::persistance::write_stats();
    let (persist_barriers, persist_barriers_unavailable) = crate::fs::persistance::volume_statistiques();
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
        ata_present: {
            use crate::drivers::ata::{self, Drive};
            ata::present(Drive::Master) || ata::present(Drive::Slave)
        },
        ata_read_ops,
        ata_read_bytes,
        ata_read_ns,
        ata_read_max_ns,
        nvme_read_bytes,
        nvme_write_bytes,
        persist_payload_bytes,
        persist_commits,
        persist_barriers,
        persist_barriers_unavailable,
        persist_available,
        desktop_backend: crate::gui::composition::presentation_backend(),
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
        let rate = |new: u64, old_value: u64| {
            new.checked_sub(old_value)?
                .saturating_mul(PERIOD_NS).checked_div(time_ns.saturating_sub(old.time_ns))
        };
        if old.ata_present && new.ata_present {
            new.ata_read_bytes_per_sec = rate(new.ata_read_bytes, old.ata_read_bytes);
        }
        if old.nvme_present && new.nvme_present {
            new.nvme_read_bytes_per_sec = rate(new.nvme_read_bytes, old.nvme_read_bytes);
            new.nvme_write_bytes_per_sec = rate(new.nvme_write_bytes, old.nvme_write_bytes);
        }
        if old.persist_available && new.persist_available {
            new.persist_payload_bytes_per_sec = rate(new.persist_payload_bytes, old.persist_payload_bytes);
        }
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
    records(s, true).into_bytes()
}

pub(crate) struct Available(pub Option<u64>);

impl fmt::Display for Available {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) => value.fmt(f),
            None => f.write_str("N/A"),
        }
    }
}

impl Sample {
    pub(crate) fn ram_used_pct(self) -> Option<u64> {
        self.ram_used_bytes.saturating_mul(100).checked_div(self.ram_total_bytes)
    }
}

fn records(s: Sample, scheduler: bool) -> alloc::string::String {
    use core::fmt::Write;
    let mut out = format!(
        "SYSTEM_METRICS version=1 monotonic_ns={} cpu_online={} cpu_busy_pct={} cpu_user_ns={} cpu_system_ns={} cpu_idle_ns={} ram_used_bytes={} ram_free_bytes={} ram_total_bytes={} ram_used_pct={}\n\
         DISK_METRICS monotonic_ns={} ata_present={} ata_backing_read_ops={} ata_backing_read_bytes={} ata_backing_read_ns={} ata_backing_read_max_ns={} ata_backing_read_bytes_per_sec={} nvme_present={} nvme_reads={} nvme_writes={} nvme_flushes={} persist_barrier_ns={} persist_barrier_max_ns={} nvme_read_bytes_per_sec={} nvme_write_bytes_per_sec={} persist_payload_bytes_per_sec={} persist_commits={} persist_barriers={} persist_barriers_unavailable={}\n\
         GPU_METRICS monotonic_ns={} desktop_backend={} vulkan_render=N/A utilization_pct=N/A virgl_ready={} virgl_frames={} virgl_uploaded_pixels={} virtio_presents={} virtio_errors={} virtio_present_ns={} virtio_present_mean_us={} desktop_frames={} desktop_present_p50_bucket_us={} desktop_present_p95_bucket_us={} desktop_present_p99_bucket_us={}\n",
        s.time_ns, s.cpu_online, value(s.cpu_busy_pct), s.cpu_user_ns, s.cpu_system_ns,
        s.cpu_idle_ns, s.ram_used_bytes, s.ram_total_bytes - s.ram_used_bytes, s.ram_total_bytes, Available(s.ram_used_pct()),
        s.time_ns, u8::from(s.ata_present), s.ata_read_ops, s.ata_read_bytes, s.ata_read_ns, s.ata_read_max_ns,
        value(s.ata_read_bytes_per_sec), u8::from(s.nvme_present), s.nvme_reads, s.nvme_writes,
        s.nvme_flushes, s.barrier_ns, s.barrier_max_ns,
        Available(s.nvme_read_bytes_per_sec), Available(s.nvme_write_bytes_per_sec),
        Available(s.persist_payload_bytes_per_sec), Available(s.persist_available.then_some(s.persist_commits)),
        Available(s.persist_available.then_some(s.persist_barriers)), Available(s.persist_available.then_some(s.persist_barriers_unavailable)),
        s.time_ns, s.desktop_backend, u8::from(s.virgl_ready), s.virgl_frames, s.virgl_uploaded_pixels,
        s.virtio_presents, s.virtio_errors, s.virtio_present_ns, value(s.virtio_present_us), s.desktop_frames,
        value(s.desktop_present_buckets_us[0]), value(s.desktop_present_buckets_us[1]),
        value(s.desktop_present_buckets_us[2]),
    );
    if scheduler {
        let _ = writeln!(out, "SCHED_METRICS max_observed_runnable_wait_ns={}", s.scheduler_wait_max_ns);
    }
    out
}

static SERIAL_SAMPLE_NS: AtomicU64 = AtomicU64::new(0);
static SERIAL_LOG_NS: AtomicU64 = AtomicU64::new(0);

/// Called by services-metrics in task context. The serial prefix only reads
/// the already formatted snapshot, including during early boot and panics.
pub(crate) fn publish_serial() {
    let now = timer::monotonic_ns();
    let last = SERIAL_SAMPLE_NS.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < PERIOD_NS {
        return;
    }
    let Some(sample) = current() else { return };
    SERIAL_SAMPLE_NS.store(now, Ordering::Relaxed);
    crate::kernel::journal::publish_metrics(sample);
    let last_log = SERIAL_LOG_NS.load(Ordering::Relaxed);
    if last_log == 0 || now.saturating_sub(last_log) >= SERIAL_PERIOD_NS {
        SERIAL_LOG_NS.store(now, Ordering::Relaxed);
        for line in records(sample, false).lines() {
            crate::serial_println!("{}", line);
        }
    }
}
