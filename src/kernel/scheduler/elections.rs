//! BOUCHAUD_ELECTIONS_RECENTES_V1 -- QUI A TENU LE COEUR PENDANT UN PIC.
//!
//! `HID_LATENCY_SPIKE` disait combien le fil d'entree avait attendu, sur quel
//! coeur et derriere combien de taches pretes (`runqueue=8..35` sur le banc
//! SMP1). Il ne disait pas QUI avait tenu le coeur pendant l'attente : un
//! seul monopole (fil noyau sans point sur, calcul sans appel systeme) ou un
//! tourniquet de taches normales passees devant une interactive.
//!
//! Chaque commutation range ici, par coeur, la tache elue et l'instant de son
//! election (quatre ecritures atomiques, sans verrou ni allocation, dans
//! `switch_to`). Au pic, `ecris` restitue les elections qui recouvrent la
//! fenetre d'attente, dans l'ordre, avec la duree tenue par chacune.

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::arch::x86_64::smp::MAX_CPUS;

const PROFONDEUR: usize = 16;

struct Case {
    tid: AtomicU32,
    pid: AtomicU32,
    noyau: AtomicBool,
    debut_ns: AtomicU64,
    /// Dernier RIP vu par le tic du minuteur avant que la tache ne quitte le
    /// coeur : noyau (adresse basse) ou utilisateur (0x4000...).
    rip_fin: AtomicU64,
    /// Temps utilisateur / noyau de la tache a son election, puis le temps
    /// consomme pendant cette tenue (rempli a sa sortie du coeur).
    user_debut: AtomicU64,
    noyau_debut: AtomicU64,
    user_tenu: AtomicU64,
    noyau_tenu: AtomicU64,
    /// Appel systeme en cours a la sortie du coeur (u64::MAX : aucun).
    appel_fin: AtomicU64,
}

impl Case {
    const fn neuve() -> Self {
        Self {
            tid: AtomicU32::new(0),
            pid: AtomicU32::new(0),
            noyau: AtomicBool::new(false),
            debut_ns: AtomicU64::new(0),
            rip_fin: AtomicU64::new(0),
            user_debut: AtomicU64::new(0),
            noyau_debut: AtomicU64::new(0),
            user_tenu: AtomicU64::new(u64::MAX),
            noyau_tenu: AtomicU64::new(u64::MAX),
            appel_fin: AtomicU64::new(u64::MAX),
        }
    }
}

static HISTORIQUE: [[Case; PROFONDEUR]; MAX_CPUS] =
    [const { [const { Case::neuve() }; PROFONDEUR] }; MAX_CPUS];
static PROCHAINE: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

/// Une election sur `cpu` (appele par `switch_to`, interruptions masquees :
/// un seul ecrivain par coeur).
#[inline]
pub fn note(cpu: usize, tid: u32, pid: u32, noyau: bool, maintenant_ns: u64) {
    note_avec_temps(cpu, (0, 0), u64::MAX, tid, pid, noyau, (0, 0), maintenant_ns);
}

/// `note`, avec le temps cumule (utilisateur, noyau) de la tache SORTANTE
/// (deja compte par `account_slice_end`) et celui de l'ENTRANTE.
#[inline]
pub fn note_avec_temps(
    cpu: usize,
    sortante: (u64, u64),
    appel_sortant: u64,
    tid: u32,
    pid: u32,
    noyau: bool,
    entrante: (u64, u64),
    maintenant_ns: u64,
) {
    if cpu >= MAX_CPUS {
        return;
    }
    let rang = PROCHAINE[cpu].load(Ordering::Relaxed);
    if rang != 0 {
        let (rip, _) = crate::kernel::task::rips_timer(cpu);
        let precedente = &HISTORIQUE[cpu][(rang - 1) % PROFONDEUR];
        precedente.rip_fin.store(rip, Ordering::Relaxed);
        precedente.appel_fin.store(appel_sortant, Ordering::Relaxed);
        if sortante != (0, 0) {
            precedente.user_tenu.store(
                sortante.0.saturating_sub(precedente.user_debut.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
            precedente.noyau_tenu.store(
                sortante.1.saturating_sub(precedente.noyau_debut.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
        }
    }
    let case = &HISTORIQUE[cpu][rang % PROFONDEUR];
    case.rip_fin.store(0, Ordering::Relaxed);
    case.appel_fin.store(u64::MAX, Ordering::Relaxed);
    case.user_debut.store(entrante.0, Ordering::Relaxed);
    case.noyau_debut.store(entrante.1, Ordering::Relaxed);
    case.user_tenu.store(u64::MAX, Ordering::Relaxed);
    case.noyau_tenu.store(u64::MAX, Ordering::Relaxed);
    case.debut_ns.store(0, Ordering::Relaxed);
    case.tid.store(tid, Ordering::Relaxed);
    case.pid.store(pid, Ordering::Relaxed);
    case.noyau.store(noyau, Ordering::Relaxed);
    case.debut_ns.store(maintenant_ns, Ordering::Release);
    PROCHAINE[cpu].store(rang.wrapping_add(1), Ordering::Release);
}

/// Les elections de `cpu` qui recouvrent `[depuis_ns, jusqu_a_ns]`, de la
/// plus ancienne a la plus recente : `pid/tid[n]:duree_us` (`n` = fil noyau),
/// puis la plus longue tenue. Lecture sans verrou : une case reecrite pendant
/// la lecture est sautee (instant nul ou hors fenetre), jamais inventee.
pub fn ecris<W: Write>(sortie: &mut W, cpu: usize, depuis_ns: u64, jusqu_a_ns: u64) {
    if cpu >= MAX_CPUS {
        return;
    }
    let fin = PROCHAINE[cpu].load(Ordering::Acquire);
    let mut lues = [(0u32, 0u32, false, 0u64); PROFONDEUR];
    let mut rips = [0u64; PROFONDEUR];
    let mut appels = [u64::MAX; PROFONDEUR];
    let mut temps = [(u64::MAX, u64::MAX); PROFONDEUR];
    let mut n = 0usize;
    for i in 0..PROFONDEUR.min(fin) {
        let case = &HISTORIQUE[cpu][(fin - 1 - i) % PROFONDEUR];
        let debut = case.debut_ns.load(Ordering::Acquire);
        if debut == 0 {
            continue;
        }
        lues[n] = (
            case.tid.load(Ordering::Relaxed),
            case.pid.load(Ordering::Relaxed),
            case.noyau.load(Ordering::Relaxed),
            debut,
        );
        rips[n] = case.rip_fin.load(Ordering::Relaxed);
        appels[n] = case.appel_fin.load(Ordering::Relaxed);
        temps[n] = (case.user_tenu.load(Ordering::Relaxed), case.noyau_tenu.load(Ordering::Relaxed));
        n += 1;
        if debut <= depuis_ns {
            break;
        }
    }
    // `lues` va de la plus recente a la plus ancienne ; la fin de chacune est
    // le debut de la suivante (ou `jusqu_a_ns` pour la derniere).
    let mut pire = (0u32, 0u32, false, 0u64);
    let _ = write!(sortie, " tenues=");
    for k in (0..n).rev() {
        let (tid, pid, noyau, debut) = lues[k];
        let fin_ns = if k == 0 { jusqu_a_ns } else { lues[k - 1].3 };
        let debut_utile = debut.max(depuis_ns);
        let duree_us = fin_ns.saturating_sub(debut_utile) / 1_000;
        if duree_us > pire.3 {
            pire = (tid, pid, noyau, duree_us);
        }
        let _ = write!(sortie, "{}/{}{}:{}", pid, tid, if noyau { "n" } else { "" }, duree_us);
        if temps[k].0 != u64::MAX && temps[k].1 != u64::MAX {
            let _ = write!(sortie, "(u{}k{})", temps[k].0 / 1_000, temps[k].1 / 1_000);
        }
        if appels[k] < 1024 {
            let _ = write!(sortie, "#{}", appels[k]);
        }
        if rips[k] != 0 {
            let _ = write!(sortie, "@{:#x}", rips[k]);
        }
        if k != 0 {
            let _ = write!(sortie, ",");
        }
    }
    let _ = write!(
        sortie,
        " elections={} pire={}/{}{}:{}us",
        n, pire.1, pire.0, if pire.2 { "n" } else { "" }, pire.3
    );
}
