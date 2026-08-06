//! Gestion du temps noyau : ticks PIT et mesure de charge CPU via TSC.

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::interrupts;
use crate::arch::x86_64::ports::outb;

static mut TICKS: u64 = 0;
static mut BOOT_TSC: u64 = 0;

/// Frequence de base du PIT 8253/8254, en hertz.
const PIT_BASE_HZ: u32 = 1_193_182;

pub fn init() {
    unsafe {
        BOOT_TSC = cpu::rdtsc();
        LAST_FRAME_TSC = BOOT_TSC;
        RENDER_START_TSC = BOOT_TSC;
    }
    program_pit(TICKS_PER_SECOND as u32);
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
    unsafe {
        let t = core::ptr::read_volatile(&TICKS);
        core::ptr::write_volatile(&mut TICKS, t.wrapping_add(1));
    }
}

/// Frequence du tick noyau. Le PIT est programme sur cette valeur par [`init`].
pub const TICKS_PER_SECOND: u64 = 1000;

pub fn ticks() -> u64 {
    unsafe { core::ptr::read_volatile(&TICKS) }
}

pub fn seconds() -> u64 {
    ticks() / TICKS_PER_SECOND
}

/// Millisecondes ecoulees depuis le boot.
///
/// Le tick etant a la milliseconde, c'est directement le compteur ; on garde
/// une fonction dediee pour que le reste du code ne depende pas de ce choix.
pub fn monotonic_ms() -> u64 {
    ticks() * 1000 / TICKS_PER_SECOND
}

/// Convertit une duree en millisecondes en nombre de ticks, arrondi au
/// superieur pour ne jamais rendre la main trop tot.
pub fn ms_to_ticks(ms: u64) -> u64 {
    ms.saturating_mul(TICKS_PER_SECOND).div_ceil(1000)
}

pub fn cycles_since_boot() -> u64 {
    unsafe { cpu::rdtsc().wrapping_sub(BOOT_TSC) }
}

pub fn timer_enabled() -> bool {
    interrupts::enabled()
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
        if spins > 500_000_000 { return; }
    }
    let elapsed_ticks = ticks().wrapping_sub(start_tick).max(1);
    let elapsed_tsc = cpu::rdtsc().wrapping_sub(start_tsc);
    let elapsed_ms = elapsed_ticks * 1000 / TICKS_PER_SECOND;
    if elapsed_ms > 0 {
        unsafe { CYCLES_PER_MS = elapsed_tsc / elapsed_ms; }
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
