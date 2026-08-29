//! Préfixe des lignes du journal série : heure, charge et FPS utiles.
//!
//!     [18:51:48][ 12%:  5%:  6%][FPS: 58] ...
//!
//! V16.2 construit tout le préfixe dans un tampon fixe sur pile puis l'envoie
//! au UART d'un bloc. Auparavant chaque couleur, séparateur et nombre réentrait
//! dans `serial_print!`, ce qui multipliait les attentes THRE sous QEMU/TCG.

use crate::arch::x86_64::rtc;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

static PRET: AtomicBool = AtomicBool::new(false);
static COULEURS: AtomicBool = AtomicBool::new(true);
static CACHE: AtomicU32 = AtomicU32::new(0);
static CACHE_TICK: AtomicU64 = AtomicU64::new(0);
const VALIDITE_TICKS: u64 = 250;
const PREFIXE_MAX: usize = 256;

pub fn demarre() {
    PRET.store(true, Ordering::Release);
    crate::serial_println!(
        "[journal] prefixe : [heure][cpu%:memoire physique%:nœuds RAMFS%][FPS:trames utiles]"
    );
}

pub fn pose_couleurs(actif: bool) {
    COULEURS.store(actif, Ordering::Relaxed);
}

pub fn couleurs() -> bool {
    COULEURS.load(Ordering::Relaxed)
}

const RESET: &str = "\x1b[0m";
const GRIS: &str = "\x1b[90m";
const CYAN: &str = "\x1b[36m";
const VERT: &str = "\x1b[32m";
const JAUNE: &str = "\x1b[33m";
const ROUGE: &str = "\x1b[31m";

fn teinte(pourcentage: u8) -> &'static str {
    match pourcentage {
        0..=49 => VERT,
        50..=79 => JAUNE,
        _ => ROUGE,
    }
}

fn charge() -> (u8, u8, u8) {
    if !PRET.load(Ordering::Acquire) {
        return (u8::MAX, u8::MAX, u8::MAX);
    }

    let maintenant = crate::kernel::timer::ticks();
    let cache_tick = CACHE_TICK.load(Ordering::Relaxed);
    if maintenant.wrapping_sub(cache_tick) < VALIDITE_TICKS && cache_tick != 0 {
        let packed = CACHE.load(Ordering::Relaxed);
        return (packed as u8, (packed >> 8) as u8, (packed >> 16) as u8);
    }

    let cpu = crate::kernel::timer::cpu_load_pct();
    let (utilise, total) = crate::kernel::vmm::frame_stats_relaxed();
    let ram = if total > 0 { (utilise * 100 / total) as u8 } else { 0 };
    let noeuds = crate::fs::ramfs::used_nodes_relaxed();
    let disque = if crate::fs::ramfs::MAX_NODES > 0 {
        (noeuds * 100 / crate::fs::ramfs::MAX_NODES) as u8
    } else {
        0
    };

    let packed = cpu as u32 | (ram as u32) << 8 | (disque as u32) << 16;
    CACHE.store(packed, Ordering::Relaxed);
    CACHE_TICK.store(maintenant.max(1), Ordering::Relaxed);
    (cpu, ram, disque)
}

// BOUCHAUD_V16_2_PREFIX_BUFFER
struct Prefixe {
    donnees: [u8; PREFIXE_MAX],
    len: usize,
}

impl Prefixe {
    const fn neuf() -> Self {
        Self { donnees: [0; PREFIXE_MAX], len: 0 }
    }

    fn octets(&self) -> &[u8] {
        &self.donnees[..self.len]
    }
}

impl fmt::Write for Prefixe {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let octets = s.as_bytes();
        if self.len + octets.len() > self.donnees.len() {
            return Err(fmt::Error);
        }
        self.donnees[self.len..self.len + octets.len()].copy_from_slice(octets);
        self.len += octets.len();
        Ok(())
    }
}

#[inline]
fn ajoute_couleur(out: &mut Prefixe, couleur: &str) {
    if couleurs() {
        let _ = out.write_str(couleur);
    }
}

fn ajoute_pourcentage(out: &mut Prefixe, valeur: u8) {
    if valeur == u8::MAX {
        ajoute_couleur(out, GRIS);
        let _ = out.write_str(" --");
        return;
    }
    ajoute_couleur(out, teinte(valeur));
    let _ = out.write_fmt(format_args!("{:3}", valeur));
}

fn ajoute_fps(out: &mut Prefixe, valeur: Option<u16>) {
    match valeur {
        Some(valeur) => {
            ajoute_couleur(out, CYAN);
            let _ = out.write_fmt(format_args!("{:3}", valeur.min(999)));
        }
        None => {
            ajoute_couleur(out, GRIS);
            let _ = out.write_str(" --");
        }
    }
}

pub fn ecris_prefixe() {
    let heure = rtc::now();
    let (cpu, ram, disque) = charge();
    let frame = if PRET.load(Ordering::Acquire) {
        Some(crate::gui::frame_clock::snapshot())
    } else {
        None
    };

    let mut out = Prefixe::neuf();

    ajoute_couleur(&mut out, GRIS);
    let _ = out.write_str("[");
    ajoute_couleur(&mut out, CYAN);
    let _ = out.write_fmt(format_args!(
        "{:02}:{:02}:{:02}",
        heure.hour, heure.minute, heure.second
    ));
    ajoute_couleur(&mut out, GRIS);
    let _ = out.write_str("][");

    ajoute_pourcentage(&mut out, cpu);
    ajoute_couleur(&mut out, GRIS);
    let _ = out.write_str("%:");
    ajoute_pourcentage(&mut out, ram);
    ajoute_couleur(&mut out, GRIS);
    let _ = out.write_str("%:");
    ajoute_pourcentage(&mut out, disque);
    ajoute_couleur(&mut out, GRIS);
    let _ = out.write_str("%][FPS:");

    ajoute_fps(&mut out, frame.filter(|s| s.active).map(|s| s.fps_arrondi()));
    ajoute_couleur(&mut out, GRIS);
    let _ = out.write_str("] ");
    ajoute_couleur(&mut out, RESET);

    crate::drivers::serial::ecris_octets_sans_prefixe(out.octets());
}
