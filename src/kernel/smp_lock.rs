//! Verrou noyau global reentrant pour le premier scheduler SMP de Bouchaud OS.
//!
//! Les structures historiques du noyau (`Rc<RefCell<_>>`, allocateur, RAMFS,
//! pilotes) ont ete ecrites avec l'invariant UP. Le passage immediat a des
//! verrous fins partout serait un chantier beaucoup plus vaste que le scheduler
//! lui-meme. Cette couche fournit donc un Big Kernel Lock reentrant par CPU :
//! plusieurs coeurs executent du ring 3 en parallele, mais un seul manipule les
//! structures noyau globales a la fois.
//!
//! Le scheduler peut relacher temporairement le verrou autour d'un changement de
//! contexte. Le nombre de prises reentrantes est restaure lorsque la tache reprend.
//!
//! IMPORTANT SMP/IRQ : OWNER et DEPTH forment ensemble l'etat du verrou.
//! Une IRQ locale ne doit jamais observer l'etat intermediaire entre leur mise a
//! jour. On masque donc les IRQ uniquement pendant ces tres courtes transitions.
//! On ne garde jamais les IRQ masquees pendant l'attente d'un OWNER distant.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use x86_64::instructions::interrupts;

pub const MAX_CPUS: usize = 16;
const FREE: usize = 0;

static OWNER: AtomicUsize = AtomicUsize::new(FREE);
static DEPTH: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

// BOUCHAUD_SMP4_OWNER_PROVENANCE_PROBE_V3
// Provenance du Big Kernel Lock. Aucun log n'est emis dans le hot path :
// on ne fait que mettre a jour des atomiques. Le PIT imprime un snapshot
// une fois par seconde, ce qui limite fortement l'effet Heisenberg.
// kind acquire : 1=enter, 2=try_enter, 3=resume_after_schedule.
// kind release : 1=Drop/release_one, 2=suspend_for_schedule.
static PROBE_ACQUIRE_SEQ: AtomicU64 = AtomicU64::new(0);
static PROBE_RELEASE_SEQ: AtomicU64 = AtomicU64::new(0);
static PROBE_REENTER_SEQ: AtomicU64 = AtomicU64::new(0);
static PROBE_OWNER_GEN: AtomicU64 = AtomicU64::new(0);
static PROBE_OWNER_SINCE: AtomicU64 = AtomicU64::new(0);
static PROBE_OWNER_KIND: AtomicU32 = AtomicU32::new(0);
static PROBE_OWNER_TASK: AtomicUsize = AtomicUsize::new(usize::MAX);
static PROBE_OWNER_SYSCALL: AtomicU64 = AtomicU64::new(u64::MAX);
static PROBE_OWNER_SYSCALL_PHASE: AtomicU32 = AtomicU32::new(0);
static PROBE_OWNER_SITE: AtomicU32 = AtomicU32::new(0);
static PROBE_OWNER_AUX: AtomicU64 = AtomicU64::new(0);
static PROBE_LAST_RELEASE_TICK: AtomicU64 = AtomicU64::new(0);
static PROBE_LAST_RELEASE_CPU: AtomicUsize = AtomicUsize::new(usize::MAX);
static PROBE_LAST_RELEASE_KIND: AtomicU32 = AtomicU32::new(0);
static PROBE_LAST_RELEASE_GEN: AtomicU64 = AtomicU64::new(0);

#[inline]
fn cpu() -> usize {
    crate::arch::x86_64::usermode::cpu_index().min(MAX_CPUS - 1)
}

#[inline]
fn token(cpu: usize) -> usize {
    cpu + 1
}

#[inline]
fn probe_note_reenter() {
    PROBE_REENTER_SEQ.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn probe_note_acquire(cpu: usize, kind: u32) {
    // GEN=0 signifie uniquement "transition de metadata". Le token OWNER
    // est deja installe par le CAS ; le PIT sait donc ignorer ce tres court
    // intervalle plutot que de fabriquer un faux snapshot coherent.
    PROBE_OWNER_GEN.store(0, Ordering::Release);
    let generation = PROBE_ACQUIRE_SEQ.fetch_add(1, Ordering::AcqRel) + 1;
    let (task, syscall_nr, syscall_phase, site, aux) =
        crate::kernel::task::stall_probe_local_context();
    PROBE_OWNER_SINCE.store(crate::kernel::timer::ticks(), Ordering::Release);
    PROBE_OWNER_KIND.store(kind, Ordering::Release);
    PROBE_OWNER_TASK.store(task, Ordering::Release);
    PROBE_OWNER_SYSCALL.store(syscall_nr, Ordering::Release);
    PROBE_OWNER_SYSCALL_PHASE.store(syscall_phase, Ordering::Release);
    PROBE_OWNER_SITE.store(site, Ordering::Release);
    PROBE_OWNER_AUX.store(aux, Ordering::Release);
    PROBE_OWNER_GEN.store(generation, Ordering::Release);
    let _ = cpu;
}

#[inline]
fn probe_note_release(cpu: usize, kind: u32) {
    let generation = PROBE_OWNER_GEN.load(Ordering::Acquire);
    PROBE_RELEASE_SEQ.fetch_add(1, Ordering::AcqRel);
    PROBE_LAST_RELEASE_TICK.store(crate::kernel::timer::ticks(), Ordering::Release);
    PROBE_LAST_RELEASE_CPU.store(cpu, Ordering::Release);
    PROBE_LAST_RELEASE_KIND.store(kind, Ordering::Release);
    PROBE_LAST_RELEASE_GEN.store(generation, Ordering::Release);
}

// BOUCHAUD_BKL_ADAPTIVE_IDLE
//
// Sous Ladybird plusieurs processus entrent en noyau en parallele. Le BKL
// serialise encore ces passages : un spin pur faisait alors consommer un vCPU
// entier a chaque contender. On garde un court spin actif pour les sections
// critiques tres breves, puis on parque le CPU avec HLT si IF est actif.
//
// Le reveil est borne par l'architecture SMP actuelle : PIT sur le BSP et IPI
// de quantum sur les AP (4 ms actuellement).
const BKL_ACTIVE_SPINS: usize = 64;

#[inline]
fn wait_for_owner_change(active_spins: &mut usize) {
    if *active_spins < BKL_ACTIVE_SPINS {
        *active_spins += 1;
        spin_loop();
        return;
    }

    *active_spins = 0;

    // Ne jamais faire STI depuis un contexte qui avait IF=0 (ex. IRQ).
    // Dans ce cas rare on conserve le spin actif.
    if interrupts::are_enabled() {
        crate::arch::x86_64::cpu::wait_for_interrupt();
    } else {
        spin_loop();
    }
}

/// Serialise les transitions OWNER/DEPTH contre les IRQ du CPU courant.
/// L'etat IF precedent est restaure exactement au Drop.
struct LocalIrqGuard {
    restore_enabled: bool,
}

impl LocalIrqGuard {
    #[inline]
    fn acquire() -> Self {
        let restore_enabled = interrupts::are_enabled();
        interrupts::disable();
        Self { restore_enabled }
    }
}

impl Drop for LocalIrqGuard {
    #[inline]
    fn drop(&mut self) {
        if self.restore_enabled {
            interrupts::enable();
        }
    }
}

pub struct KernelGuard {
    cpu: usize,
    active: bool,
}

impl Drop for KernelGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        release_one(self.cpu);
    }
}

fn release_one(cpu: usize) {
    // OWNER + DEPTH doivent changer atomiquement vis-a-vis d'une IRQ locale.
    let _irq = LocalIrqGuard::acquire();

    let depth = DEPTH[cpu].load(Ordering::Relaxed);
    debug_assert!(depth > 0, "smp_lock: release sans acquisition");
    debug_assert_eq!(
        OWNER.load(Ordering::Acquire),
        token(cpu),
        "smp_lock: release par un CPU non proprietaire"
    );

    if depth > 1 {
        DEPTH[cpu].store(depth - 1, Ordering::Relaxed);
        return;
    }

    DEPTH[cpu].store(0, Ordering::Relaxed);
    probe_note_release(cpu, 1);
    OWNER.store(FREE, Ordering::Release);
}

pub fn enter() -> KernelGuard {
    let cpu = cpu();
    let mine = token(cpu);
    let mut active_spins = 0usize;

    loop {
        // Ne masquer les IRQ que pour le snapshot + la transition locale.
        {
            let _irq = LocalIrqGuard::acquire();
            let owner = OWNER.load(Ordering::Acquire);

            if owner == mine {
                DEPTH[cpu].fetch_add(1, Ordering::Relaxed);
                probe_note_reenter();
                return KernelGuard { cpu, active: true };
            }

            if owner == FREE
                && OWNER
                    .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                // Aucun handler local ne peut voir OWNER=mine avec DEPTH=0.
                DEPTH[cpu].store(1, Ordering::Relaxed);
                probe_note_acquire(cpu, 1);
                return KernelGuard { cpu, active: true };
            }
        }

        // Spin court puis HLT : ne plus bruler un coeur entier sur contention.
        wait_for_owner_change(&mut active_spins);
    }
}

/// Variante non bloquante, utile aux IPI de preemption : un IPI ne doit pas
/// immobiliser un coeur utilisateur entier si un autre CPU est deja dans le noyau.
pub fn try_enter() -> Option<KernelGuard> {
    let cpu = cpu();
    let mine = token(cpu);
    let _irq = LocalIrqGuard::acquire();

    let owner = OWNER.load(Ordering::Acquire);
    if owner == mine {
        DEPTH[cpu].fetch_add(1, Ordering::Relaxed);
        probe_note_reenter();
        return Some(KernelGuard { cpu, active: true });
    }

    if owner != FREE {
        return None;
    }

    if OWNER
        .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        DEPTH[cpu].store(1, Ordering::Relaxed);
        probe_note_acquire(cpu, 2);
        Some(KernelGuard { cpu, active: true })
    } else {
        None
    }
}

pub fn held_by_current_cpu() -> bool {
    let cpu = cpu();
    let _irq = LocalIrqGuard::acquire();
    OWNER.load(Ordering::Acquire) == token(cpu)
        && DEPTH[cpu].load(Ordering::Relaxed) > 0
}

/// Libere completement le BKL avant un switch de contexte et rend la profondeur
/// a restaurer lorsque cette pile noyau reprendra.
pub fn suspend_for_schedule() -> usize {
    let cpu = cpu();
    let mine = token(cpu);

    // C'etait la race observee : auparavant DEPTH passait a 0 avant OWNER,
    // avec IRQ encore actives. Le PIT pouvait alors entrer reentrant, puis
    // liberer OWNER avant notre assertion.
    let _irq = LocalIrqGuard::acquire();

    let depth = DEPTH[cpu].load(Ordering::Relaxed);
    if depth == 0 {
        return 0;
    }

    debug_assert_eq!(
        OWNER.load(Ordering::Acquire),
        mine,
        "smp_lock: suspend sans ownership"
    );

    DEPTH[cpu].store(0, Ordering::Relaxed);
    probe_note_release(cpu, 2);
    OWNER.store(FREE, Ordering::Release);
    depth
}

/// Reprend le BKL avec exactement la profondeur qu'avait la tache avant son
/// changement de contexte.
pub fn resume_after_schedule(depth: usize) {
    if depth == 0 {
        return;
    }

    let cpu = cpu();
    let mine = token(cpu);
    let mut active_spins = 0usize;

    loop {
        {
            let _irq = LocalIrqGuard::acquire();

            if OWNER
                .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Aucun handler local ne peut observer OWNER=mine avant que
                // la profondeur de la pile reprise soit restauree.
                DEPTH[cpu].store(depth, Ordering::Relaxed);
                probe_note_acquire(cpu, 3);
                return;
            }
        }

        // Meme politique adaptative lors de la reprise d'une pile noyau.
        wait_for_owner_change(&mut active_spins);
    }
}

pub fn owner_cpu() -> Option<usize> {
    match OWNER.load(Ordering::Acquire) {
        FREE => None,
        value => Some(value - 1),
    }
}

// BOUCHAUD_SMP4_STALL_PROBE_V1 -- lectures atomiques uniquement, utilisables
// depuis le PIT avant toute tentative de BKL.

#[derive(Clone, Copy)]
pub struct StallBklProvenance {
    pub owner_token: usize,
    pub generation: u64,
    pub coherent: bool,
    pub since_tick: u64,
    pub acquire_seq: u64,
    pub release_seq: u64,
    pub reenter_seq: u64,
    pub acquire_kind: u32,
    pub task: usize,
    pub syscall_nr: u64,
    pub syscall_phase: u32,
    pub site: u32,
    pub aux: u64,
    pub last_release_tick: u64,
    pub last_release_cpu: usize,
    pub last_release_kind: u32,
    pub last_release_gen: u64,
}

pub fn stall_probe_provenance() -> StallBklProvenance {
    for _ in 0..4 {
        let g1 = PROBE_OWNER_GEN.load(Ordering::Acquire);
        let owner = OWNER.load(Ordering::Acquire);
        let snapshot = StallBklProvenance {
            owner_token: owner,
            generation: g1,
            coherent: false,
            since_tick: PROBE_OWNER_SINCE.load(Ordering::Acquire),
            acquire_seq: PROBE_ACQUIRE_SEQ.load(Ordering::Acquire),
            release_seq: PROBE_RELEASE_SEQ.load(Ordering::Acquire),
            reenter_seq: PROBE_REENTER_SEQ.load(Ordering::Acquire),
            acquire_kind: PROBE_OWNER_KIND.load(Ordering::Acquire),
            task: PROBE_OWNER_TASK.load(Ordering::Acquire),
            syscall_nr: PROBE_OWNER_SYSCALL.load(Ordering::Acquire),
            syscall_phase: PROBE_OWNER_SYSCALL_PHASE.load(Ordering::Acquire),
            site: PROBE_OWNER_SITE.load(Ordering::Acquire),
            aux: PROBE_OWNER_AUX.load(Ordering::Acquire),
            last_release_tick: PROBE_LAST_RELEASE_TICK.load(Ordering::Acquire),
            last_release_cpu: PROBE_LAST_RELEASE_CPU.load(Ordering::Acquire),
            last_release_kind: PROBE_LAST_RELEASE_KIND.load(Ordering::Acquire),
            last_release_gen: PROBE_LAST_RELEASE_GEN.load(Ordering::Acquire),
        };
        let g2 = PROBE_OWNER_GEN.load(Ordering::Acquire);
        let owner2 = OWNER.load(Ordering::Acquire);
        if g1 == g2 && owner == owner2 && (owner == FREE || g1 != 0) {
            return StallBklProvenance { coherent: true, ..snapshot };
        }
    }
    StallBklProvenance {
        owner_token: OWNER.load(Ordering::Acquire),
        generation: PROBE_OWNER_GEN.load(Ordering::Acquire),
        coherent: false,
        since_tick: PROBE_OWNER_SINCE.load(Ordering::Acquire),
        acquire_seq: PROBE_ACQUIRE_SEQ.load(Ordering::Acquire),
        release_seq: PROBE_RELEASE_SEQ.load(Ordering::Acquire),
        reenter_seq: PROBE_REENTER_SEQ.load(Ordering::Acquire),
        acquire_kind: PROBE_OWNER_KIND.load(Ordering::Acquire),
        task: PROBE_OWNER_TASK.load(Ordering::Acquire),
        syscall_nr: PROBE_OWNER_SYSCALL.load(Ordering::Acquire),
        syscall_phase: PROBE_OWNER_SYSCALL_PHASE.load(Ordering::Acquire),
        site: PROBE_OWNER_SITE.load(Ordering::Acquire),
        aux: PROBE_OWNER_AUX.load(Ordering::Acquire),
        last_release_tick: PROBE_LAST_RELEASE_TICK.load(Ordering::Acquire),
        last_release_cpu: PROBE_LAST_RELEASE_CPU.load(Ordering::Acquire),
        last_release_kind: PROBE_LAST_RELEASE_KIND.load(Ordering::Acquire),
        last_release_gen: PROBE_LAST_RELEASE_GEN.load(Ordering::Acquire),
    }
}
pub fn stall_probe_owner_token() -> usize {
    OWNER.load(Ordering::Acquire)
}

pub fn stall_probe_depth(cpu: usize) -> usize {
    DEPTH[cpu.min(MAX_CPUS - 1)].load(Ordering::Acquire)
}
