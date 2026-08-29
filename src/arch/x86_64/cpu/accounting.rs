// Accounting SMP et charge CPU.

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

// Fenêtre de charge par CPU logique. Le PIT BSP échantillonne IDLE[] pour tous
// les CPU online; aucun IPI supplémentaire n'est nécessaire.
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
        let sample =
            100u64.saturating_sub(idle.saturating_mul(100) / elapsed) as u8;
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
    if total < target {
        return;
    }

    let idle_ticks = LOAD_WINDOW_IDLE.swap(0, Ordering::Relaxed);
    let window = LOAD_WINDOW_TOTAL.swap(0, Ordering::Relaxed).max(1);
    let sample =
        100u64.saturating_sub(idle_ticks.saturating_mul(100) / window) as u8;
    let old = CPU_LOAD_PCT.load(Ordering::Relaxed);
    let filtered = if old == 0 {
        sample
    } else {
        ((old as u16 * 3 + sample as u16) / 4) as u8
    };
    CPU_LOAD_PCT.store(filtered.min(100), Ordering::Relaxed);
}

/// Appelé par le PIT sur le BSP.
pub fn account_timer_tick(interrupted_user: bool) -> bool {
    note_pit_tick();

    let bsp = hardware_cpu_index();
    let online = smp::schedulable_cpus().max(1).min(smp::MAX_CPUS);
    let mut idle_count = 0u64;
    for cpu in 0..online {
        if IDLE[cpu].load(Ordering::Acquire) {
            idle_count += 1;
        }
    }

    update_core_load_windows(online);

    let bsp_idle = IDLE[bsp].load(Ordering::Acquire);
    CPU_TOTAL_TICKS.fetch_add(online as u64, Ordering::Relaxed);
    CPU_IDLE_TICKS.fetch_add(idle_count, Ordering::Relaxed);

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
