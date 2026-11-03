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

use core::sync::atomic::{AtomicBool, Ordering};

static DIAGNOSTIC: AtomicBool = AtomicBool::new(false);

/// Lit les reglages exportes. Appele une fois, au demarrage du bureau.
pub fn charge() {
    for entree in crate::shell::exported().iter() {
        if let Some(v) = entree.strip_prefix("BO_ATELIER_DIAGNOSTIC=") {
            DIAGNOSTIC.store(matches!(v.trim(), "1" | "oui" | "on"), Ordering::Relaxed);
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
