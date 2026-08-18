//! Decouverte CPU via CPUID et primitives bas niveau (halt, rdtsc).

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

/// Accounting global du CPU unique actuel.
///
/// L'etat idle est lu depuis IRQ0 pendant que l'instruction `hlt` n'a pas
/// encore rendu la main au code interrompu. Il doit donc etre observable
/// exactement a travers cette frontiere d'interruption.
static IDLE: AtomicBool = AtomicBool::new(false);
static CPU_TOTAL_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_USER_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_KERNEL_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_IDLE_TICKS: AtomicU64 = AtomicU64::new(0);

/// Fenetre de charge systeme, alimentee directement par IRQ0.
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
        if self.total_ticks == 0 {
            0
        } else {
            self.idle_ticks.saturating_mul(100) / self.total_ticks
        }
    }

    pub fn busy_percent(self) -> u64 {
        100u64.saturating_sub(self.idle_percent())
    }
}

fn update_load_window(idle: bool) {
    if idle {
        LOAD_WINDOW_IDLE.fetch_add(1, Ordering::Relaxed);
    }
    let window = LOAD_WINDOW_TOTAL.fetch_add(1, Ordering::Relaxed) + 1;
    if window < LOAD_WINDOW_TICKS {
        return;
    }

    // IRQ0 est l'unique ecrivain sur le CPU UP actuel.
    let idle_ticks = LOAD_WINDOW_IDLE.swap(0, Ordering::Relaxed);
    LOAD_WINDOW_TOTAL.store(0, Ordering::Relaxed);

    let sample = 100u64.saturating_sub(
        idle_ticks.saturating_mul(100) / window.max(1)
    ) as u8;

    // EWMA courte : stable dans l'UI sans masquer les changements rapides.
    let old = CPU_LOAD_PCT.load(Ordering::Relaxed);
    let filtered = if old == 0 {
        sample
    } else {
        ((old as u16 * 3 + sample as u16) / 4) as u8
    };
    CPU_LOAD_PCT.store(filtered.min(100), Ordering::Relaxed);
}

/// Appele une fois par IRQ0. Rend true si le tick a reveille HLT.
pub fn account_timer_tick(interrupted_user: bool) -> bool {
    let idle = IDLE.load(Ordering::Acquire);

    CPU_TOTAL_TICKS.fetch_add(1, Ordering::Relaxed);
    if idle {
        CPU_IDLE_TICKS.fetch_add(1, Ordering::Relaxed);
    } else if interrupted_user {
        CPU_USER_TICKS.fetch_add(1, Ordering::Relaxed);
    } else {
        CPU_KERNEL_TICKS.fetch_add(1, Ordering::Relaxed);
    }

    update_load_window(idle);
    idle
}

/// Snapshot coherent sur le CPU unique : si IRQ0 passe entre les lectures,
/// on recommence afin de conserver user + kernel + idle == total.
pub fn accounting() -> CpuAccounting {
    loop {
        let before = CPU_TOTAL_TICKS.load(Ordering::Acquire);
        let user = CPU_USER_TICKS.load(Ordering::Relaxed);
        let kernel = CPU_KERNEL_TICKS.load(Ordering::Relaxed);
        let idle = CPU_IDLE_TICKS.load(Ordering::Relaxed);
        let after = CPU_TOTAL_TICKS.load(Ordering::Acquire);

        if before == after {
            return CpuAccounting {
                total_ticks: after,
                user_ticks: user,
                kernel_ticks: kernel,
                idle_ticks: idle,
            };
        }
        core::hint::spin_loop();
    }
}

/// Charge CPU systeme recente (0..=100), calculee depuis busy/idle IRQ0.
pub fn load_percent() -> u8 {
    CPU_LOAD_PCT.load(Ordering::Relaxed)
}


/// Boucle d'arret du CPU. Utilisee par le panic handler et l'arret du noyau.
pub fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)); }
    }
}

/// Pause le CPU jusqu'a la prochaine interruption (PIT/clavier/souris).
/// Reduit la charge CPU dans les boucles d'evenements.
///
/// N'appeler que si les interruptions sont **actives** : un `hlt` avec `IF=0`
/// arrete la machine pour de bon. Dans une attente de tache, preferer
/// [`wait_for_interrupt`], qui garantit la condition.
pub fn hlt() {
    IDLE.store(true, Ordering::SeqCst);
    // Pas de `nomem` ici : l'asm sert aussi de barriere compilateur pour
    // l'etat IDLE observe depuis l'interruption qui reveille HLT.
    unsafe {
        asm!("hlt", options(nostack, preserves_flags));
    }
    IDLE.store(false, Ordering::SeqCst);
}

/// Attend la prochaine interruption, interruptions actives quoi qu'il arrive.
///
/// C'est la primitive d'attente des boucles bloquantes du noyau (`poll`,
/// `select`, `futex`, sommeil, attente d'un fils). Un `hlt` seul y serait un
/// piege : il suffit que l'appelant ait herite d'un `IF=0` pour que le CPU
/// s'arrete sans plus jamais recevoir le tick qui devait le reveiller — la
/// machine entiere gele, sans faute ni message.
///
/// `sti` ne prend effet qu'apres l'instruction suivante : la paire `sti; hlt`
/// est donc indivisible, aucune interruption ne peut se glisser entre les deux
/// et etre perdue.
pub fn wait_for_interrupt() {
    IDLE.store(true, Ordering::SeqCst);
    // `sti; hlt` garantit qu'aucune IRQ ne peut etre perdue entre
    // l'activation des interruptions et le sommeil.
    unsafe {
        asm!("sti; hlt", options(nostack));
    }
    IDLE.store(false, Ordering::SeqCst);
}

/// Les interruptions sont-elles actives (drapeau `IF` de RFLAGS) ?
pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe { asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags)); }
    flags & (1 << 9) != 0
}

/// Lit le compteur de cycles (Time Stamp Counter).
///
/// Sert de mesure de temps grossiere tant que le timer materiel (PIT/APIC) et
/// ses interruptions ne sont pas actives.
pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags));
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Renvoie l'identifiant constructeur du CPU (12 octets ASCII).
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

/// Affiche les informations CPU detaillees (commande `cpuinfo`).
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
}
