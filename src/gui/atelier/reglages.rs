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
use super::usage::{self, Usage};

static DIAGNOSTIC: AtomicBool = AtomicBool::new(false);
static ANIMATIONS: AtomicU8 = AtomicU8::new(0);
static MODE_USAGE: AtomicU8 = AtomicU8::new(0);

const fn code(mode: Mode) -> u8 {
    match mode {
        Mode::Normal => 0,
        Mode::Reduit => 1,
        Mode::Aucun => 2,
    }
}

/// Mode d'usage courant. Personnalise indique un ajustement manuel.
pub fn usage_courant() -> Usage {
    match MODE_USAGE.load(Ordering::Relaxed) {
        1 => Usage::Travail,
        2 => Usage::Focus,
        3 => Usage::Jeu,
        4 => Usage::Personnalise,
        _ => Usage::Standard,
    }
}
const fn code_usage(mode: Usage) -> u8 {
    match mode {
        Usage::Standard => 0, Usage::Travail => 1, Usage::Focus => 2,
        Usage::Jeu => 3, Usage::Personnalise => 4,
    }
}
/// Change de vraies preferences visuelles, pas de politique scheduler.
pub fn regle_usage(mode: Usage) {
    if let Some(profil) = usage::profil(mode) {
        ANIMATIONS.store(code(profil.animations), Ordering::Relaxed);
        DIAGNOSTIC.store(profil.diagnostic, Ordering::Relaxed);
        MODE_USAGE.store(code_usage(mode), Ordering::Relaxed);
        crate::serial_println!(
            "BOUCHAUD_ATELIER_USAGE mode={:?} scope=VISUEL animations={:?} diagnostic={}",
            mode, profil.animations, profil.diagnostic as u8
        );
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
        MODE_USAGE.store(code_usage(Usage::Personnalise), Ordering::Relaxed);
        crate::serial_println!("BOUCHAUD_ATELIER_REGLAGE animations={:?}", mode);
    }
}

/// Lit les reglages exportes. Appele une fois, au demarrage du bureau.
pub fn charge() {
    // Les reglages exportes explicitement priment, quel que soit leur ordre.
    let mut choix_usage = None;
    let mut choix_diagnostic = None;
    let mut choix_animations = None;
    for entree in crate::shell::exported().iter() {
        if let Some(v) = entree.strip_prefix("BO_ATELIER_USAGE=") {
            choix_usage = usage::depuis_env(v);
        }
        if let Some(v) = entree.strip_prefix("BO_ATELIER_DIAGNOSTIC=") {
            choix_diagnostic = Some(matches!(v.trim(), "1" | "oui" | "on"));
        }
        if let Some(v) = entree.strip_prefix("BO_ATELIER_ANIMATIONS=") {
            choix_animations = Some(match v.trim() {
                "reduites" | "reduit" => Mode::Reduit,
                "aucune" | "0" | "non" => Mode::Aucun,
                _ => Mode::Normal,
            });
        }
    }
    if let Some(mode) = choix_usage {
        if let Some(p) = usage::profil(mode) {
            ANIMATIONS.store(code(p.animations), Ordering::Relaxed);
            DIAGNOSTIC.store(p.diagnostic, Ordering::Relaxed);
            MODE_USAGE.store(code_usage(mode), Ordering::Relaxed);
        }
    } else {
        MODE_USAGE.store(code_usage(Usage::Standard), Ordering::Relaxed);
    }
    if let Some(mode) = choix_animations {
        if animations() != mode {
            ANIMATIONS.store(code(mode), Ordering::Relaxed);
            MODE_USAGE.store(code_usage(Usage::Personnalise), Ordering::Relaxed);
        }
    }
    if let Some(value) = choix_diagnostic {
        if diagnostic() != value {
            DIAGNOSTIC.store(value, Ordering::Relaxed);
            MODE_USAGE.store(code_usage(Usage::Personnalise), Ordering::Relaxed);
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
    MODE_USAGE.store(code_usage(Usage::Personnalise), Ordering::Relaxed);
    crate::serial_println!("BOUCHAUD_ATELIER_REGLAGE diagnostic={}", nouveau as u8);
    nouveau
}
