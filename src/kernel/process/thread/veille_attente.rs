// ---------------------------------------------------------------------------
// BOUCHAUD_VEILLE_ATTENTE_VIVE_V1 : une tache prete qui attend, vue PENDANT
// qu'elle attend, et ce qui occupe son coeur
// ---------------------------------------------------------------------------
//
// L'observatoire de latence (`scheduler::latency`) mesure l'attente pret->elu
// A L'ELECTION : apres coup, sans dire ce qui occupait le coeur pendant ce
// temps. La ligne de base Scheduler NG a mesure ainsi 5,5 s d'attente sur le
// coeur zero (SMP4) -- une tache interactive et un calcul elus a la meme
// milliseconde -- sans pouvoir nommer le coupable : la sonde de blocage ne
// tourne que sur l'IRQ du coeur zero, celui-la meme qui etait pris.
//
// Cette veille tourne sur TOUS les coeurs (tic du BSP, IPI de quantum des
// AP) ; un seul passe a la fois, au plus toutes les 20 ms (CAS sur
// l'echeance). Elle releve chaque tache `Ready`, hors de tout coeur, prete
// depuis plus de 200 ms, avec l'etat du coeur de sa file : la tache qui
// l'occupe, noyau ou non, l'appel systeme en cours, le site noyau, l'idle.
// Un rapport par episode (la tache et l'instant ou elle est devenue prete),
// trente-deux au plus par demarrage ; le compte et le pire sont dans
// `smpstat`.
//
// Lecture seule : aucun etat d'ordonnancement n'est modifie.

const VEILLE_PERIODE_NS: u64 = 20_000_000;
const SEUIL_ATTENTE_VIVE_NS: u64 = 50_000_000;
const VEILLE_RAPPORTS_MAX: u32 = 32;
const VEILLE_EPISODES_RETENUS: usize = 16;

static VEILLE_PROCHAINE_NS: AtomicU64 = AtomicU64::new(0);
static VEILLE_EPISODES: AtomicU64 = AtomicU64::new(0);
static VEILLE_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static VEILLE_RAPPORTS: AtomicU32 = AtomicU32::new(0);
/// (tid << 40) ^ instant de mise en file : les episodes deja comptes.
static VEILLE_DEJA_VUS: [AtomicU64; VEILLE_EPISODES_RETENUS] =
    [const { AtomicU64::new(0) }; VEILLE_EPISODES_RETENUS];
static VEILLE_CURSEUR: AtomicUsize = AtomicUsize::new(0);

fn veille_episode_nouveau(cle: u64) -> bool {
    if VEILLE_DEJA_VUS.iter().any(|vu| vu.load(Ordering::Relaxed) == cle) {
        return false;
    }
    let i = VEILLE_CURSEUR.fetch_add(1, Ordering::Relaxed) % VEILLE_EPISODES_RETENUS;
    VEILLE_DEJA_VUS[i].store(cle, Ordering::Relaxed);
    true
}

/// A appeler depuis le tic de chaque coeur. Rend la main en une lecture
/// atomique 49 fois sur 50.
pub fn veille_attentes_vives() {
    let maintenant = crate::kernel::timer::monotonic_ns();
    let prochaine = VEILLE_PROCHAINE_NS.load(Ordering::Relaxed);
    if maintenant < prochaine {
        return;
    }
    if VEILLE_PROCHAINE_NS
        .compare_exchange(prochaine, maintenant + VEILLE_PERIODE_NS, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let liste = tasks();
    for index in 0..liste.len() {
        let tache = &liste[index];
        if tache.state != TaskState::Ready || tache.on_cpu >= 0 {
            continue;
        }
        let depuis = tache.ready_since_ns.charge();
        if depuis == 0 || maintenant <= depuis {
            continue;
        }
        let attente = maintenant - depuis;
        if attente < SEUIL_ATTENTE_VIVE_NS {
            continue;
        }
        VEILLE_PIRE_NS.fetch_max(attente, Ordering::Relaxed);
        if !veille_episode_nouveau(((tache.tid as u64) << 40) ^ depuis) {
            continue;
        }
        VEILLE_EPISODES.fetch_add(1, Ordering::Relaxed);
        if VEILLE_RAPPORTS.fetch_add(1, Ordering::Relaxed) >= VEILLE_RAPPORTS_MAX {
            continue;
        }
        let cpu = (tache.runq_cpu.charge() as usize).min(MAX_CPUS - 1);
        let courant = CURRENT[cpu].load(Ordering::Acquire);
        let (cur_tid, cur_pid, cur_noyau) = if courant != NO_TASK && courant < liste.len() {
            let c = &liste[courant];
            (c.tid, c.process.pid, c.noyau as u8)
        } else {
            (0, 0, 0)
        };
        let (rip, rip_noyau) = rips_timer(cpu);
        crate::serial_println!(
            "[SCHED-NG-ATTENTE-VIVE] tid={} pid={} classe={} cpu={} attente_ms={} file={} \
cur_tid={} cur_pid={} cur_noyau={} idle={} syscall={} site={} rip={:#x} rip_noyau={:#x}",
            tache.tid,
            tache.process.pid,
            if tache.priorite == Priorite::Interactive { "interactive" } else { "normale" },
            cpu,
            attente / 1_000_000,
            ready_count_cpu(cpu),
            cur_tid,
            cur_pid,
            cur_noyau,
            crate::arch::x86_64::cpu::is_idle(cpu) as u8,
            STALL_SYSCALL_NR[cpu].load(Ordering::Relaxed),
            STALL_KERNEL_SITE[cpu].load(Ordering::Relaxed),
            rip,
            rip_noyau,
        );
    }
}

/// episodes, pire attente vive (ns), rapports emis.
pub fn compteurs_veille() -> (u64, u64, u32) {
    (
        VEILLE_EPISODES.load(Ordering::Relaxed),
        VEILLE_PIRE_NS.load(Ordering::Relaxed),
        VEILLE_RAPPORTS.load(Ordering::Relaxed).min(VEILLE_RAPPORTS_MAX),
    )
}
