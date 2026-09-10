//! Flight recorder persistant pour le bring-up physique TRIGKEY.
//!
//! BOUCHAUD_TRIGKEY_BLACKBOX_V2
//!
//! Le chemin normal ne touche jamais le NVMe interne. Les donnees sont
//! ecrites dans la partition GPT brute `BOUCHAUD-BLACKBOX` de la cle USB
//! Stage2, via le transport xHCI Bulk-Only ajoute dans `xhci_active`.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub const KIND_SERIAL: u16 = 1;
pub const KIND_SAMPLE: u16 = 2;
pub const KIND_MARKER: u16 = 3;
pub const KIND_FLIGHT: u16 = 4;
pub const KIND_MEMORY: u16 = 5;
pub const KIND_FATAL: u16 = 9;

const PAYLOAD_MAX: usize = 4032;
const FLIGHT_SLOTS: usize = 8192;
const FLIGHT_EVENT_BYTES: usize = 32;
const FLIGHT_EVENTS_PER_RECORD: usize = PAYLOAD_MAX / FLIGHT_EVENT_BYTES;

const POLL_NS: u64 = 250_000_000;
const SAMPLE_NS: u64 = 250_000_000;
const MEMORY_NS: u64 = 1_000_000_000;

const EVT_TIMER_ENTER: u16 = 1;
const EVT_TIMER_EXIT: u16 = 2;
const EVT_GFX_ENTER: u16 = 10;
const EVT_GFX_EXIT: u16 = 11;
// Un tick sur 16 est journalise dans le flight recorder. Les compteurs et
// TIMER_STAGE restent, eux, mis a jour a CHAQUE IRQ. On conserve donc le
// diagnostic d'un IRQ bloque sans transformer la cle USB en traceur 1 kHz.
const TIMER_EVENT_DIVISOR: u64 = 16;

struct FlightSlot {
    commit: AtomicU64,
    ts_ns: AtomicU64,
    meta: AtomicU64,
    arg: AtomicU64,
}

impl FlightSlot {
    const fn new() -> Self {
        Self {
            commit: AtomicU64::new(0),
            ts_ns: AtomicU64::new(0),
            meta: AtomicU64::new(0),
            arg: AtomicU64::new(0),
        }
    }
}

static FLIGHT: [FlightSlot; FLIGHT_SLOTS] = [const { FlightSlot::new() }; FLIGHT_SLOTS];
static FLIGHT_WRITE: AtomicU64 = AtomicU64::new(0);
static FLIGHT_FLUSHED: AtomicU64 = AtomicU64::new(0);

static BOOT_ID: AtomicU64 = AtomicU64::new(0);
static RECORD_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_TRACE_SEQ: AtomicUsize = AtomicUsize::new(0);

static STARTED: AtomicBool = AtomicBool::new(false);
static LAST_POLL_NS: AtomicU64 = AtomicU64::new(0);
static LAST_SAMPLE_NS: AtomicU64 = AtomicU64::new(0);
static LAST_MEMORY_NS: AtomicU64 = AtomicU64::new(0);

const MAX_CPUS: usize = 64;
static TIMER_STAGE: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static TIMER_ENTERS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static TIMER_EXITS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

#[inline]
fn now_ns() -> u64 {
    crate::kernel::timer::monotonic_ns()
}

#[inline]
fn cpu_clamped(cpu: usize) -> usize {
    cpu.min(MAX_CPUS - 1)
}

#[inline]
fn event(kind: u16, cpu: usize, stage: u32, arg: u64) {
    let seq = FLIGHT_WRITE.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let slot = &FLIGHT[(seq as usize) % FLIGHT_SLOTS];
    let meta = (kind as u64)
        | (((cpu.min(u16::MAX as usize) as u16) as u64) << 16)
        | ((stage as u64) << 32);
    slot.ts_ns.store(now_ns(), Ordering::Relaxed);
    slot.meta.store(meta, Ordering::Relaxed);
    slot.arg.store(arg, Ordering::Relaxed);
    slot.commit.store(seq, Ordering::Release);
}

pub struct TimerIrqGuard {
    cpu: usize,
    recorded: bool,
}

impl Drop for TimerIrqGuard {
    #[inline]
    fn drop(&mut self) {
        let cpu = cpu_clamped(self.cpu);
        TIMER_STAGE[cpu].store(0, Ordering::Release);
        TIMER_EXITS[cpu].fetch_add(1, Ordering::Relaxed);
        if self.recorded {
            event(EVT_TIMER_EXIT, cpu, 0, 0);
        }
    }
}

#[inline]
pub fn timer_irq(cpu: usize, interrupted_rip: u64, interrupted_user: bool) -> TimerIrqGuard {
    let cpu = cpu_clamped(cpu);
    let count = TIMER_ENTERS[cpu].fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    TIMER_STAGE[cpu].store(1, Ordering::Release);
    let recorded = count % TIMER_EVENT_DIVISOR == 0;
    if recorded {
        event(EVT_TIMER_ENTER, cpu, if interrupted_user { 1 } else { 0 }, interrupted_rip);
    }
    TimerIrqGuard { cpu, recorded }
}

#[inline]
pub fn timer_stage(cpu: usize, stage: u32) {
    TIMER_STAGE[cpu_clamped(cpu)].store(stage as u64, Ordering::Release);
}

pub struct GfxPresentGuard {
    cpu: usize,
}

impl Drop for GfxPresentGuard {
    #[inline]
    fn drop(&mut self) {
        event(EVT_GFX_EXIT, self.cpu, 0, 0);
    }
}

#[inline]
pub fn gfx_present(x: usize, y: usize, w: usize, h: usize) -> GfxPresentGuard {
    let cpu = crate::arch::x86_64::usermode::cpu_index();
    let packed = ((x.min(0xffff) as u64) << 48)
        | ((y.min(0xffff) as u64) << 32)
        | ((w.min(0xffff) as u64) << 16)
        | (h.min(0xffff) as u64);
    event(EVT_GFX_ENTER, cpu, 0, packed);
    GfxPresentGuard { cpu }
}

fn make_boot_id() -> u64 {
    let dt = crate::arch::x86_64::rtc::now();
    let date = (dt.year as u64) * 10_000_000_000
        + (dt.month as u64) * 100_000_000
        + (dt.day as u64) * 1_000_000
        + (dt.hour as u64) * 10_000
        + (dt.minute as u64) * 100
        + dt.second as u64;
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    date.saturating_mul(1000).saturating_add(tsc % 1000).max(1)
}

fn boot_id() -> u64 {
    let current = BOOT_ID.load(Ordering::Acquire);
    if current != 0 {
        return current;
    }
    let candidate = make_boot_id();
    match BOOT_ID.compare_exchange(0, candidate, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => candidate,
        Err(value) => value,
    }
}

#[inline]
fn next_record_seq() -> u64 {
    RECORD_SEQ.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
}

fn append(kind: u16, payload: &[u8], ts_ns: u64, trace_end: usize) -> bool {
    crate::drivers::xhci_active::blackbox_append_record(
        kind,
        boot_id(),
        next_record_seq(),
        ts_ns,
        trace_end as u64,
        payload,
    )
}

struct Text {
    bytes: [u8; PAYLOAD_MAX],
    len: usize,
}

impl Text {
    const fn new() -> Self {
        Self { bytes: [0; PAYLOAD_MAX], len: 0 }
    }
    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let data = s.as_bytes();
        let room = self.bytes.len().saturating_sub(self.len);
        let n = room.min(data.len());
        self.bytes[self.len..self.len + n].copy_from_slice(&data[..n]);
        self.len += n;
        if n == data.len() { Ok(()) } else { Err(fmt::Error) }
    }
}

fn flush_flight(ts_ns: u64) {
    let end = FLIGHT_WRITE.load(Ordering::Acquire);
    let mut next = FLIGHT_FLUSHED.load(Ordering::Acquire).saturating_add(1);
    let oldest = end.saturating_sub(FLIGHT_SLOTS as u64).saturating_add(1);
    if next < oldest {
        next = oldest;
    }
    if next == 0 || next > end {
        return;
    }

    let mut payload = [0u8; PAYLOAD_MAX];
    let mut count = 0usize;
    let mut last = next.saturating_sub(1);

    while next <= end && count < FLIGHT_EVENTS_PER_RECORD {
        let slot = &FLIGHT[(next as usize) % FLIGHT_SLOTS];
        if slot.commit.load(Ordering::Acquire) != next {
            next = next.saturating_add(1);
            continue;
        }
        let ts = slot.ts_ns.load(Ordering::Relaxed);
        let meta = slot.meta.load(Ordering::Relaxed);
        let arg = slot.arg.load(Ordering::Relaxed);
        let kind = (meta & 0xffff) as u16;
        let cpu = ((meta >> 16) & 0xffff) as u16;
        let stage = (meta >> 32) as u32;

        let off = count * FLIGHT_EVENT_BYTES;
        payload[off..off + 8].copy_from_slice(&next.to_le_bytes());
        payload[off + 8..off + 16].copy_from_slice(&ts.to_le_bytes());
        payload[off + 16..off + 18].copy_from_slice(&kind.to_le_bytes());
        payload[off + 18..off + 20].copy_from_slice(&cpu.to_le_bytes());
        payload[off + 20..off + 24].copy_from_slice(&stage.to_le_bytes());
        payload[off + 24..off + 32].copy_from_slice(&arg.to_le_bytes());

        count += 1;
        last = next;
        next = next.saturating_add(1);
    }

    if count != 0
        && append(
            KIND_FLIGHT,
            &payload[..count * FLIGHT_EVENT_BYTES],
            ts_ns,
            crate::drivers::serial::trace_total_bytes(),
        )
    {
        FLIGHT_FLUSHED.store(last, Ordering::Release);
    }
}

fn flush_serial(ts_ns: u64, emergency: bool) {
    let (ring_start, ring_end) = crate::drivers::serial::trace_bornes();
    if ring_end == 0 {
        return;
    }

    let mut next = LAST_TRACE_SEQ.load(Ordering::Acquire);
    if next == 0 {
        next = ring_start;
    }
    if next < ring_start {
        let mut gap = Text::new();
        let _ = write!(
            &mut gap,
            "BLACKBOX_SERIAL_GAP lost={} old={} new={}\n",
            ring_start.saturating_sub(next),
            next,
            ring_start,
        );
        let _ = append(KIND_MARKER, gap.as_bytes(), ts_ns, ring_end);
        next = ring_start;
    }

    if emergency {
        let keep = PAYLOAD_MAX.saturating_mul(8);
        next = next.max(ring_end.saturating_sub(keep));
    }

    let max_records = if emergency { 8 } else { 1 };
    for _ in 0..max_records {
        if next >= ring_end {
            break;
        }
        let mut payload = [0u8; PAYLOAD_MAX];
        let n = (ring_end - next).min(PAYLOAD_MAX);
        for i in 0..n {
            payload[i] = crate::drivers::serial::trace_octet(next + i);
        }
        if !append(KIND_SERIAL, &payload[..n], ts_ns, ring_end) {
            break;
        }
        next += n;
        LAST_TRACE_SEQ.store(next, Ordering::Release);
    }
}

fn sample(ts_ns: u64) {
    let mut out = Text::new();
    let (task, syscall, phase, site, aux) = crate::kernel::task::stall_probe_local_context();
    let bkl = crate::kernel::smp_lock::health_snapshot();
    let cpu = crate::arch::x86_64::usermode::cpu_index();
    let rsp: u64;
    let here: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags));
        core::arch::asm!("lea {}, [rip + 0]", out(reg) here, options(nomem, nostack, preserves_flags));
    }
    let (t0, k0) = crate::kernel::task::rips_timer(0);
    let (t1, k1) = crate::kernel::task::rips_timer(1);
    let (t2, k2) = crate::kernel::task::rips_timer(2);
    let (t3, k3) = crate::kernel::task::rips_timer(3);
    let (polls, events, reports, kbd, mouse, hid_errors, rearms, kicks) =
        crate::drivers::xhci_active::hid_transport_stats();
    let (bb_writes, bb_failures, bb_consecutive, bb_last_error, bb_busy_skips, bb_last_ok_ns) =
        crate::drivers::xhci_active::blackbox_storage_extended_counters();

    let _ = write!(
        &mut out,
        concat!(
            "sample ts_ns={} cpu={} rsp={:#x} here={:#x} ",
            "task={} syscall={} phase={} site={} aux={:#x} need_resched={} ",
            "bkl_owner={} bkl_cpu={} bkl_depth={} bkl_ok={} parked={:#x} resume={:#x} ",
            "timer0={:#x}/{:#x}/stage{}/{}:{} ",
            "timer1={:#x}/{:#x}/stage{}/{}:{} ",
            "timer2={:#x}/{:#x}/stage{}/{}:{} ",
            "timer3={:#x}/{:#x}/stage{}/{}:{} ",
            "hid polls={} events={} reports={} kbd={} mouse={} errors={} rearms={} kicks={} ",
            "bb_writes={} bb_failures={} bb_consecutive={} bb_last_error={} ",
            "bb_busy_skips={} bb_last_ok_ns={}\n"
        ),
        ts_ns, cpu, rsp, here,
        task, syscall, phase, site, aux,
        crate::kernel::task::besoin_de_replanifier() as u8,
        bkl.owner_token, bkl.owner_cpu, bkl.owner_depth, bkl.owner_depth_ok as u8,
        bkl.parked_mask, bkl.resume_mask,
        t0, k0, TIMER_STAGE[0].load(Ordering::Acquire),
        TIMER_ENTERS[0].load(Ordering::Relaxed), TIMER_EXITS[0].load(Ordering::Relaxed),
        t1, k1, TIMER_STAGE[1].load(Ordering::Acquire),
        TIMER_ENTERS[1].load(Ordering::Relaxed), TIMER_EXITS[1].load(Ordering::Relaxed),
        t2, k2, TIMER_STAGE[2].load(Ordering::Acquire),
        TIMER_ENTERS[2].load(Ordering::Relaxed), TIMER_EXITS[2].load(Ordering::Relaxed),
        t3, k3, TIMER_STAGE[3].load(Ordering::Acquire),
        TIMER_ENTERS[3].load(Ordering::Relaxed), TIMER_EXITS[3].load(Ordering::Relaxed),
        polls, events, reports, kbd, mouse, hid_errors, rearms, kicks,
        bb_writes, bb_failures, bb_consecutive, bb_last_error,
        bb_busy_skips, bb_last_ok_ns,
    );
    let _ = append(KIND_SAMPLE, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
}

fn memory_sample(ts_ns: u64) {
    let d = crate::kernel::dalles_tas::stats();
    let (frames_used, frames_total) = crate::kernel::vmm::frame_stats_relaxed();
    let mut out = Text::new();
    let _ = write!(
        &mut out,
        concat!(
            "memory ts_ns={} frames_used={} frames_total={} ",
            "dalles={} vides={} vivants={} candidats={} manques={} saturations={} ",
            "sous_flux={} surallocations={} max_probe={}\n"
        ),
        ts_ns,
        frames_used,
        frames_total,
        d.enregistrees,
        d.dalles_vides,
        d.objets_vivants,
        d.candidats_vides,
        d.manques,
        d.saturations,
        d.sous_flux,
        d.surallocations,
        d.max_probe,
    );
    let _ = append(KIND_MEMORY, out.as_bytes(), ts_ns, crate::drivers::serial::trace_total_bytes());
}

pub fn poll() {
    if !crate::drivers::xhci_active::blackbox_storage_ready() {
        return;
    }
    let now = now_ns();
    let previous = LAST_POLL_NS.load(Ordering::Relaxed);
    if previous != 0 && now.saturating_sub(previous) < POLL_NS {
        return;
    }
    if LAST_POLL_NS
        .compare_exchange(previous, now, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    if !STARTED.load(Ordering::Acquire) {
        let mut msg = Text::new();
        let _ = write!(
            &mut msg,
            "BOUCHAUD_TRIGKEY_BLACKBOX_V1 START boot_id={} ts_ns={}\n",
            boot_id(),
            now,
        );
        if append(KIND_MARKER, msg.as_bytes(), now, crate::drivers::serial::trace_total_bytes()) {
            STARTED.store(true, Ordering::Release);
        }
    }

    flush_flight(now);
    flush_flight(now);
    flush_serial(now, false);

    let last_sample = LAST_SAMPLE_NS.load(Ordering::Relaxed);
    if last_sample == 0 || now.saturating_sub(last_sample) >= SAMPLE_NS {
        LAST_SAMPLE_NS.store(now, Ordering::Relaxed);
        sample(now);
    }

    let last_memory = LAST_MEMORY_NS.load(Ordering::Relaxed);
    if last_memory == 0 || now.saturating_sub(last_memory) >= MEMORY_NS {
        LAST_MEMORY_NS.store(now, Ordering::Relaxed);
        memory_sample(now);
    }
}

pub fn fatal_best_effort(cpu: usize, vector: u8, rip: u64, rsp: u64, code: u64) {
    if !crate::drivers::xhci_active::blackbox_storage_ready() {
        return;
    }
    let now = now_ns();
    let mut out = Text::new();
    let _ = write!(
        &mut out,
        "FATAL cpu={} vector={} rip={:#x} rsp={:#x} code={:#x} ts_ns={}\n",
        cpu, vector, rip, rsp, code, now,
    );
    let _ = append(KIND_FATAL, out.as_bytes(), now, crate::drivers::serial::trace_total_bytes());
    flush_serial(now, true);
    crate::drivers::xhci_active::blackbox_force_sync();
}
