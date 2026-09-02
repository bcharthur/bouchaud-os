//! Runtime lock-order diagnostics for Bouchaud OS P0-NG1.
//!
//! The static verifier in `ordre_verrous` proves selected model traces. This
//! module complements it at runtime: every ranked lock publishes its class on a
//! per-CPU stack and an inversion becomes an immediate, attributable failure in
//! debug builds instead of a silent SMP deadlock.

use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use crate::arch::x86_64::smp;

const MAX_HELD: usize = 16;

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LockClass {
    /// Transition locale de l'ordonnanceur. Elle ne protege aucune donnee
    /// globale : elle interdit seulement qu'une IRQ reprenne le changement de
    /// contexte du CPU qu'elle vient d'interrompre.
    SchedulerTransition = 1,
    /// Alarmes POSIX consultees par le scheduler apres la transition locale.
    SchedulerAlarms = 5,
    TaskTable = 10,
    ProcessTable = 20,
    Process = 30,
    FdTable = 40,
    PosixRecord = 45,
    Vfs = 50,
    PageCache = 60,
    Vm = 70,
    Network = 80,
    Driver = 90,
    Persistence = 100,
}

impl LockClass {
    pub const fn rank(self) -> u16 { self as u16 }
    pub const fn name(self) -> &'static str {
        match self {
            Self::SchedulerTransition => "scheduler-transition",
            Self::SchedulerAlarms => "scheduler-alarms",
            Self::TaskTable => "task-table",
            Self::ProcessTable => "process-table",
            Self::Process => "process",
            Self::FdTable => "fd-table",
            Self::PosixRecord => "posix-record",
            Self::Vfs => "vfs",
            Self::PageCache => "page-cache",
            Self::Vm => "vm",
            Self::Network => "network",
            Self::Driver => "driver",
            Self::Persistence => "persistence",
        }
    }
}

static DEPTH: [AtomicUsize; smp::MAX_CPUS] =
    [const { AtomicUsize::new(0) }; smp::MAX_CPUS];
static STACK: [[AtomicU32; MAX_HELD]; smp::MAX_CPUS] =
    [const { [const { AtomicU32::new(0) }; MAX_HELD] }; smp::MAX_CPUS];
static ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
static VIOLATIONS: AtomicU64 = AtomicU64::new(0);
static MAX_DEPTH: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub acquisitions: u64,
    pub violations: u64,
    pub max_depth: usize,
}

#[inline]
fn cpu() -> usize { smp::cpu_index().min(smp::MAX_CPUS - 1) }

#[track_caller]
pub fn before_acquire(class: LockClass) {
    let c = cpu();
    let depth = DEPTH[c].load(Ordering::Acquire);
    if depth == 0 { return; }
    let previous = STACK[c][depth.min(MAX_HELD) - 1].load(Ordering::Acquire) as u16;
    if previous >= class.rank() {
        VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        #[cfg(debug_assertions)]
        panic!(
            "LOCKDEP inversion cpu={} held_rank={} acquiring={}({}) at {}:{}",
            c,
            previous,
            class.name(),
            class.rank(),
            core::panic::Location::caller().file(),
            core::panic::Location::caller().line(),
        );
    }
}

pub fn acquired(class: LockClass) {
    let c = cpu();
    let depth = DEPTH[c].load(Ordering::Relaxed);
    if depth >= MAX_HELD {
        VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        #[cfg(debug_assertions)]
        panic!("LOCKDEP stack overflow cpu={} depth={}", c, depth);
        #[cfg(not(debug_assertions))]
        return;
    }
    STACK[c][depth].store(class.rank() as u32, Ordering::Release);
    DEPTH[c].store(depth + 1, Ordering::Release);
    ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
    MAX_DEPTH.fetch_max(depth + 1, Ordering::Relaxed);
}

pub fn released(class: LockClass) {
    let c = cpu();
    let depth = DEPTH[c].load(Ordering::Acquire);
    if depth == 0 {
        VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        #[cfg(debug_assertions)]
        panic!("LOCKDEP release without acquisition: {}", class.name());
        #[cfg(not(debug_assertions))]
        return;
    }
    let index = depth - 1;
    let actual = STACK[c][index].load(Ordering::Acquire) as u16;
    if actual != class.rank() {
        VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        #[cfg(debug_assertions)]
        panic!(
            "LOCKDEP non-LIFO release cpu={} expected_rank={} actual_rank={}",
            c, actual, class.rank()
        );
    }
    STACK[c][index].store(0, Ordering::Relaxed);
    DEPTH[c].store(index, Ordering::Release);
}

pub fn depth() -> usize { DEPTH[cpu()].load(Ordering::Acquire) }

pub fn stats() -> Stats {
    Stats {
        acquisitions: ACQUISITIONS.load(Ordering::Relaxed),
        violations: VIOLATIONS.load(Ordering::Relaxed),
        max_depth: MAX_DEPTH.load(Ordering::Relaxed),
    }
}

pub fn log_stats() {
    let s = stats();
    crate::serial_println!(
        "[LOCKDEP] acquisitions={} violations={} max_depth={}",
        s.acquisitions, s.violations, s.max_depth
    );
}
