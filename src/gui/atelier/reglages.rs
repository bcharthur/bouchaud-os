//! Reglages Atelier tenus par le bureau (BOUCHAUD_ATELIER_REGLAGES_V1).
//!
//! Un reglage n'existe ici que s'il commande quelque chose de reel. Il se lit
//! au demarrage depuis l'environnement exporte du shell, et se change depuis le
//! centre de controle.
//!
//! * `BO_ATELIER_DIAGNOSTIC=1` : le compteur de trames reste dans la barre du
//!   haut. Par defaut il n'y est pas -- la barre montre l'etat du systeme, pas
//!   ses mesures ; le compteur reste ecrit dans chaque ligne du journal
//!   (`[FPS:...]`) et lisible dans Insights.
//! * `BO_ATELIER_ANIMATIONS=normales|reduites|aucune` : le mouvement des
//!   fenetres (`atelier::animations::Mode`). « reduites » garde le fondu et
//!   retire tout deplacement ; « aucune » ne lance rien.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use super::animations::Mode;

static DIAGNOSTIC: AtomicBool = AtomicBool::new(false);
static ANIMATIONS: AtomicU8 = AtomicU8::new(0);

const fn code(mode: Mode) -> u8 {
    match mode {
        Mode::Normal => 0,
        Mode::Reduit => 1,
        Mode::Aucun => 2,
    }
}

/// Le mode de mouvement courant.
pub fn animations() -> Mode {
    match ANIMATIONS.load(Ordering::Relaxed) {
        1 => Mode::Reduit,
        2 => Mode::Aucun,
        _ => Mode::Normal,
    }
}

/// Change le mode de mouvement.
pub fn regle_animations(mode: Mode) {
    if animations() != mode {
        ANIMATIONS.store(code(mode), Ordering::Relaxed);
        crate::serial_println!("BOUCHAUD_ATELIER_REGLAGE animations={:?}", mode);
    }
}

/// Lit les reglages exportes. Appele une fois, au demarrage du bureau.
pub fn charge() {
    for entree in crate::shell::exported().iter() {
        if let Some(v) = entree.strip_prefix("BO_ATELIER_DIAGNOSTIC=") {
            DIAGNOSTIC.store(matches!(v.trim(), "1" | "oui" | "on"), Ordering::Relaxed);
        }
        if let Some(v) = entree.strip_prefix("BO_ATELIER_ANIMATIONS=") {
            let mode = match v.trim() {
                "reduites" | "reduit" => Mode::Reduit,
                "aucune" | "0" | "non" => Mode::Aucun,
                _ => Mode::Normal,
            };
            ANIMATIONS.store(code(mode), Ordering::Relaxed);
        }
    }
}

/// Le mode diagnostic est-il actif ?
pub fn diagnostic() -> bool {
    DIAGNOSTIC.load(Ordering::Relaxed)
}

/// Bascule le mode diagnostic ; rend le nouvel etat.
pub fn bascule_diagnostic() -> bool {
    let nouveau = !diagnostic();
    DIAGNOSTIC.store(nouveau, Ordering::Relaxed);
    crate::serial_println!("BOUCHAUD_ATELIER_REGLAGE diagnostic={}", nouveau as u8);
    nouveau
}
