//! SMP-NG CPU foundation.
//!
//! This module is the runtime source of truth for Bouchaud logical CPUs.
//! Hardware APIC IDs are identifiers, not array indexes.
//!
//! NG1 deliberately keeps the existing SIPI trampoline unchanged: before the AP
//! reaches Rust it still uses the legacy 8-bit APIC ID to select one of the
//! bootstrap stacks. Once Rust starts, every CPU is registered under a dense
//! logical [`CpuId`] and the rest of the kernel can stop assuming
//! `logical_cpu == APIC_ID`.

use alloc::vec::Vec;
use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::sync::atomic::{
    AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

use crate::arch::x86_64::smp::MAX_CPUS;
use crate::kernel::sync::SpinLockIrq;

/// No task is currently attached to a CPU-local slot.
pub const NO_TASK: usize = usize::MAX;

/// Dense Bouchaud CPU identifier.
///
/// Unlike an APIC ID, this is always in `0..MAX_CPUS` and is therefore safe to
/// use as an index into per-CPU tables.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CpuId(u16);

impl CpuId {
    pub const BSP: Self = Self(0);

    pub const fn from_index(index: usize) -> Option<Self> {
        if index < MAX_CPUS && index <= u16::MAX as usize {
            Some(Self(index as u16))
        } else {
            None
        }
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Compact set of logical CPUs.
///
/// `MAX_CPUS` is currently 16, so a u64 leaves room for growth without changing
/// the ABI of the scheduler data structures that will use this type in NG4/NG5.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuMask(u64);

impl CpuMask {
    pub const EMPTY: Self = Self(0);

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub fn all_registered() -> Self {
        let count = registered_cpus().min(64);
        if count == 0 {
            return Self::EMPTY;
        }
        if count >= 64 {
            return Self(u64::MAX);
        }
        Self((1u64 << count) - 1)
    }

    pub fn all_online() -> Self {
        let mut bits = 0u64;
        let count = registered_cpus().min(64);
        for index in 0..count {
            if CPUS[index].online.load(Ordering::Acquire) {
                bits |= 1u64 << index;
            }
        }
        Self(bits)
    }

    pub const fn contains(self, cpu: CpuId) -> bool {
        let bit = cpu.as_usize();
        bit < 64 && (self.0 & (1u64 << bit)) != 0
    }

    pub fn insert(&mut self, cpu: CpuId) {
        let bit = cpu.as_usize();
        if bit < 64 {
            self.0 |= 1u64 << bit;
        }
    }

    pub fn remove(&mut self, cpu: CpuId) {
        let bit = cpu.as_usize();
        if bit < 64 {
            self.0 &= !(1u64 << bit);
        }
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CpuDescriptor {
    pub logical_id: CpuId,
    /// x2APIC ID when CPUID topology leaves expose it; legacy APIC ID otherwise.
    pub apic_id: u32,
    /// CPUID.1 EBX[31:24], kept only for the current real-mode trampoline.
    pub legacy_apic_id: u8,
    pub package_id: u32,
    pub core_id: u32,
    pub thread_id: u32,
    pub online: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuStats {
    pub context_switches: u64,
    pub migrations: u64,
    pub ipi_rx: u64,
    pub ipi_tx: u64,
    pub tlb_shootdowns: u64,
}

/// Runtime state owned by one logical processor.
///
/// Fields used on hot paths are atomic from day one. NG4 will move the scheduler
/// current/runqueue ownership here; NG8 will consume the TLB counters.
pub struct CpuLocal {
    registered: AtomicBool,
    online: AtomicBool,
    apic_id: AtomicU32,
    legacy_apic_id: AtomicU32,
    package_id: AtomicU32,
    core_id: AtomicU32,
    thread_id: AtomicU32,

    current_task: AtomicUsize,
    need_resched: AtomicBool,
    irq_depth: AtomicU32,
    preempt_count: AtomicU32,
// BOUCHAUD_RUNQ_IRQ_V1
    //
    // POURQUOI `SpinLockIrq` ET NON `SpinLock`
    //
    // Cette file est atteignable depuis un GESTIONNAIRE D'INTERRUPTION. Le
    // chemin exact, observe au runtime :
    //
    //     IRQ clavier/souris (8042)
    //       -> push_scancode / mouse::handle_byte
    //       -> kernel::sync::signale_interface(...)      [reveil du compositeur]
    //       -> WaitQueue::wake_all
    //       -> task::wake_wait_queue
    //       -> task::publish_ready
    //       -> CpuLocal::enqueue        <-- reprend cette meme file
    //
    // Un `SpinLock` ordinaire ne masque pas les interruptions. Si l'IRQ tombe
    // pendant qu'une tache du MEME CPU est a l'interieur d'un accesseur --
    // `enqueue`, `dequeue`, `steal`, `run_queue_len` --, le gestionnaire
    // reprend le verrou que le contexte interrompu detient deja. Le
    // `debug_assert!` de recursion de `SpinLock` le voit et panique :
    //
    //     SpinLock recursive acquisition on CPU 0
    //
    // La fenetre n'est pas theorique : `queue.contains()` est lineaire,
    // `queue.remove(0)` deplace tout le vecteur, et `queue.push()` peut
    // reallouer -- donc allouer -- sous le verrou.
    //
    // Le gros verrou ne protege PAS de ce cas : il appartient a un CPU, pas a
    // une tache, et `smp_lock::enter()` est REENTRANTE. Un gestionnaire
    // d'interruption qui le reprend sur un CPU qui le detient deja obtient donc
    // un garde valide et continue -- c'est le comportement voulu, et c'est
    // precisement ce qui amene le handler jusqu'ici.
    //
    // `SpinLockIrq` masque les interruptions pour la duree de la section
    // critique. L'IRQ ne disparait pas : elle reste en attente dans l'APIC
    // local et est delivree des le relachement. Aucun reveil n'est perdu.
    // BOUCHAUD_C1_FILE_GENERATIONNELLE_V1
    //
    // La file portait des INDICES d'emplacement. Un emplacement se recycle :
    // une entree laissee par une tache morte designait alors la tache
    // suivante, qui n'a jamais demande a etre ordonnancee. C'est le probleme
    // ABA, et il se manifeste comme une tache fantome qui prend des quantums.
    //
    // Elle porte desormais des IDENTITES empaquetees -- emplacement plus
    // generation. Le consommateur refuse celles dont la generation ne
    // correspond plus.
    run_queue: SpinLockIrq<Vec<u64>>,

    context_switches: AtomicU64,
    migrations: AtomicU64,
    ipi_rx: AtomicU64,
    ipi_tx: AtomicU64,
    tlb_shootdowns: AtomicU64,
}

impl CpuLocal {
    const fn new() -> Self {
        Self {
            registered: AtomicBool::new(false),
            online: AtomicBool::new(false),
            apic_id: AtomicU32::new(u32::MAX),
            legacy_apic_id: AtomicU32::new(u32::MAX),
            package_id: AtomicU32::new(0),
            core_id: AtomicU32::new(0),
            thread_id: AtomicU32::new(0),
            current_task: AtomicUsize::new(NO_TASK),
            need_resched: AtomicBool::new(false),
            irq_depth: AtomicU32::new(0),
            preempt_count: AtomicU32::new(0),
            run_queue: SpinLockIrq::new(Vec::new()),
            context_switches: AtomicU64::new(0),
            migrations: AtomicU64::new(0),
            ipi_rx: AtomicU64::new(0),
            ipi_tx: AtomicU64::new(0),
            tlb_shootdowns: AtomicU64::new(0),
        }
    }

    pub fn current_task(&self) -> usize {
        self.current_task.load(Ordering::Acquire)
    }

    pub fn set_current_task(&self, task: usize) {
        self.current_task.store(task, Ordering::Release);
    }

    pub fn need_resched(&self) -> bool {
        self.need_resched.load(Ordering::Acquire)
    }

    pub fn request_resched(&self) {
        self.need_resched.store(true, Ordering::Release);
    }

    pub fn clear_resched(&self) {
        self.need_resched.store(false, Ordering::Release);
    }

    pub fn irq_enter(&self) {
        self.irq_depth.fetch_add(1, Ordering::Relaxed);
    }

    pub fn irq_exit(&self) {
        let old = self.irq_depth.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(old != 0, "cpu_local: irq_depth underflow");
    }

    pub fn irq_depth(&self) -> u32 {
        self.irq_depth.load(Ordering::Relaxed)
    }

    pub fn preempt_disable(&self) {
        self.preempt_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn preempt_enable(&self) {
        let old = self.preempt_count.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(old != 0, "cpu_local: preempt_count underflow");
    }

    pub fn preempt_count(&self) -> u32 {
        self.preempt_count.load(Ordering::Relaxed)
    }

    /// Met une IDENTITE en file. `identite` est un `TacheId` empaquete.
    pub fn enqueue(&self, identite: u64) {
        let mut queue = self.run_queue.lock();
        if !queue.contains(&identite) {
            queue.push(identite);
        }
    }

    pub fn dequeue(&self) -> Option<u64> {
        let mut queue = self.run_queue.lock();
        if queue.is_empty() { None } else { Some(queue.remove(0)) }
    }

    pub fn steal(&self) -> Option<u64> {
        self.run_queue.lock().pop()
    }

    pub fn remove(&self, task: u64) -> bool {
        let mut queue = self.run_queue.lock();
        let Some(index) = queue.iter().position(|candidate| *candidate == task) else {
            return false;
        };
        queue.remove(index);
        true
    }

    /// La file contient-elle cette identite ? `None` si elle est verrouillee.
    ///
    /// `try_lock` et non `lock` : cette sonde s'execute depuis l'IRQ du timer,
    /// qui peut avoir interrompu le porteur de ce meme verrou sur ce meme CPU.
    /// Attendre y serait un interblocage certain ; ne pas conclure est la
    /// seule reponse honnete.
    pub fn file_contient(&self, identite: u64) -> Option<bool> {
        self.run_queue.try_lock().map(|file| file.contains(&identite))
    }

    /// La file a-t-elle du travail ? `None` si elle est verrouillee.
    ///
    /// L'appelant doit traiter `None` comme « peut-etre du travail » : perdre
    /// un `hlt` ne coute qu'un tour de boucle, perdre un reveil fige la
    /// machine. L'incertitude se resout donc toujours du meme cote.
    pub fn file_non_vide_essai(&self) -> Option<bool> {
        self.run_queue.try_lock().map(|file| !file.is_empty())
    }

    pub fn run_queue_len(&self) -> usize {
        self.run_queue.lock().len()
    }

    pub fn note_context_switch(&self) {
        self.context_switches.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_migration(&self) {
        self.migrations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_ipi_rx(&self) {
        self.ipi_rx.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_ipi_tx(&self) {
        self.ipi_tx.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_tlb_shootdown(&self) {
        self.tlb_shootdowns.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stats(&self) -> CpuStats {
        CpuStats {
            context_switches: self.context_switches.load(Ordering::Relaxed),
            migrations: self.migrations.load(Ordering::Relaxed),
            ipi_rx: self.ipi_rx.load(Ordering::Relaxed),
            ipi_tx: self.ipi_tx.load(Ordering::Relaxed),
            tlb_shootdowns: self.tlb_shootdowns.load(Ordering::Relaxed),
        }
    }
}

static CPUS: [CpuLocal; MAX_CPUS] = [const { CpuLocal::new() }; MAX_CPUS];
static REGISTERED_CPUS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
struct Topology {
    apic_id: u32,
    legacy_apic_id: u8,
    package_id: u32,
    core_id: u32,
    thread_id: u32,
}

fn low_mask(bits: u32) -> u32 {
    if bits == 0 {
        0
    } else if bits >= 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    }
}

fn topology_from_leaf(leaf: u32) -> Option<Topology> {
    let first = __cpuid_count(leaf, 0);
    if first.ebx == 0 {
        return None;
    }

    let mut smt_shift = 0u32;
    let mut core_shift = 0u32;
    let mut x2apic_id = first.edx;

    for level in 0..8u32 {
        let r = __cpuid_count(leaf, level);
        if r.ebx == 0 {
            break;
        }

        x2apic_id = r.edx;
        let shift = r.eax & 0x1f;
        let level_type = (r.ecx >> 8) & 0xff;

        match level_type {
            1 => smt_shift = shift,
            2 => core_shift = shift,
            _ => {}
        }
    }

    if core_shift < smt_shift {
        core_shift = smt_shift;
    }

    let thread_id = x2apic_id & low_mask(smt_shift);
    let core_width = core_shift.saturating_sub(smt_shift);
    let core_id = (x2apic_id >> smt_shift) & low_mask(core_width);
    let package_id = if core_shift >= 32 {
        0
    } else {
        x2apic_id >> core_shift
    };

    Some(Topology {
        apic_id: x2apic_id,
        legacy_apic_id: legacy_hardware_apic_id(),
        package_id,
        core_id,
        thread_id,
    })
}

fn detect_topology() -> Topology {
    let max_leaf = __cpuid(0).eax;

    if max_leaf >= 0x1f {
        if let Some(topo) = topology_from_leaf(0x1f) {
            return topo;
        }
    }
    if max_leaf >= 0x0b {
        if let Some(topo) = topology_from_leaf(0x0b) {
            return topo;
        }
    }

    let legacy = legacy_hardware_apic_id();
    Topology {
        apic_id: legacy as u32,
        legacy_apic_id: legacy,
        package_id: 0,
        core_id: legacy as u32,
        thread_id: 0,
    }
}

/// Legacy 8-bit APIC ID used by the current real-mode trampoline.
pub fn legacy_hardware_apic_id() -> u8 {
    ((__cpuid(1).ebx >> 24) & 0xff) as u8
}

/// Best APIC identifier available to Rust (x2APIC when CPUID exposes it).
pub fn hardware_apic_id() -> u32 {
    detect_topology().apic_id
}

fn initialize(cpu: CpuId, topology: Topology) {
    let local = &CPUS[cpu.as_usize()];
    local.apic_id.store(topology.apic_id, Ordering::Relaxed);
    local
        .legacy_apic_id
        .store(topology.legacy_apic_id as u32, Ordering::Relaxed);
    local.package_id.store(topology.package_id, Ordering::Relaxed);
    local.core_id.store(topology.core_id, Ordering::Relaxed);
    local.thread_id.store(topology.thread_id, Ordering::Relaxed);
    local.registered.store(true, Ordering::Release);
}

/// Register the bootstrap processor as logical CPU 0.
pub fn register_bsp() -> CpuId {
    let id = CpuId::BSP;
    initialize(id, detect_topology());

    let mut seen = REGISTERED_CPUS.load(Ordering::Acquire);
    while seen < 1 {
        match REGISTERED_CPUS.compare_exchange_weak(
            seen,
            1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(actual) => seen = actual,
        }
    }
    id
}

/// Register the AP executing this function and allocate a dense logical CPU ID.
pub fn register_current_ap() -> Option<CpuId> {
    let topology = detect_topology();

    if let Some(existing) = logical_for_apic(topology.apic_id) {
        return Some(existing);
    }

    let slot = REGISTERED_CPUS.fetch_add(1, Ordering::AcqRel);
    let id = CpuId::from_index(slot)?;
    initialize(id, topology);
    Some(id)
}

pub fn registered_cpus() -> usize {
    REGISTERED_CPUS.load(Ordering::Acquire).min(MAX_CPUS)
}

pub fn local(cpu: CpuId) -> &'static CpuLocal {
    &CPUS[cpu.as_usize()]
}

pub fn descriptor(cpu: CpuId) -> Option<CpuDescriptor> {
    let local = local(cpu);
    if !local.registered.load(Ordering::Acquire) {
        return None;
    }

    Some(CpuDescriptor {
        logical_id: cpu,
        apic_id: local.apic_id.load(Ordering::Relaxed),
        legacy_apic_id: local.legacy_apic_id.load(Ordering::Relaxed) as u8,
        package_id: local.package_id.load(Ordering::Relaxed),
        core_id: local.core_id.load(Ordering::Relaxed),
        thread_id: local.thread_id.load(Ordering::Relaxed),
        online: local.online.load(Ordering::Acquire),
    })
}

pub fn logical_for_apic(apic_id: u32) -> Option<CpuId> {
    let count = registered_cpus();
    for index in 0..count {
        let local = &CPUS[index];
        if local.registered.load(Ordering::Acquire)
            && local.apic_id.load(Ordering::Relaxed) == apic_id
        {
            return CpuId::from_index(index);
        }
    }
    None
}

pub fn mark_online(cpu: CpuId, online: bool) {
    local(cpu).online.store(online, Ordering::Release);
}

pub fn is_online(cpu: CpuId) -> bool {
    local(cpu).online.load(Ordering::Acquire)
}
