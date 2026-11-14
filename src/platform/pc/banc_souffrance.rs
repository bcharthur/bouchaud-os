//! BOUCHAUD_SOUFFRANCE_PROPRIETAIRE_V1 (#283) -- temoin du noyau fige.
//!
//! Deux gels de #283 (b95fc0a, 2e61a6b) n'ont laisse qu'une ligne :
//! `[DIAG-EN-SOUFFRANCE] ... spin=[0x181906fb810/1085334 ...]`. Le fil
//! `diag-noyau`, qui imprime le proprietaire et le site d'appel
//! (`[SMP-SPIN]`), ne tournait plus. Ce temoin reproduit cet etat sur deux
//! coeurs :
//!
//!   * un fil noyau prend `VERROU` et ne le rend jamais (coeur 1 en pratique :
//!     le contexte de demarrage occupe le coeur 0) ;
//!   * le contexte de demarrage, des que le verrou est tenu, l'attend a son
//!     tour -- interruptions ouvertes, comme le `services-metrics` du gel de
//!     2e61a6b.
//!
//! Plus aucune tache ne s'ordonnance ; seule l'IRQ du minuteur parle encore.
//! `tools/ci/run_souffrance_verrou.sh` exige que sa ligne nomme le verrou, le
//! coeur proprietaire et la ligne d'attente ci-dessous.
//!
//! Compile seulement sous `banc-souffrance-verrou` : jamais livre.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::kernel::sync::SpinLock;

static VERROU: SpinLock<u64> = SpinLock::new(0);
static TENU: AtomicBool = AtomicBool::new(false);

fn detenteur() -> ! {
    let mut garde = VERROU.lock();
    *garde += 1;
    TENU.store(true, Ordering::Release);
    crate::serial_println!(
        "BOUCHAUD_BANC_SOUFFRANCE detenteur cpu={} verrou={:#x}",
        crate::arch::x86_64::smp::cpu_index(),
        &VERROU as *const _ as usize,
    );
    // Le verrou n'est JAMAIS rendu.
    loop {
        core::hint::spin_loop();
    }
}

pub fn demarre() {
    crate::serial_println!("BOUCHAUD_BANC_SOUFFRANCE armement");
    if !crate::kernel::task::spawn_noyau(detenteur, "banc-souffrance") {
        crate::serial_println!("BOUCHAUD_BANC_SOUFFRANCE_ECHEC spawn");
        return;
    }
    let debut = crate::kernel::timer::ticks();
    while !TENU.load(Ordering::Acquire) {
        if crate::kernel::timer::ticks().saturating_sub(debut) > 20 * crate::kernel::timer::TICKS_PER_SECOND {
            crate::serial_println!("BOUCHAUD_BANC_SOUFFRANCE_ECHEC detenteur_jamais_ordonnance");
            return;
        }
        core::hint::spin_loop();
    }
    crate::serial_println!(
        "BOUCHAUD_BANC_SOUFFRANCE attente cpu={}",
        crate::arch::x86_64::smp::cpu_index(),
    );
    // `line!()` et l'appel a `lock()` sur la MEME ligne : c'est elle que
    // `Location::caller()` publie pour le coeur qui attend.
    let (_ligne_attente, garde) = (line!(), VERROU.lock());
    drop(garde);
    crate::serial_println!("BOUCHAUD_BANC_SOUFFRANCE_ECHEC verrou_obtenu");
}
