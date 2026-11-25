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

static OOM_VICTIME_PID: AtomicU32 = AtomicU32::new(0);
static OOM_VICTIME_TUEE_NS: AtomicU64 = AtomicU64::new(0);
static OOM_VICTIMES: AtomicU64 = AtomicU64::new(0);
static OOM_ABANDONS: AtomicU64 = AtomicU64::new(0);
static OOM_ATTENTES: AtomicU64 = AtomicU64::new(0);
/// BOUCHAUD_OOM_VICTIME_RENDUE_V1 : le dernier pid dont l'espace a ete rendu
/// a la mort. Une victime devient zombie AVANT de rendre sa memoire (fermer
/// ses descripteurs, ses handles, prevenir le gardien, puis
/// `rend_espace_a_la_mort`) : la juger « morte » sur `zombie` faisait
/// abandonner le fautif dans cette fenetre, sa memoire encore tenue
/// (memoire-mort-probe oom sous charge : OOM_ABANDON 11 ms apres la
/// victime, le pere tue par SIGSEGV).
static OOM_ESPACE_RENDU_PID: AtomicU32 = AtomicU32::new(0);

/// Appele par la sortie du dernier fil, une fois l'espace rendu (ou mis en
/// quarantaine : `vide_quarantaine` le rendra des que les coeurs l'auront
/// quitte).
pub(crate) fn note_espace_rendu(pid: u32) {
    OOM_ESPACE_RENDU_PID.store(pid, Ordering::Release);
}

/// La victime `pid` tient-elle encore sa memoire ? Oui tant qu'elle existe et
/// que sa sortie ne l'a pas rendue. Un processus recolte n'existe plus : son
/// `Mm` est parti avec lui.
fn victime_tient_sa_memoire(pid: u32) -> bool {
    OOM_ESPACE_RENDU_PID.load(Ordering::Acquire) != pid
        && processes().iter().any(|p| p.pid == pid)
}

/// Pages que la reclamation tente de rendre avant de designer une victime.
const OOM_RECLAMATION_PAGES: usize = 512;
/// Pas d'attente de la victime, en ticks (une milliseconde chacun).
const OOM_PAS_ATTENTE_TICKS: u64 = 10;

pub(crate) fn sur_faute_sans_memoire(fautif: u32) -> bool {
    use crate::kernel::oom::{self, Action, Candidat, Verdict, VictimeEnCours};

    // Une tache condamnee, ou la victime designee elle-meme, n'attend pas :
    // sa propre mort est ce qui rendra la memoire.
    if condamnee_courante() || OOM_VICTIME_PID.load(Ordering::Acquire) == fautif {
        return false;
    }
    // Un espace en quarantaine dont les coeurs sont partis se rend ici.
    let recuperees = crate::kernel::memory_pressure::reclaim_now(OOM_RECLAMATION_PAGES)
        + vide_quarantaine();
    let victime = match OOM_VICTIME_PID.load(Ordering::Acquire) {
        0 => None,
        pid => Some(VictimeEnCours {
            pid,
            tuee_a_ns: OOM_VICTIME_TUEE_NS.load(Ordering::Acquire),
            vivante: victime_tient_sa_memoire(pid),
        }),
    };
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
                    OOM_VICTIME_TUEE_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Release);
                    OOM_VICTIME_PID.store(pid, Ordering::Release);
                    OOM_VICTIMES.fetch_add(1, Ordering::Relaxed);
                    let nom = match process_by_pid(pid) {
                        Some(victime) => {
                            let nom = victime.metadata.lock().name.clone();
                            victime.signals.lock().raise(crate::kernel::signal::SIGKILL);
                            nom
                        }
                        None => String::new(),
                    };
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
