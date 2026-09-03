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
// BOUCHAUD_C2_REPORT_REFUSE_V1
//
// Instant du PREMIER refus servi a une demande de preemption encore pendante.
// Zero tant qu'aucun point sur ne l'a refusee.
//
// # Pourquoi ce compteur n'est pas `REQUEST_NS`
//
// `REQUEST_NS` date la DEMANDE. L'ecart entre la demande et le service
// contient donc tout le temps ou ce coeur n'a simplement rien execute : sur un
// coeur au repos, personne n'appelle `safe_point`, et la mesure grandit sans
// que personne n'attende. Une trace SMP4 mesuree l'a montre -- 2,1 s de
// « report » avec `blocked_*=0`, c'est-a-dire pas un seul refus, pendant que la
// latence prete->coeur plafonnait a 19,8 ms. Le chiffre ne mesurait pas un
// figement : il mesurait de l'inactivite.
//
// Ce qu'un budget doit borner, c'est le report SUBI : une preemption demandee,
// un point sur atteint, et le refus qui s'ensuit -- verrou global tenu,
// section critique, contexte d'interruption. Cela seul est du ressort des
// chantiers 1 et 2, et cela seul peut figer une interface.
static REFUS_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static MAX_DEFER_NS: AtomicU64 = AtomicU64::new(0);
static MAX_ATTENTE_NS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub requests: u64,
    pub safe_points: u64,
    pub switches: u64,
    pub blocked_bkl: u64,
    pub blocked_preempt: u64,
    pub blocked_context: u64,
    /// Plus long report SUBI : du premier refus au service effectif.
    pub max_defer_ns: u64,
    /// Plus longue attente demande->service, INACTIVITE COMPRISE. Diagnostic
    /// seul : sur un coeur au repos elle grandit sans que personne n'attende,
    /// et elle n'est donc bornee par aucun budget.
    pub max_attente_ns: u64,
}

/// Date le premier refus d'une demande encore pendante, et lui seul.
#[inline]
fn note_refus(index: usize) {
    let _ = REFUS_NS[index].compare_exchange(
        0,
        crate::kernel::timer::monotonic_ns(),
        Ordering::AcqRel,
        Ordering::Relaxed,
    );
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
        note_refus(index);
        return false;
    }
    if local.preempt_count() != 0 || crate::kernel::sync::lockdep::depth() != 0 {
        BLOCKED_PREEMPT.fetch_add(1, Ordering::Relaxed);
        note_refus(index);
        return false;
    }
    if crate::kernel::smp_lock::held_by_current_cpu() {
        BLOCKED_BKL.fetch_add(1, Ordering::Relaxed);
        note_refus(index);
        return false;
    }
    if !crate::kernel::task::in_user_task() || crate::kernel::task::current_is_kernel_task() {
        BLOCKED_CONTEXT.fetch_add(1, Ordering::Relaxed);
        note_refus(index);
        return false;
    }

    local.clear_resched();
    let requested = REQUEST_NS[index].swap(0, Ordering::AcqRel);
    let refuse = REFUS_NS[index].swap(0, Ordering::AcqRel);
    let maintenant = crate::kernel::timer::monotonic_ns();
    // Le BUDGET porte sur le report subi : sans refus, il n'y a rien a borner.
    if refuse != 0 {
        MAX_DEFER_NS.fetch_max(maintenant.saturating_sub(refuse), Ordering::Relaxed);
    }
    if requested != 0 {
        MAX_ATTENTE_NS.fetch_max(maintenant.saturating_sub(requested), Ordering::Relaxed);
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
        max_attente_ns: MAX_ATTENTE_NS.load(Ordering::Relaxed),
    }
}

pub fn log_stats() {
    let s = stats();
    crate::serial_println!(
        "[SCHED-NG-PREEMPT] requests={} safe={} switches={} blocked_bkl={} blocked_preempt={} blocked_ctx={} max_defer_ns={} attente_service_max_ns={}",
        s.requests, s.safe_points, s.switches, s.blocked_bkl,
        s.blocked_preempt, s.blocked_context, s.max_defer_ns, s.max_attente_ns
    );
}
