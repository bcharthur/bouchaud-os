// BOUCHAUD_OOM_CONTROLE_V1 -- la colle noyau de `kernel::oom` (politique pure).
//
// Appelee par une faute de page anonyme dont l'allocation a echoue, AUCUN
// verrou tenu. Rend `true` s'il faut refaire la faute, `false` si elle doit
// echouer (le fautif meurt, comme avant ce lot).
//
// La victime meurt par SIGKILL, donc par `exit_group_par_signal` et la
// sortie de son dernier fil : sa memoire revient A SA MORT
// (BOUCHAUD_ESPACE_RENDU_A_LA_MORT_V1), sans attendre que son pere la
// recolte. C'est ce qui rend l'attente du fautif bornee et utile.

#[derive(Clone)]
struct VictimeOom {
    pid: u32,
    tuee_a_ns: u64,
    processus: alloc::sync::Weak<Process>,
}

static OOM_VICTIME: SpinLock<Option<VictimeOom>> = SpinLock::new(None);
static OOM_VICTIMES: AtomicU64 = AtomicU64::new(0);
static OOM_ABANDONS: AtomicU64 = AtomicU64::new(0);
static OOM_ATTENTES: AtomicU64 = AtomicU64::new(0);

/// Pages que la reclamation tente de rendre avant de designer une victime.
const OOM_RECLAMATION_PAGES: usize = 512;
/// Pas d'attente de la victime, en ticks (une milliseconde chacun).
const OOM_PAS_ATTENTE_TICKS: u64 = 10;

pub(crate) fn sur_faute_sans_memoire(fautif: u32) -> bool {
    use crate::kernel::oom::{self, Action, Candidat, Verdict, VictimeEnCours};

    // Une tache condamnee, ou la victime designee elle-meme, n'attend pas :
    // sa propre mort est ce qui rendra la memoire.
    if condamnee_courante() || OOM_VICTIME.lock().as_ref().is_some_and(|v| v.pid == fautif) {
        return false;
    }
    // Un espace en quarantaine dont les coeurs sont partis se rend ici.
    let recuperees = crate::kernel::memory_pressure::reclaim_now(OOM_RECLAMATION_PAGES)
        + vide_quarantaine();
    // La quarantaine garde le Process vivant meme apres wait4, y compris
    // pendant la barriere. Le rendu est publie apres la destruction du Mm.
    let en_cours = OOM_VICTIME.lock().clone();
    let victime = en_cours.map(|v| VictimeEnCours {
        pid: v.pid,
        tuee_a_ns: v.tuee_a_ns,
        vivante: v.processus.upgrade()
            .is_some_and(|p| !p.espace_rendu.load(Ordering::Acquire)),
    });
    match oom::action(recuperees, victime, crate::kernel::timer::monotonic_ns()) {
        Action::Reessaie => true,
        Action::AttendsVictime => {
            OOM_ATTENTES.fetch_add(1, Ordering::Relaxed);
            sleep_ticks_noyau(OOM_PAS_ATTENTE_TICKS);
            true
        }
        Action::ChoisisVictime => {
            // Instantane du registre, PUIS lecture de chaque `Mm` : jamais un
            // `Mm` sous le verrou du registre (cycle registre/IRQ/Mm du gel
            // #283, PASSATION_2026-11-17).
            let candidats: Vec<Candidat> = processes()
                .iter()
                .map(|p| Candidat {
                    pid: p.pid,
                    pages: p.mm.lock().space.mapped_pages() as u64,
                    zombie: p.lifecycle.lock().zombie,
                    emprunteur: p.emprunte_mm(),
                    protege: false,
                })
                .collect();
            crate::kernel::memory_pressure::note_oom();
            match oom::verdict(fautif, &candidats) {
                Verdict::Tue(pid) => {
                    let pages = candidats.iter().find(|c| c.pid == pid).map(|c| c.pages).unwrap_or(0);
                    let Some(victime) = process_by_pid(pid) else {
                        return true;
                    };
                    *OOM_VICTIME.lock() = Some(VictimeOom {
                        pid,
                        tuee_a_ns: crate::kernel::timer::monotonic_ns(),
                        processus: Arc::downgrade(&victime),
                    });
                    OOM_VICTIMES.fetch_add(1, Ordering::Relaxed);
                    let nom = victime.metadata.lock().name.clone();
                    victime.signals.lock().raise(crate::kernel::signal::SIGKILL);
                    drop(victime);
                    reveille_pour_signal(pid);
                    crate::kernel::dmesg::log_fmt(format_args!(
                        "OOM_VICTIME t={} pid={} image={} kio={} fautif={}",
                        crate::kernel::timer::monotonic_ms(),
                        pid,
                        nom,
                        pages.saturating_mul(4),
                        fautif,
                    ));
                    sleep_ticks_noyau(OOM_PAS_ATTENTE_TICKS);
                    true
                }
                Verdict::Abandonne => {
                    OOM_ABANDONS.fetch_add(1, Ordering::Relaxed);
                    crate::kernel::dmesg::log_fmt(format_args!(
                        "OOM_ABANDON t={} fautif={} raison=le_fautif_est_le_plus_gros_ou_personne",
                        crate::kernel::timer::monotonic_ms(),
                        fautif,
                    ));
                    false
                }
            }
        }
    }
}

/// (victimes tuees, abandons, attentes)
pub fn oom_stats() -> (u64, u64, u64) {
    (
        OOM_VICTIMES.load(Ordering::Relaxed),
        OOM_ABANDONS.load(Ordering::Relaxed),
        OOM_ATTENTES.load(Ordering::Relaxed),
    )
}
