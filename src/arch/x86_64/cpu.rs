//! Decouverte CPU via CPUID, halt/rdtsc et accounting SMP.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use crate::arch::x86_64::smp;

static IDLE: [AtomicBool; smp::MAX_CPUS] = [const { AtomicBool::new(false) }; smp::MAX_CPUS];
static IDLE_SINCE_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static IDLE_ACCUM_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static LOAD_LAST_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static LOAD_LAST_IDLE_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static CPU_TOTAL_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_USER_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_KERNEL_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_IDLE_TICKS: AtomicU64 = AtomicU64::new(0);

const LOAD_WINDOW_TICKS: u64 = 100;
static LOAD_WINDOW_TOTAL: AtomicU64 = AtomicU64::new(0);
static LOAD_WINDOW_IDLE: AtomicU64 = AtomicU64::new(0);
static CPU_LOAD_PCT: AtomicU8 = AtomicU8::new(0);

// BOUCHAUD_SMP_NG2_PERCPU_LOAD_V1
// Fenetre de charge par CPU logique. Le PIT BSP echantillonne IDLE[] pour tous
// les CPU online; aucun IPI supplementaire n'est necessaire.
static CORE_LOAD_PCT: [AtomicU8; smp::MAX_CPUS] =
    [const { AtomicU8::new(0) }; smp::MAX_CPUS];

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuAccounting {
    pub total_ticks: u64,
    pub user_ticks: u64,
    pub kernel_ticks: u64,
    pub idle_ticks: u64,
}

impl CpuAccounting {
    pub fn idle_percent(self) -> u64 {
        if self.total_ticks == 0 { 0 } else { self.idle_ticks.saturating_mul(100) / self.total_ticks }
    }
    pub fn busy_percent(self) -> u64 { 100u64.saturating_sub(self.idle_percent()) }
}

/// Logical Bouchaud CPU index used by per-CPU accounting arrays.
///
/// The historical name is kept for source compatibility during NG1.
pub fn hardware_cpu_index() -> usize {
    smp::cpu_index().min(smp::MAX_CPUS - 1)
}

fn update_core_load_windows(online: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    for cpu in 0..online.min(smp::MAX_CPUS) {
        let last = LOAD_LAST_NS[cpu].swap(now, Ordering::AcqRel);
        let idle_total = idle_ns_at(cpu, now);
        let last_idle = LOAD_LAST_IDLE_NS[cpu].swap(idle_total, Ordering::AcqRel);
        if last == 0 {
            continue;
        }
        let elapsed = now.saturating_sub(last).max(1);
        let idle = idle_total.saturating_sub(last_idle).min(elapsed);
        let sample = 100u64.saturating_sub(idle.saturating_mul(100) / elapsed) as u8;
        let old = CORE_LOAD_PCT[cpu].load(Ordering::Relaxed);
        let filtered = if old == 0 {
            sample
        } else {
            ((old as u16 * 3 + sample as u16) / 4) as u8
        };
        CORE_LOAD_PCT[cpu].store(filtered.min(100), Ordering::Relaxed);
    }
}

fn update_load_window(idle_count: u64, online: u64) {
    LOAD_WINDOW_IDLE.fetch_add(idle_count, Ordering::Relaxed);
    let total = LOAD_WINDOW_TOTAL.fetch_add(online, Ordering::Relaxed) + online;
    let target = LOAD_WINDOW_TICKS.saturating_mul(online.max(1));
    if total < target { return; }

    let idle_ticks = LOAD_WINDOW_IDLE.swap(0, Ordering::Relaxed);
    let window = LOAD_WINDOW_TOTAL.swap(0, Ordering::Relaxed).max(1);
    let sample = 100u64.saturating_sub(idle_ticks.saturating_mul(100) / window) as u8;
    let old = CPU_LOAD_PCT.load(Ordering::Relaxed);
    let filtered = if old == 0 { sample } else { ((old as u16 * 3 + sample as u16) / 4) as u8 };
    CPU_LOAD_PCT.store(filtered.min(100), Ordering::Relaxed);
}

/// Appele par le PIT sur le BSP. Un tick represente une milliseconde de
/// capacite pour chacun des CPU online : la charge affichee devient donc la
/// moyenne reelle de la machine, et non plus la saturation du seul BSP.
pub fn account_timer_tick(interrupted_user: bool) -> bool {
    let bsp = hardware_cpu_index();
    let online = smp::schedulable_cpus().max(1).min(smp::MAX_CPUS);
    let mut idle_count = 0u64;
    for cpu in 0..online {
        if IDLE[cpu].load(Ordering::Acquire) { idle_count += 1; }
    }

    update_core_load_windows(online);

    let bsp_idle = IDLE[bsp].load(Ordering::Acquire);
    CPU_TOTAL_TICKS.fetch_add(online as u64, Ordering::Relaxed);
    CPU_IDLE_TICKS.fetch_add(idle_count, Ordering::Relaxed);

    // Les AP executent presque exclusivement du ring3 entre deux IPI. Pour le
    // compteur detaille on range donc leur temps busy dans user. La charge
    // globale, elle, est exacte car calculee uniquement depuis busy/idle.
    let other_busy = (online as u64)
        .saturating_sub(idle_count)
        .saturating_sub(if bsp_idle { 0 } else { 1 });
    CPU_USER_TICKS.fetch_add(other_busy, Ordering::Relaxed);
    if !bsp_idle {
        if interrupted_user {
            CPU_USER_TICKS.fetch_add(1, Ordering::Relaxed);
        } else {
            CPU_KERNEL_TICKS.fetch_add(1, Ordering::Relaxed);
        }
    }

    update_load_window(idle_count, online as u64);
    bsp_idle
}

pub fn accounting() -> CpuAccounting {
    CpuAccounting {
        total_ticks: CPU_TOTAL_TICKS.load(Ordering::Acquire),
        user_ticks: CPU_USER_TICKS.load(Ordering::Relaxed),
        kernel_ticks: CPU_KERNEL_TICKS.load(Ordering::Relaxed),
        idle_ticks: CPU_IDLE_TICKS.load(Ordering::Relaxed),
    }
}

pub fn load_percent() -> u8 {
    let online = smp::schedulable_cpus().max(1).min(smp::MAX_CPUS);
    let sum: u64 = (0..online)
        .map(|cpu| CORE_LOAD_PCT[cpu].load(Ordering::Relaxed) as u64)
        .sum();
    (sum / online as u64).min(100) as u8
}

pub fn load_percent_cpu(cpu: usize) -> u8 {
    if cpu >= smp::MAX_CPUS {
        0
    } else {
        CORE_LOAD_PCT[cpu].load(Ordering::Relaxed)
    }
}

pub fn load_snapshot() -> [u8; smp::MAX_CPUS] {
    let mut out = [0u8; smp::MAX_CPUS];
    let mut cpu = 0usize;
    while cpu < smp::MAX_CPUS {
        out[cpu] = CORE_LOAD_PCT[cpu].load(Ordering::Relaxed);
        cpu += 1;
    }
    out
}

pub fn is_idle(cpu: usize) -> bool {
    cpu < smp::MAX_CPUS && IDLE[cpu].load(Ordering::Acquire)
}

pub fn idle_mask() -> u64 {
    let mut mask = 0u64;
    for cpu in 0..smp::schedulable_cpus().min(64) {
        if is_idle(cpu) { mask |= 1u64 << cpu; }
    }
    mask
}

pub fn halt_loop() -> ! {
    loop { unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)); } }
}

pub fn hlt() {
    let cpu = hardware_cpu_index();
    idle_enter(cpu);
    unsafe { asm!("hlt", options(nostack, preserves_flags)); }
    idle_exit(cpu);
}

pub fn wait_for_interrupt() {
    let cpu = hardware_cpu_index();
    idle_enter(cpu);
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
}

// BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
//
// Scheduler sleep is a two-phase handshake.
//
// PREPARE runs while the caller still owns the BKL:
//   1. IF is cleared;
//   2. IDLE[cpu] becomes visible.
//
// Only then may the caller release the BKL. Every normal Ready publication is
// serialized by that same BKL, so a producer can no longer enqueue work in the
// old "not idle yet / already released the BKL" window.
//
// COMMIT uses the architectural STI;HLT interrupt shadow. If a targeted wakeup
// became pending after PREPARE, HLT executes atomically with respect to that
// pending interrupt and returns immediately instead of losing the wakeup.
pub fn prepare_scheduler_idle() {
    debug_assert!(
        interrupts_enabled(),
        "cpu: prepare_scheduler_idle requires IF=1"
    );
    let cpu = hardware_cpu_index();
    unsafe { asm!("cli", options(nomem, nostack)); }
    idle_enter(cpu);
}

pub fn commit_scheduler_idle() {
    debug_assert!(
        !interrupts_enabled(),
        "cpu: commit_scheduler_idle requires IF=0"
    );
    let cpu = hardware_cpu_index();
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
}

// BOUCHAUD_P1_BKL_PARK_WAKE_V1
//
// Meme handshake en deux temps que le sommeil du scheduler, pour un CPU qui
// s'arrete faute d'obtenir un verrou plutot que faute de travail.
//
// La difference tient au reveilleur. Le scheduler endort un CPU qui n'a rien a
// faire : c'est la publication d'une tache Ready qui le rappelle. Ici le CPU a
// du travail et lui manque seulement le verrou : c'est donc la LIBERATION de ce
// verrou qui doit le rappeler, et elle seule.
//
// Ces trois fonctions existent pour que `kernel::sync` n'ait pas a connaitre
// `sti`, `hlt` ni l'APIC. Le protocole -- publier l'attente avant de relire le
// proprietaire, et ne dormir qu'ensuite -- vit cote noyau, ou il peut etre
// relu sans savoir sur quelle machine il tourne.

/// PREPARE : masque les interruptions et publie l'arret de ce CPU.
///
/// A appeler AVANT de publier l'attente et de relire le proprietaire du verrou.
/// Une fois IF a zero, un reveil qui arrive reste en attente dans l'APIC local
/// au lieu d'etre consomme avant le `hlt`.
pub fn prepare_lock_park() {
    debug_assert!(interrupts_enabled(), "cpu: prepare_lock_park requires IF=1");
    let cpu = hardware_cpu_index();
    unsafe { asm!("cli", options(nomem, nostack)); }
    idle_enter(cpu);
}

/// COMMIT : dort jusqu'au prochain reveil, puis reprend.
///
/// `sti; hlt` est atomique vis-a-vis d'une interruption devenue pendante depuis
/// le PREPARE : l'ombre du `sti` garantit que le `hlt` s'execute et rend la
/// main aussitot, au lieu de perdre le reveil.
pub fn commit_lock_park() {
    debug_assert!(!interrupts_enabled(), "cpu: commit_lock_park requires IF=0");
    let cpu = hardware_cpu_index();
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
}

/// ANNULE le PREPARE : rend les interruptions sans jamais dormir.
///
/// Le verrou s'est libere entre le spin et la publication de l'attente. Dormir
/// maintenant serait attendre un reveil que plus personne n'enverra, puisque le
/// liberateur est deja passe.
pub fn abort_lock_park() {
    debug_assert!(!interrupts_enabled(), "cpu: abort_lock_park requires IF=0");
    let cpu = hardware_cpu_index();
    idle_exit(cpu);
    unsafe { asm!("sti", options(nomem, nostack)); }
}

/// Rappelle un CPU arrete par [`commit_lock_park`].
///
/// Le vecteur est celui de la preemption : son gestionnaire prend le verrou par
/// `try_enter`, echoue proprement s'il est pris, et ne commute jamais depuis un
/// contexte noyau interrompu. Le recevoir sans raison ne coute donc qu'un
/// aller-retour d'interruption -- c'est ce qui permet de l'envoyer sans savoir
/// si la cible dort vraiment.
pub fn wake_parked_cpu(cpu: usize) {
    smp::reschedule_cpu(cpu);
}

fn idle_enter(cpu: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    IDLE_SINCE_NS[cpu].store(now, Ordering::Release);
    IDLE[cpu].store(true, Ordering::Release);
}

fn idle_exit(cpu: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    if IDLE[cpu].swap(false, Ordering::AcqRel) {
        let since = IDLE_SINCE_NS[cpu].swap(0, Ordering::AcqRel);
        IDLE_ACCUM_NS[cpu].fetch_add(now.saturating_sub(since), Ordering::Relaxed);
    }
}

fn idle_ns_at(cpu: usize, now: u64) -> u64 {
    let accumulated = IDLE_ACCUM_NS[cpu].load(Ordering::Acquire);
    if IDLE[cpu].load(Ordering::Acquire) {
        accumulated.saturating_add(
            now.saturating_sub(IDLE_SINCE_NS[cpu].load(Ordering::Acquire)),
        )
    } else {
        accumulated
    }
}

pub fn idle_ns(cpu: usize) -> u64 {
    if cpu >= smp::MAX_CPUS {
        0
    } else {
        idle_ns_at(cpu, crate::kernel::timer::monotonic_ns())
    }
}

pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe { asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags)); }
    flags & (1 << 9) != 0
}

pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe { asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)); }
    ((hi as u64) << 32) | lo as u64
}

/// Lecture TSC ordonnee pour le timekeeping. RDTSCP attend les instructions
/// anterieures; LFENCE+RDTSC fournit le meme ordre minimal sur le fallback.
pub fn read_tsc_ordered(rdtscp: bool) -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        if rdtscp {
            asm!(
                "rdtscp",
                out("eax") lo,
                out("edx") hi,
                out("ecx") _,
                options(nomem, nostack)
            );
        } else {
            asm!(
                "lfence",
                "rdtsc",
                out("eax") lo,
                out("edx") hi,
                options(nomem, nostack)
            );
        }
    }
    ((hi as u64) << 32) | lo as u64
}

#[cfg(target_arch = "x86_64")]
pub fn vendor() -> [u8; 12] {
    use core::arch::x86_64::__cpuid;
    let res = __cpuid(0);
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&res.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&res.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&res.ecx.to_le_bytes());
    vendor
}

fn bit(value: u32, index: u32) -> &'static str {
    if value & (1u32 << index) != 0 { "yes" } else { "no" }
}

#[cfg(target_arch = "x86_64")]
pub fn print_cpuinfo() {
    use core::arch::x86_64::__cpuid;
    let vendor = vendor();
    crate::print!("vendor_id: ");
    for b in vendor { crate::print!("{}", b as char); }
    println!("");
    let leaf1 = __cpuid(1);
    let family = (leaf1.eax >> 8) & 0xf;
    let model = (leaf1.eax >> 4) & 0xf;
    let stepping = leaf1.eax & 0xf;
    println!("family: {}", family);
    println!("model: {}", model);
    println!("stepping: {}", stepping);
    println!("features:");
    println!("  sse3={} pclmulqdq={} vmx={} ssse3={}", bit(leaf1.ecx, 0), bit(leaf1.ecx, 1), bit(leaf1.ecx, 5), bit(leaf1.ecx, 9));
    println!("  sse={} sse2={} htt={}", bit(leaf1.edx, 25), bit(leaf1.edx, 26), bit(leaf1.edx, 28));
    println!("  smp_online={} logical_cpu={} apic_id={}", smp::schedulable_cpus(), hardware_cpu_index(), smp::hardware_apic_id());
}
