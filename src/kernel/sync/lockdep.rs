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
/// Instant de prise, par emplacement de pile. Sert a la duree de DETENTION.
static DEBUT_DETENTION_NS: [[AtomicU64; MAX_HELD]; smp::MAX_CPUS] =
    [const { [const { AtomicU64::new(0) }; MAX_HELD] }; smp::MAX_CPUS];
/// Les interruptions etaient-elles masquees a la prise de cet emplacement ?
static IRQ_MASQUEES: [[AtomicU32; MAX_HELD]; smp::MAX_CPUS] =
    [const { [const { AtomicU32::new(0) }; MAX_HELD] }; smp::MAX_CPUS];
/// Instant du dernier `before_acquire` de ce CPU. Sert a la duree d'ATTENTE.
static DEBUT_ATTENTE_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];

/// Plus longue attente observee entre `before_acquire` et `acquired`.
static ATTENTE_MAX_NS: AtomicU64 = AtomicU64::new(0);
/// Plus longue detention observee, toutes classes confondues.
static DETENTION_MAX_NS: AtomicU64 = AtomicU64::new(0);
/// Classe qui a produit `DETENTION_MAX_NS`. Un maximum sans coupable oblige a
/// le chercher ; avec, il se lit.
static DETENTION_MAX_CLASSE: AtomicU32 = AtomicU32::new(0);
/// Plus longue detention INTERRUPTIONS MASQUEES.
///
/// C'est la mesure qui compte le plus pour la reactivite : pendant ce temps, ce
/// coeur ne prend ni tick, ni entree, ni achevement. Une detention ordinaire
/// ralentit ceux qui attendent le verrou ; une detention IRQ-off ralentit tout
/// ce que le coeur aurait du servir.
static DETENTION_IRQ_OFF_MAX_NS: AtomicU64 = AtomicU64::new(0);

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
    // La date est posee AVANT le retour anticipe : une prise sans verrou deja
    // tenu est justement celle qui peut attendre le plus longtemps, et la
    // manquer viderait la mesure de son cas le plus interessant.
    DEBUT_ATTENTE_NS[c].store(maintenant_ns(), Ordering::Release);
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
    let maintenant = maintenant_ns();
    let debut_attente = DEBUT_ATTENTE_NS[c].swap(0, Ordering::AcqRel);
    if debut_attente != 0 {
        ATTENTE_MAX_NS.fetch_max(maintenant.saturating_sub(debut_attente), Ordering::Relaxed);
    }
    STACK[c][depth].store(class.rank() as u32, Ordering::Release);
    DEBUT_DETENTION_NS[c][depth].store(maintenant, Ordering::Release);
    IRQ_MASQUEES[c][depth].store(!interruptions_actives() as u32, Ordering::Release);
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
    let debut = DEBUT_DETENTION_NS[c][index].swap(0, Ordering::AcqRel);
    if debut != 0 {
        let tenue = maintenant_ns().saturating_sub(debut);
        if tenue > DETENTION_MAX_NS.fetch_max(tenue, Ordering::AcqRel) {
            // La classe est publiee APRES le maximum : un lecteur qui les voit
            // desaccordes lit un maximum plus recent que sa classe, jamais une
            // classe qui n'a jamais tenu ce maximum.
            DETENTION_MAX_CLASSE.store(class.rank() as u32, Ordering::Release);
        }
        if IRQ_MASQUEES[c][index].swap(0, Ordering::AcqRel) != 0 {
            DETENTION_IRQ_OFF_MAX_NS.fetch_max(tenue, Ordering::Relaxed);
        }
    }
    STACK[c][index].store(0, Ordering::Relaxed);
    DEPTH[c].store(index, Ordering::Release);
}

/// L'horloge, isolee pour que la preuve hote puisse la remplacer.
#[inline]
fn maintenant_ns() -> u64 {
    crate::kernel::timer::monotonic_ns()
}

/// Les interruptions sont-elles actives sur ce coeur ?
#[inline]
fn interruptions_actives() -> bool {
    crate::arch::x86_64::cpu::interrupts_enabled()
}

/// Attente maximale, detention maximale, sa classe, et detention IRQ-off
/// maximale -- en nanosecondes.
pub fn temps() -> (u64, u64, u16, u64) {
    (
        ATTENTE_MAX_NS.load(Ordering::Relaxed),
        DETENTION_MAX_NS.load(Ordering::Relaxed),
        DETENTION_MAX_CLASSE.load(Ordering::Relaxed) as u16,
        DETENTION_IRQ_OFF_MAX_NS.load(Ordering::Relaxed),
    )
}

pub fn depth() -> usize { DEPTH[cpu()].load(Ordering::Acquire) }

/// Remet la pile de ce CPU a zero.
///
/// # Pourquoi cette fonction existe
///
/// Une violation detectee panique en construction de debogage, et la panique
/// laisse la pile du CPU dans l'etat ou elle etait -- c'est voulu : le releve
/// de faute doit pouvoir la lire.
///
/// La preuve hote, elle, enchaine plusieurs cas dans un MEME binaire, donc sur
/// les memes statiques. Sans remise a zero, le premier cas qui panique
/// fausserait tous les suivants, et la suite ne prouverait plus ce qu'elle
/// annonce. C'est le seul appelant, et le nom le dit.
#[doc(hidden)]
pub fn reinitialise_pour_preuve() {
    let c = cpu();
    for index in 0..MAX_HELD {
        STACK[c][index].store(0, Ordering::Relaxed);
        DEBUT_DETENTION_NS[c][index].store(0, Ordering::Relaxed);
        IRQ_MASQUEES[c][index].store(0, Ordering::Relaxed);
    }
    DEBUT_ATTENTE_NS[c].store(0, Ordering::Relaxed);
    DEPTH[c].store(0, Ordering::Release);
}

pub fn stats() -> Stats {
    Stats {
        acquisitions: ACQUISITIONS.load(Ordering::Relaxed),
        violations: VIOLATIONS.load(Ordering::Relaxed),
        max_depth: MAX_DEPTH.load(Ordering::Relaxed),
    }
}

pub fn log_stats() {
    let s = stats();
    let (attente, detention, classe, irq_off) = temps();
    crate::serial_println!(
        "[LOCKDEP] acquisitions={} violations={} max_depth={} \
         attente_max_ns={} detention_max_ns={} detention_max_rang={} \
         detention_irq_off_max_ns={}",
        s.acquisitions, s.violations, s.max_depth,
        attente, detention, classe, irq_off
    );
}
