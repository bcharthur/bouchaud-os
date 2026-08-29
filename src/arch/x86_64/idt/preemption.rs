// V8 hard-IRQ preemption gate and observability.
//
// Why this exists:
// `stall_site=41` used to survive some early returns from preempt_from_irq().
// That made later, unrelated BKL ownership look like it had been acquired by
// the preemption path. V8 clears the site at the IDT boundary on every return.
//
// In addition, CPU0/BSP temporarily never performs an immediate context switch
// from a hardware IRQ. It requests the already-existing deferred preemption
// instead. APs retain direct IRQ preemption.
//
// This is a P0 diagnostic/mitigation mode. It deliberately tests whether the
// BSP hard-IRQ preemption path is the remaining source of global desktop
// stalls. It is not intended to be the final scheduler policy.

const PREEMPT_SOURCE_TIMER: u8 = 1;
const PREEMPT_SOURCE_RESCHEDULE: u8 = 2;

/// P0 diagnostic: direct `preempt_from_irq()` is disabled on logical CPU0.
/// The next syscall return consumes NEED_RESCHED through the existing Linux
/// dispatcher. AP CPUs keep immediate IRQ preemption.
pub const BSP_DEFER_DIRECT_IRQ_PREEMPT_V8: bool = true;

static PREEMPT_IRQ_REQUESTS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static PREEMPT_IRQ_DIRECT_CALLS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static PREEMPT_IRQ_DIRECT_RETURNS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static PREEMPT_IRQ_BSP_DEFERRED: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static PREEMPT_IRQ_SITE_CLEARS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static PREEMPT_IRQ_MAX_CONTINUATION_NS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

static PREEMPT_IRQ_ACTIVE: [core::sync::atomic::AtomicBool; smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicBool::new(false) }; smp::MAX_CPUS];
static PREEMPT_IRQ_ACTIVE_SINCE_NS: [core::sync::atomic::AtomicU64; smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicU64::new(0) }; smp::MAX_CPUS];
static PREEMPT_IRQ_LAST_RETURN_NS: [core::sync::atomic::AtomicU64; smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicU64::new(0) }; smp::MAX_CPUS];
static PREEMPT_IRQ_LAST_SOURCE: [core::sync::atomic::AtomicU8; smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicU8::new(0) }; smp::MAX_CPUS];

#[inline]
fn preempt_source_name(source: u8) -> &'static str {
    match source {
        PREEMPT_SOURCE_TIMER => "pit",
        PREEMPT_SOURCE_RESCHEDULE => "resched-ipi",
        _ => "none",
    }
}

/// IDT-owned boundary around task::preempt_from_irq().
///
/// Important: a direct call can switch stacks and only return when the
/// *preempted* IRQ continuation is scheduled again. Therefore `continuation_ns`
/// is not a BKL hold duration. The BKL code must be read separately. What this
/// lifetime tells us is whether an IRQ continuation is still outstanding.
#[inline]
fn dispatch_irq_preempt(source: u8) {
    use core::sync::atomic::Ordering;

    PREEMPT_IRQ_REQUESTS.fetch_add(1, Ordering::Relaxed);
    let cpu = crate::arch::x86_64::cpu::hardware_cpu_index();

    if cpu == 0 && BSP_DEFER_DIRECT_IRQ_PREEMPT_V8 {
        PREEMPT_IRQ_BSP_DEFERRED.fetch_add(1, Ordering::Relaxed);
        crate::kernel::task::request_deferred_preempt();

        // Fix the diagnostic leak even if an older early-return path left a
        // task-local site marker behind.
        crate::kernel::task::stall_site_clear();
        PREEMPT_IRQ_SITE_CLEARS.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let now = crate::kernel::timer::monotonic_ns();
    PREEMPT_IRQ_DIRECT_CALLS.fetch_add(1, Ordering::Relaxed);
    PREEMPT_IRQ_LAST_SOURCE[cpu].store(source, Ordering::Release);
    PREEMPT_IRQ_ACTIVE_SINCE_NS[cpu].store(now, Ordering::Release);
    PREEMPT_IRQ_ACTIVE[cpu].store(true, Ordering::Release);

    crate::kernel::task::preempt_from_irq();

    // This boundary executes on every normal/early return from the task layer.
    // It closes the stale `site=41` ambiguity that V7 exposed.
    crate::kernel::task::stall_site_clear();
    PREEMPT_IRQ_SITE_CLEARS.fetch_add(1, Ordering::Relaxed);

    let done = crate::kernel::timer::monotonic_ns();
    let elapsed = done.saturating_sub(now);
    PREEMPT_IRQ_MAX_CONTINUATION_NS.fetch_max(elapsed, Ordering::Relaxed);
    PREEMPT_IRQ_LAST_RETURN_NS[cpu].store(done, Ordering::Release);
    PREEMPT_IRQ_ACTIVE[cpu].store(false, Ordering::Release);
    PREEMPT_IRQ_DIRECT_RETURNS.fetch_add(1, Ordering::Relaxed);
}

/// Periodic, non-IRQ serial report. Called through the existing V6 diagnostic.
pub fn log_preempt_irq_diagnostic() {
    use core::sync::atomic::Ordering;

    let now = crate::kernel::timer::monotonic_ns();
    let provenance = crate::kernel::smp_lock::stall_probe_provenance();

    crate::serial_println!(
        "[PREEMPT-IRQ] bsp_defer={} requests={} direct={}/{} bsp_deferred={} site_clears={} continuation_max_ns={} bkl_owner={} bkl_cpu={} bkl_site={} bkl_kind={}",
        BSP_DEFER_DIRECT_IRQ_PREEMPT_V8 as u8,
        PREEMPT_IRQ_REQUESTS.load(Ordering::Relaxed),
        PREEMPT_IRQ_DIRECT_CALLS.load(Ordering::Relaxed),
        PREEMPT_IRQ_DIRECT_RETURNS.load(Ordering::Relaxed),
        PREEMPT_IRQ_BSP_DEFERRED.load(Ordering::Relaxed),
        PREEMPT_IRQ_SITE_CLEARS.load(Ordering::Relaxed),
        PREEMPT_IRQ_MAX_CONTINUATION_NS.load(Ordering::Relaxed),
        provenance.owner_token,
        if provenance.owner_token == 0 { usize::MAX } else { provenance.owner_token - 1 },
        provenance.site,
        provenance.acquire_kind,
    );

    let online = smp::schedulable_cpus().max(1).min(smp::MAX_CPUS);
    for cpu in 0..online {
        let active = PREEMPT_IRQ_ACTIVE[cpu].load(Ordering::Acquire);
        let since = PREEMPT_IRQ_ACTIVE_SINCE_NS[cpu].load(Ordering::Acquire);
        let last_return = PREEMPT_IRQ_LAST_RETURN_NS[cpu].load(Ordering::Acquire);
        let source = PREEMPT_IRQ_LAST_SOURCE[cpu].load(Ordering::Acquire);
        crate::serial_println!(
            "[PREEMPT-CPU] cpu={} active={} source={} active_age_ns={} last_return_age_ns={}",
            cpu,
            active as u8,
            preempt_source_name(source),
            if active && since != 0 { now.saturating_sub(since) } else { 0 },
            if last_return != 0 { now.saturating_sub(last_return) } else { 0 },
        );
    }
}
