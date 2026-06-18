//! Gestion du temps noyau : ticks et mesure grossiere via le TSC.
//!
//! Tant que les interruptions timer (PIT/APIC) ne sont pas activees, le
//! compteur de ticks reste a zero : il sera incremente par le handler d'IRQ0
//! en V0.7. En attendant, on expose le compteur de cycles CPU (TSC) comme
//! mesure de liveness honnete.

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::interrupts;

/// Compteur de ticks timer. Incremente par l'IRQ0 une fois le timer active.
static mut TICKS: u64 = 0;

/// Valeur du TSC au boot, base de la mesure "cycles depuis le demarrage".
static mut BOOT_TSC: u64 = 0;

/// Capture l'instant de boot. A appeler une fois tres tot au demarrage.
pub fn init() {
    unsafe { BOOT_TSC = cpu::rdtsc(); }
}

/// Increment du compteur de ticks. Sera appele par le handler d'IRQ0.
pub fn tick() {
    unsafe {
        let t = core::ptr::read_volatile(&TICKS);
        core::ptr::write_volatile(&mut TICKS, t.wrapping_add(1));
    }
}

/// Frequence par defaut du PIT (canal 0) non reprogramme : ~18.2065 Hz.
pub const TICKS_PER_SECOND: u64 = 18;

/// Nombre de ticks timer ecoules (0 tant que le timer n'est pas active).
/// Lecture volatile : le compteur est modifie par l'IRQ0, le compilateur ne
/// doit pas mettre cette lecture en cache (boucles d'attente optimisees).
pub fn ticks() -> u64 {
    unsafe { core::ptr::read_volatile(&TICKS) }
}

/// Duree approximative depuis le boot, en secondes (base PIT par defaut).
pub fn seconds() -> u64 {
    ticks() / TICKS_PER_SECOND
}

/// Cycles CPU ecoules depuis le boot (approximation via TSC).
pub fn cycles_since_boot() -> u64 {
    unsafe { cpu::rdtsc().wrapping_sub(BOOT_TSC) }
}

/// Indique si une vraie base de temps par interruption est active.
pub fn timer_enabled() -> bool {
    interrupts::enabled()
}
