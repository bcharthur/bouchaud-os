//! Journal de mortalite et relance bornee des fils redemarrables.
// P18_SERVICE_GUARDIAN_V1
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::kernel::sync::SpinLockIrq;
use super::reprise::{Fil, Politique};
static POLITIQUE: SpinLockIrq<Politique> = SpinLockIrq::new(Politique::nouvelle());
static TOUR: AtomicBool = AtomicBool::new(false);
static DERNIER_MS: AtomicU64 = AtomicU64::new(0);
static SORTIES: AtomicU64 = AtomicU64::new(0);
static ANOMALIES: AtomicU64 = AtomicU64::new(0);

fn service(fil: Fil) -> (&'static str, &'static str) {
    match fil {
        Fil::Mesures => ("sys.diag.metrics", "sys.diag"),
        Fil::Audit => ("sys.diag.auditd", "sys.diag"),
        Fil::Telemetrie => ("net.lab.telemetrie", "net.lab"),
        Fil::Brdp => ("net.lab.brdp", "net.lab"),
        Fil::Reception => ("net.config.rx_worker", "net.config"),
        Fil::Lien => ("net.config.link_worker", "net.config"),
    }
}

/// Appele apres avoir rendu le verrou lifecycle, une seule fois par processus.
pub fn mort(pid: u32, ppid: u32, nom: &str, code: i32, force: bool) {
    let maintenant = crate::kernel::timer::monotonic_ms();
    SORTIES.fetch_add(1, Ordering::Relaxed);
    if code != 0 { ANOMALIES.fetch_add(1, Ordering::Relaxed); }
    let cause = if force { "tue_par_gestionnaire" } else if (128..192).contains(&code) {
        "signal_ou_exception_voir_PROCESS_FAULT"
    } else if code == 0 { "sortie_zero" } else { "sortie_non_zero" };
    if code != 0 || force {
        crate::kernel::dmesg::log_fmt(format_args!(
            "PROCESS_DEATH t={} pid={} ppid={} image={} code={} cause={}",
            maintenant, pid, ppid, nom, code, cause));
    }
    if code != 0 || force || Fil::depuis_nom(nom).is_some() {
        crate::kernel::blackbox::processus_mort(pid, ppid, nom, code, cause);
    }
    if let Some(fil) = Fil::depuis_nom(nom) {
        // L'indicateur du lanceur cesse immediatement de mentir. Les fils
        // sont des boucles sans retour normal, une sortie 0 reste anormale.
        match fil {
            Fil::Mesures => crate::gui::services::fil_mesures_termine(),
            Fil::Audit => crate::kernel::lab::auditd::fil_termine(),
            Fil::Telemetrie => crate::net::diag_distant::telemetrie::fil_termine(),
            Fil::Brdp => crate::net::diag_distant::serveur::fil_termine(),
            Fil::Reception | Fil::Lien => crate::net::fil_reseau_termine(nom),
        }
        POLITIQUE.lock().mort(fil, maintenant);
        let (id, parent) = service(fil);
        let _ = super::declare(id, parent, super::Genre::Service);
        super::etat_car(id, super::Etat::Panne, "fil_termine");
        crate::kernel::dmesg::log_fmt(format_args!(
            "SERVICE_DEATH t={} nom={} pid={} code={} relance=programmee", maintenant, nom, pid, code));
    } else if code != 0 || force {
        // Les composants integres (ARP, TLS, GUI, etc.) n'ont pas chacun un
        // processus. Un processus utilisateur arbitraire n'est pas relance
        // sans son parent, ses arguments, son contexte et ses IPC.
        crate::kernel::dmesg::log_fmt(format_args!(
            "PROCESS_RECOVERY t={} pid={} policy=owner_or_no_restart", maintenant, pid));
    }
}

pub fn compteurs() -> (u64, u64, u64, u64, u64, u64) {
    let (morts, relances, echecs, refuses) = POLITIQUE.lock().compteurs();
    (SORTIES.load(Ordering::Relaxed), ANOMALIES.load(Ordering::Relaxed),
     morts, relances, echecs, refuses)
}

/// Deux appelants independants: mesures (100 ms) et auditeur (1 Hz).
/// Pas de balayage PID global, pas d'allocation, pas de travail par paquet.
pub fn tour() {
    let maintenant = crate::kernel::timer::monotonic_ms();
    if maintenant.saturating_sub(DERNIER_MS.load(Ordering::Relaxed)) < 1_000 { return; }
    if TOUR.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() { return; }
    DERNIER_MS.store(maintenant, Ordering::Relaxed);
    // Toujours hors verrou: lancer un processus acquiert ses propres verrous.
    let candidat = { POLITIQUE.lock().reserve(maintenant) };
    if let Some((fil, essai)) = candidat {
        let ok = match fil {
            Fil::Mesures => crate::gui::services::demarre_fil_mesures_processus(),
            Fil::Audit => crate::kernel::lab::auditd::demarre(),
            Fil::Telemetrie => crate::net::diag_distant::telemetrie::demarre(),
            Fil::Brdp => crate::net::diag_distant::serveur::demarre(),
            Fil::Reception | Fil::Lien => {
                let _ = crate::net::demarre_le_veilleur_de_lien();
                crate::net::fil_reseau_actif(fil.nom())
            },
        };
        POLITIQUE.lock().resultat(fil, maintenant, ok);
        let (id, parent) = service(fil);
        let _ = super::declare(id, parent, super::Genre::Service);
        super::etat_car(id, if ok { super::Etat::Actif } else { super::Etat::Panne },
                        if ok { "relance_ok" } else { "lancement_refuse" });
        crate::kernel::dmesg::log_fmt(format_args!(
            "SERVICE_RECOVERY t={} nom={} essai={} ok={} budget={}",
            maintenant, fil.nom(), essai, ok, super::reprise::MAX_ESSAIS));
    }
    TOUR.store(false, Ordering::Release);
}
