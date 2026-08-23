//! Decouverte CPU via CPUID, halt/rdtsc et accounting SMP.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use crate::arch::x86_64::smp;

static IDLE: [AtomicBool; smp::MAX_CPUS] = [const { AtomicBool::new(false) }; smp::MAX_CPUS];
static CPU_TOTAL_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_USER_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_KERNEL_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_IDLE_TICKS: AtomicU64 = AtomicU64::new(0);

const LOAD_WINDOW_TICKS: u64 = 100;
static LOAD_WINDOW_TOTAL: AtomicU64 = AtomicU64::new(0);
static LOAD_WINDOW_IDLE: AtomicU64 = AtomicU64::new(0);
static CPU_LOAD_PCT: AtomicU8 = AtomicU8::new(0);

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

pub fn hardware_cpu_index() -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::__cpuid;
        (((__cpuid(1).ebx >> 24) & 0xff) as usize).min(smp::MAX_CPUS - 1)
    }
    #[cfg(not(target_arch = "x86_64"))]
    { 0 }
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

pub fn load_percent() -> u8 { CPU_LOAD_PCT.load(Ordering::Relaxed) }

pub fn halt_loop() -> ! {
    loop { unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)); } }
}

pub fn hlt() {
    let cpu = hardware_cpu_index();
    IDLE[cpu].store(true, Ordering::SeqCst);
    unsafe { asm!("hlt", options(nostack, preserves_flags)); }
    IDLE[cpu].store(false, Ordering::SeqCst);
}

pub fn wait_for_interrupt() {
    let cpu = hardware_cpu_index();
    IDLE[cpu].store(true, Ordering::SeqCst);
    unsafe { asm!("sti; hlt", options(nostack)); }
    IDLE[cpu].store(false, Ordering::SeqCst);
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
    println!("  smp_online={} apic_id={}", smp::schedulable_cpus(), hardware_cpu_index());
}
