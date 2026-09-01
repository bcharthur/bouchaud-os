//! Deferred kernel preemption at explicit safe points.
//!
//! Bouchaud P0-NG1 deliberately does not switch stacks from arbitrary ring-0
//! interrupt contexts. Timer/IPI code requests a reschedule; code that reaches
//! a safe boundary performs it only when interrupts are enabled, the BKL is not
//! held, no ranked critical section is active and the current task is a normal
//! user task temporarily executing in the kernel.

use core::sync::atomic::{AtomicU64, Ordering};
use crate::arch::x86_64::{cpu, cpu_local::{self, CpuId}, smp};

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static SAFE_POINTS: AtomicU64 = AtomicU64::new(0);
static SWITCHES: AtomicU64 = AtomicU64::new(0);
static BLOCKED_BKL: AtomicU64 = AtomicU64::new(0);
static BLOCKED_PREEMPT: AtomicU64 = AtomicU64::new(0);
static BLOCKED_CONTEXT: AtomicU64 = AtomicU64::new(0);
static REQUEST_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static MAX_DEFER_NS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub requests: u64,
    pub safe_points: u64,
    pub switches: u64,
    pub blocked_bkl: u64,
    pub blocked_preempt: u64,
    pub blocked_context: u64,
    pub max_defer_ns: u64,
}

#[inline]
fn local_id() -> Option<CpuId> { CpuId::from_index(smp::cpu_index()) }

pub fn disable() {
    if let Some(id) = local_id() { cpu_local::local(id).preempt_disable(); }
}

pub fn enable() {
    if let Some(id) = local_id() { cpu_local::local(id).preempt_enable(); }
}

pub fn request_local() { request_cpu(smp::cpu_index()); }

pub fn request_cpu(cpu_index: usize) {
    let Some(id) = CpuId::from_index(cpu_index) else { return; };
    let local = cpu_local::local(id);
    if !local.need_resched() {
        REQUEST_NS[cpu_index].store(crate::kernel::timer::monotonic_ns(), Ordering::Release);
        REQUESTS.fetch_add(1, Ordering::Relaxed);
    }
    local.request_resched();
}

pub fn pending() -> bool {
    local_id().map(|id| cpu_local::local(id).need_resched()).unwrap_or(false)
}

/// Execute a deferred reschedule only at a fully preemptible kernel boundary.
pub fn safe_point() -> bool {
    let Some(id) = local_id() else { return false; };
    let index = id.as_usize();
    let local = cpu_local::local(id);
    if !local.need_resched() { return false; }
    SAFE_POINTS.fetch_add(1, Ordering::Relaxed);

    if !cpu::interrupts_enabled() || local.irq_depth() != 0 {
        BLOCKED_CONTEXT.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if local.preempt_count() != 0 || crate::kernel::sync::lockdep::depth() != 0 {
        BLOCKED_PREEMPT.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if crate::kernel::smp_lock::held_by_current_cpu() {
        BLOCKED_BKL.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if !crate::kernel::task::in_user_task() || crate::kernel::task::current_is_kernel_task() {
        BLOCKED_CONTEXT.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    local.clear_resched();
    let requested = REQUEST_NS[index].swap(0, Ordering::AcqRel);
    if requested != 0 {
        MAX_DEFER_NS.fetch_max(
            crate::kernel::timer::monotonic_ns().saturating_sub(requested),
            Ordering::Relaxed,
        );
    }
    let switched = crate::kernel::task::schedule();
    if switched { SWITCHES.fetch_add(1, Ordering::Relaxed); }
    switched
}

pub fn cond_resched() -> bool { safe_point() }

pub fn stats() -> Stats {
    Stats {
        requests: REQUESTS.load(Ordering::Relaxed),
        safe_points: SAFE_POINTS.load(Ordering::Relaxed),
        switches: SWITCHES.load(Ordering::Relaxed),
        blocked_bkl: BLOCKED_BKL.load(Ordering::Relaxed),
        blocked_preempt: BLOCKED_PREEMPT.load(Ordering::Relaxed),
        blocked_context: BLOCKED_CONTEXT.load(Ordering::Relaxed),
        max_defer_ns: MAX_DEFER_NS.load(Ordering::Relaxed),
    }
}

pub fn log_stats() {
    let s = stats();
    crate::serial_println!(
        "[SCHED-NG-PREEMPT] requests={} safe={} switches={} blocked_bkl={} blocked_preempt={} blocked_ctx={} max_defer_ns={}",
        s.requests, s.safe_points, s.switches, s.blocked_bkl,
        s.blocked_preempt, s.blocked_context, s.max_defer_ns
    );
}
