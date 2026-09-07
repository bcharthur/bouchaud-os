//! Gestion du temps noyau : ticks PIT et mesure de charge CPU via TSC.

use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::arch::x86_64::cpu;
use crate::arch::x86_64::interrupts;
use crate::arch::x86_64::ports::{inb, outb};

static TICKS: AtomicU64 = AtomicU64::new(0);
static BOOT_TSC: AtomicU64 = AtomicU64::new(0);
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
static LAST_MONOTONIC_NS: AtomicU64 = AtomicU64::new(0);
static INVARIANT_TSC: AtomicBool = AtomicBool::new(false);
static HAS_RDTSCP: AtomicBool = AtomicBool::new(false);
static HPET_BASE_VIRT: AtomicU64 = AtomicU64::new(0);
static HPET_PERIOD_FS: AtomicU64 = AtomicU64::new(0);
static HPET_BOOT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Frequence de base du PIT 8253/8254, en hertz.
const PIT_BASE_HZ: u32 = 1_193_182;

/// Calibre le TSC sans aucune interruption.
///
/// Le canal 2 du PIT n'est pas raccorde a IRQ0 : on programme un one-shot
/// d'environ 50 ms puis on POLL son bit OUT2 sur le port 0x61.
/// Cela fonctionne meme si le firmware/ACPI ne route pas le vieux PIT
/// vers le PIC 8259.
fn calibrate_tsc_hz_pit2(rdtscp: bool) -> Option<u64> {
    const COUNT: u16 = 59_659; // ~50 ms a 1_193_182 Hz
    const MAX_POLLS: usize = 5_000_000;

    unsafe {
        let original = inb(0x61);

        // Haut-parleur coupe + gate 2 basse.
        outb(0x61, original & !0x03);

        // Channel 2, lobyte/hibyte, mode 0, binaire.
        outb(0x43, 0xB0);
        outb(0x42, (COUNT & 0xff) as u8);
        outb(0x42, (COUNT >> 8) as u8);

        // En mode 0, OUT2 doit etre bas pendant le comptage.
        let mut armed = false;
        for _ in 0..10_000 {
            if inb(0x61) & 0x20 == 0 {
                armed = true;
                break;
            }
            core::hint::spin_loop();
        }

        if !armed {
            outb(0x61, original);
            return None;
        }

        let start = cpu::read_tsc_ordered(rdtscp);

        // Front montant de gate => demarrage.
        outb(0x61, (original & !0x02) | 0x01);

        let mut completed = false;
        for _ in 0..MAX_POLLS {
            if inb(0x61) & 0x20 != 0 {
                completed = true;
                break;
            }
            core::hint::spin_loop();
        }

        let end = cpu::read_tsc_ordered(rdtscp);
        outb(0x61, original);

        if !completed {
            return None;
        }

        let cycles = end.wrapping_sub(start);
        if cycles == 0 {
            return None;
        }

        let hz = ((cycles as u128) * PIT_BASE_HZ as u128 / COUNT as u128)
            .min(u64::MAX as u128) as u64;

        // Filtre une mesure manifestement impossible.
        if !(100_000_000..=10_000_000_000).contains(&hz) {
            return None;
        }

        Some(hz)
    }
}

pub fn init() {
    let (invariant, rdtscp) = detect_tsc_features();
    INVARIANT_TSC.store(invariant, Ordering::Release);
    HAS_RDTSCP.store(rdtscp, Ordering::Release);
    let boot_tsc = cpu::read_tsc_ordered(rdtscp);
    BOOT_TSC.store(boot_tsc, Ordering::Release);
    // Un TSC dont la frequence varie avec P-state n'est pas une horloge. Dans
    // ce cas on force la calibration/fallback au lieu de publier des deadlines
    // architecturales trompeuses.
    let tsc_hz = if invariant {
        detect_tsc_hz()
            .or_else(|| calibrate_tsc_hz_pit2(rdtscp))
            .unwrap_or(0)
    } else {
        0
    };

    TSC_HZ.store(tsc_hz, Ordering::Release);

    // Rend immediatement utilisables les budgets en cycles, meme avant IRQ0.
    if tsc_hz != 0 {
        unsafe {
            CYCLES_PER_MS = tsc_hz / 1000;
        }
        crate::serial_println!(
            "BOUCHAUD_TSC_EARLY_CALIBRATION_OK hz={}",
            tsc_hz
        );
    }
    unsafe {
        LAST_FRAME_TSC = boot_tsc;
        RENDER_START_TSC = boot_tsc;
    }
    program_pit(TICKS_PER_SECOND as u32);
}


pub fn install_hpet_source(phys_base: u64) -> bool {
    if phys_base == 0 { return false; }
    let base = crate::kernel::memory::phys_to_virt(phys_base);
    unsafe {
        let caps = core::ptr::read_volatile(base as *const u64);
        let period_fs = caps >> 32;
        if period_fs == 0 || period_fs > 100_000_000 { return false; }
        let config = base.add(0x10) as *mut u64;
        let current = core::ptr::read_volatile(config);
        core::ptr::write_volatile(config, current | 1);
        let counter = core::ptr::read_volatile(base.add(0xF0) as *const u64);
        HPET_BASE_VIRT.store(base as u64, Ordering::Release);
        HPET_PERIOD_FS.store(period_fs, Ordering::Release);
        HPET_BOOT_COUNTER.store(counter, Ordering::Release);
        crate::serial_println!("BOUCHAUD_HPET_MONOTONIC_READY base={:#x} period_fs={}", phys_base, period_fs);
        true
    }
}

fn hpet_monotonic_ns() -> Option<u64> {
    let base = HPET_BASE_VIRT.load(Ordering::Acquire);
    let period_fs = HPET_PERIOD_FS.load(Ordering::Acquire);
    if base == 0 || period_fs == 0 { return None; }
    let now = unsafe { core::ptr::read_volatile((base as *mut u8).add(0xF0) as *const u64) };
    let delta = now.wrapping_sub(HPET_BOOT_COUNTER.load(Ordering::Acquire));
    Some(((delta as u128).saturating_mul(period_fs as u128) / 1_000_000u128).min(u64::MAX as u128) as u64)
}

/// Programme le canal 0 du PIT a la frequence demandee.
///
/// Sans reprogrammation le PIT bat a 18,2 Hz, soit 55 ms entre deux ticks :
/// c'est la granularite de tout ce qui attend dans le noyau (`nanosleep`,
/// timeout de `poll`, quantum de preemption). Une interface graphique qui vise
/// 60 images par seconde a besoin de 16 ms ; on descend donc a la milliseconde.
fn program_pit(hz: u32) {
    let divisor = (PIT_BASE_HZ / hz).clamp(1, 65535) as u16;
    unsafe {
        // Canal 0, acces poids faible puis fort, mode 3 (creneau), binaire.
        outb(0x43, 0x36);
        outb(0x40, (divisor & 0xFF) as u8);
        outb(0x40, (divisor >> 8) as u8);
    }
}

pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Frequence du tick noyau. Le PIT est programme sur cette valeur par [`init`].
pub const TICKS_PER_SECOND: u64 = 1000;

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Acquire)
}

pub fn seconds() -> u64 {
    ticks() / TICKS_PER_SECOND
}

/// Millisecondes ecoulees depuis le boot.
///
/// Le tick etant a la milliseconde, c'est directement le compteur ; on garde
/// une fonction dediee pour que le reste du code ne depende pas de ce choix.
pub fn monotonic_ms() -> u64 {
    monotonic_ns() / 1_000_000
}

/// Nanosecondes monotones depuis le boot, independantes de la livraison des
/// IRQ PIT. QEMU/TCG peut retarder fortement IRQ0 lorsqu'un vCPU monopolise le
/// traducteur; le TSC virtuel continue en revanche de representer le temps de
/// la machine invite.
pub fn monotonic_ns() -> u64 {
    let hz = TSC_HZ.load(Ordering::Acquire);
    let candidate = if hz != 0 {
        let cycles = cpu::read_tsc_ordered(HAS_RDTSCP.load(Ordering::Relaxed))
            .wrapping_sub(BOOT_TSC.load(Ordering::Acquire));
        ((cycles as u128).saturating_mul(1_000_000_000) / hz as u128)
            .min(u64::MAX as u128) as u64
    } else if let Some(hpet_ns) = hpet_monotonic_ns() {
        hpet_ns
    } else {
        ticks().saturating_mul(1_000_000_000 / TICKS_PER_SECOND)
    };

    // Preserve la monotonie entre CPU meme si leur TSC presente un leger
    // decalage. fetch_max renvoie l'ancienne valeur, d'ou le max final.
    candidate.max(LAST_MONOTONIC_NS.fetch_max(candidate, Ordering::AcqRel))
}

/// Frequence architecturale du TSC annoncee par CPUID, si exploitable.
fn detect_tsc_hz() -> Option<u64> {
    let max_basic = __cpuid(0).eax;
    if max_basic >= 0x15 {
        let leaf = __cpuid_count(0x15, 0);
        if leaf.eax != 0 && leaf.ebx != 0 && leaf.ecx != 0 {
            let hz = (leaf.ecx as u128)
                .saturating_mul(leaf.ebx as u128)
                / leaf.eax as u128;
            if hz != 0 && hz <= u64::MAX as u128 {
                return Some(hz as u64);
            }
        }
    }
    if max_basic >= 0x16 {
        let mhz = __cpuid(0x16).eax as u64;
        if mhz != 0 {
            return mhz.checked_mul(1_000_000);
        }
    }
    None
}

fn detect_tsc_features() -> (bool, bool) {
    let max_extended = __cpuid(0x8000_0000).eax;
    let rdtscp = max_extended >= 0x8000_0001
        && (__cpuid(0x8000_0001).edx & (1 << 27)) != 0;
    let invariant = max_extended >= 0x8000_0007
        && (__cpuid(0x8000_0007).edx & (1 << 8)) != 0;
    (invariant, rdtscp)
}

/// Convertit une duree en millisecondes en nombre de ticks, arrondi au
/// superieur pour ne jamais rendre la main trop tot.
pub fn ms_to_ticks(ms: u64) -> u64 {
    ms.saturating_mul(TICKS_PER_SECOND).div_ceil(1000)
}

pub fn cycles_since_boot() -> u64 {
    cpu::read_tsc_ordered(HAS_RDTSCP.load(Ordering::Relaxed))
        .wrapping_sub(BOOT_TSC.load(Ordering::Acquire))
}

pub fn timer_enabled() -> bool {
    interrupts::enabled()
}

pub fn tsc_hz() -> Option<u64> {
    let hz = TSC_HZ.load(Ordering::Acquire);
    (hz != 0).then_some(hz)
}

// ── Calibration TSC -> temps reel ──────────────────────────────────────────
// Les logs de diagnostic (reseau, DOM, CSS, layout, peinture) affichent des
// compteurs de cycles bruts ("Mc" = millions de cycles) : utiles pour comparer
// deux phases entre elles, mais illisibles pour savoir "ca a pris combien de
// temps ?" en vrai. On calibre donc une fois, au boot, la frequence du TSC en
// mesurant les cycles ecoules sur quelques ticks PIT (frequence fixe et connue,
// 18.2 Hz, independante de la vitesse d'emulation QEMU) ; cycles_to_ms()
// convertit ensuite n'importe quel delta de cycles en millisecondes reelles.
static mut CYCLES_PER_MS: u64 = 0;

/// Calibre CYCLES_PER_MS. A appeler une fois les interruptions actives (IRQ0
/// doit deja faire avancer `ticks()`), sinon la calibration echoue silencieusement
/// (CYCLES_PER_MS reste a 0, cycles_to_ms() renvoie alors 0 au lieu de planter).
pub fn calibrate() {
    // ~250 ms : assez long pour lisser le bruit de mesure, assez court pour ne
    // pas ralentir le boot.
    const CAL_TICKS: u64 = TICKS_PER_SECOND / 4;
    let start_tick = ticks();
    let start_tsc = cpu::rdtsc();
    let mut spins: u64 = 0;
    while ticks().wrapping_sub(start_tick) < CAL_TICKS {
        core::hint::spin_loop();
        spins += 1;
        // Garde-fou : abandonne (calibration a 0) plutot que de bloquer le boot
        // si les interruptions ne tournent pas encore pour une raison quelconque.
        if spins > 20_000_000 {
            crate::serial_println!(
                "BOUCHAUD_TIMER_PIT_CALIBRATION_TIMEOUT ticks={}",
                ticks().wrapping_sub(start_tick)
            );
            return;
        }
    }
    let elapsed_ticks = ticks().wrapping_sub(start_tick).max(1);
    let elapsed_tsc = cpu::rdtsc().wrapping_sub(start_tsc);
    let elapsed_ms = elapsed_ticks * 1000 / TICKS_PER_SECOND;
    if elapsed_ms > 0 {
        let cycles_per_ms = elapsed_tsc / elapsed_ms;
        unsafe { CYCLES_PER_MS = cycles_per_ms; }
        // La calibration PIT n'est qu'un repli pour les machines qui ne
        // publient aucune frequence CPUID. Elle ne remplace jamais une valeur
        // architecturale, car des IRQ perdues faussent précisément ce calcul.
        if TSC_HZ.load(Ordering::Acquire) == 0 {
            TSC_HZ.store(cycles_per_ms.saturating_mul(1000), Ordering::Release);
        }
    }
}

/// Convertit un delta de cycles TSC en millisecondes reelles (0 si non calibre).
pub fn cycles_to_ms(cycles: u64) -> u64 {
    let cpm = unsafe { CYCLES_PER_MS };
    if cpm == 0 { 0 } else { cycles / cpm }
}

/// Convertit des millisecondes reelles en delta de cycles TSC (0 si non calibre,
/// ce qui desactive tout budget calcule a partir de cette valeur plutot que de
/// planter ou de bloquer indefiniment).
pub fn ms_to_cycles(ms: u64) -> u64 {
    let cpm = unsafe { CYCLES_PER_MS };
    ms.saturating_mul(cpm)
}

// ── Metriques CPU / rendu ────────────────────────────────────────────────────
//
// La charge systeme ne depend plus du compositeur : elle vient du CPU Core et
// de la decomposition busy/idle mesuree par IRQ0. Le TSC reste utile pour
// mesurer le cout propre du rendu graphique.

static mut RENDER_LOAD: u8 = 0;
static mut RENDER_START_TSC: u64 = 0;
static mut LAST_FRAME_TSC: u64 = 0;

/// Marque le debut de la phase de rendu GUI.
pub fn frame_start() {
    unsafe { RENDER_START_TSC = cpu::rdtsc(); }
}

/// Mesure uniquement la part du temps de frame consacree au rendu.
pub fn mark_frame() {
    unsafe {
        let now = cpu::rdtsc();
        let render = now.wrapping_sub(RENDER_START_TSC);
        let total = now.wrapping_sub(LAST_FRAME_TSC);
        LAST_FRAME_TSC = now;
        if total >= 1_000 && total < 2_000_000_000 {
            let pct = ((render * 100) / total).min(100) as u8;
            RENDER_LOAD = ((RENDER_LOAD as u32 * 7 + pct as u32) / 8) as u8;
        }
    }
}

/// Cout recent du compositeur dans sa propre cadence de frame.
pub fn render_load_pct() -> u8 {
    unsafe { RENDER_LOAD }
}

/// Charge CPU systeme recente (0–100 %).
///
/// Source unique pour le journal, la barre du bureau et les diagnostics.
pub fn cpu_load_pct() -> u8 {
    cpu::load_percent()
}
