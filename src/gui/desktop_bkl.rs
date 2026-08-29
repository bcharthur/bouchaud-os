// Bouchaud V9 - BKL cooperatif du thread desktop.
//
// Le desktop reste un kernel thread legacy : `kernel_task_trampoline()` garde
// donc son KernelGuard racine. V9 ne casse pas ce contrat global d'un coup.
//
// A la place, le bureau introduit des "safe points" explicitement choisis.
// Un safe point n'agit que si:
//   - la tache courante est le kernel thread `desktop` ;
//   - IF est actif ;
//   - le BKL est detenu a profondeur EXACTEMENT 1.
//
// Profondeur > 1 signifie qu'une vraie section critique imbriquee est en cours:
// V9 ne la coupe jamais.
//
// `suspend_for_schedule()` / `resume_after_schedule()` sont deja le protocole
// officiel du noyau pour suspendre temporairement un KernelGuard vivant autour
// d'un changement de contexte. V9 reutilise exactement ce contrat.

use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Site {
    Tour = 0,
    Trame = 1,
    TrameDifferee = 2,
    Present = 3,
    PresentRect = 4,
    Rapport = 5,
}

pub const NOMBRE_SITES: usize = 6;
pub const NOMS_SITES: [&str; NOMBRE_SITES] = [
    "tour",
    "trame",
    "trame-differee",
    "present",
    "present-rect",
    "rapport",
];

include!("desktop_bkl/politique.rs");
include!("desktop_bkl/etat.rs");
include!("desktop_bkl/scope.rs");
include!("desktop_bkl/diagnostic.rs");
