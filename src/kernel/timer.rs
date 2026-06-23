//! Gestion du temps noyau : ticks PIT et mesure de charge CPU via TSC.

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::interrupts;

static mut TICKS: u64 = 0;
static mut BOOT_TSC: u64 = 0;

pub fn init() {
    unsafe {
        BOOT_TSC = cpu::rdtsc();
        LAST_FRAME_TSC = BOOT_TSC;
        RENDER_START_TSC = BOOT_TSC;
    }
}

pub fn tick() {
    unsafe {
        let t = core::ptr::read_volatile(&TICKS);
        core::ptr::write_volatile(&mut TICKS, t.wrapping_add(1));
    }
}

pub const TICKS_PER_SECOND: u64 = 18;

pub fn ticks() -> u64 {
    unsafe { core::ptr::read_volatile(&TICKS) }
}

pub fn seconds() -> u64 {
    ticks() / TICKS_PER_SECOND
}

pub fn cycles_since_boot() -> u64 {
    unsafe { cpu::rdtsc().wrapping_sub(BOOT_TSC) }
}

pub fn timer_enabled() -> bool {
    interrupts::enabled()
}

// ── Charge CPU via TSC ────────────────────────────────────────────────────────
// Mesure le ratio (cycles de rendu) / (cycles totaux de frame).
// Avec HLT entre frames, le temps total = rendu + sommeil.
// CPU% = rendu / (rendu + sommeil) → reflete la vraie charge.

static mut CPU_LOAD: u8 = 0;
static mut RENDER_START_TSC: u64 = 0;
static mut LAST_FRAME_TSC: u64 = 0;

/// Marque le début de la phase de rendu. Appeler au tout début de la boucle GUI.
pub fn frame_start() {
    unsafe { RENDER_START_TSC = cpu::rdtsc(); }
}

/// Marque la fin du rendu et met à jour CPU%. Appeler juste avant hlt().
pub fn mark_frame() {
    unsafe {
        let now = cpu::rdtsc();
        let render = now.wrapping_sub(RENDER_START_TSC);
        let total  = now.wrapping_sub(LAST_FRAME_TSC);
        LAST_FRAME_TSC = now;
        // Sanity: ignore frame times < 1 000 cycles ou > 2 milliards (1 sec à 2 GHz).
        if total >= 1_000 && total < 2_000_000_000 {
            let pct = ((render * 100) / total).min(100) as u8;
            // EWMA α = 1/8 : lisse les pics sans trop ralentir la réponse.
            CPU_LOAD = ((CPU_LOAD as u32 * 7 + pct as u32) / 8) as u8;
        }
    }
}

/// Charge CPU estimée (0–100 %). Mise à jour par `mark_frame()`.
pub fn cpu_load_pct() -> u8 {
    unsafe { CPU_LOAD }
}
