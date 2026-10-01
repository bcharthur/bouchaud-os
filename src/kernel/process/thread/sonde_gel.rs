// ---------------------------------------------------------------------------
// BOUCHAUD_SONDE_GEL_V1 : un coeur qui ne recoit plus son tic, vu des DEUX
// cotes du trou
// ---------------------------------------------------------------------------
//
// scheduler-ng-banc SMP4/SMP8 : dans deux demarrages sur dix, TOUS les coeurs
// s'arretent ensemble deux a quatre secondes et demie (`PERF_FORK reste_us`
// ~2 s, `HID_LATENCY_SPIKE delta_us` ~2 s, attentes vives simultanees). Les
// compteurs existants (verrous tournants, shootdowns TLB) restent a zero --
// ce qui ne prouve pas que leurs sous-systemes soient innocents : ils ne
// mesurent que ce qu'ils instrumentent.
//
// Cette sonde ne suppose rien. A chaque tic local (IRQ0 sur le coeur zero, tic
// de quantum sur chaque coeur), elle compare l'horloge monotone (TSC) a celle
// du tic precedent du MEME coeur. Un ecart superieur a `GEL_SEUIL_NS` est un
// episode : le coeur n'a pris aucun tic pendant ce temps. L'episode garde ce
// que le coeur faisait au dernier tic AVANT le trou et au premier APRES --
// tache, RIP interrompu, mode, site noyau, appel systeme, repos -- et le
// nombre de tics PIT livres au coeur zero pendant le trou.
//
// Lecture : des episodes simultanes sur tous les coeurs, avec des RIP
// d'arrivee quelconques, designent un arret EXTERIEUR a l'invite (vCPU non
// executes, emulateur bloque) ; un seul coeur, ou un RIP d'arrivee juste apres
// une section aux interruptions masquees, designe l'invite. La sonde hote
// (fils de QEMU echantillonnes) tranche entre « vCPU qui tournent » et « vCPU
// qui attendent ».
//
// Rien n'est imprime depuis l'interruption : l'episode est range en RAM,
// `smpstat` le publie depuis une tache.

const GEL_SEUIL_NS: u64 = 100_000_000;
const GEL_EPISODES: usize = 32;

/// Dernier tic vu par chaque coeur.
static GEL_DERNIER_NS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static GEL_DERNIER_PIT: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static GEL_DERNIER_RIP: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
/// tid << 16 | site << 8 | drapeaux (bit 0 : espace utilisateur, bit 1 : repos,
/// bit 2 : fil noyau, bits 4-5 : source du tic).
static GEL_DERNIER_ETAT: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

static GEL_VUS: AtomicU64 = AtomicU64::new(0);
static GEL_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static GEL_CURSEUR: AtomicUsize = AtomicUsize::new(0);

/// Un episode : dix mots. `EP_PRET` passe a 1 quand tous sont ecrits.
const EP_MOTS: usize = 10;
static GEL_EPISODE: [[AtomicU64; EP_MOTS]; GEL_EPISODES] =
    [const { [const { AtomicU64::new(0) }; EP_MOTS] }; GEL_EPISODES];
static EP_PRET: [AtomicU32; GEL_EPISODES] = [const { AtomicU32::new(0) }; GEL_EPISODES];

/// Source du tic : IRQ0 (coeur zero) ou tic de quantum.
pub const GEL_SOURCE_PIT: u64 = 1;
pub const GEL_SOURCE_QUANTUM: u64 = 2;

fn gel_etat_courant(cpu: usize, depuis_utilisateur: bool, source: u64) -> u64 {
    let liste = tasks();
    let courant = CURRENT[cpu].load(Ordering::Acquire);
    let (tid, noyau) = if courant != NO_TASK && courant < liste.len() {
        (liste[courant].tid as u64, liste[courant].noyau)
    } else {
        (0, false)
    };
    let site = (STALL_KERNEL_SITE[cpu].load(Ordering::Relaxed) as u64) & 0xff;
    (tid << 16)
        | (site << 8)
        | depuis_utilisateur as u64
        | (crate::arch::x86_64::cpu::is_idle(cpu) as u64) << 1
        | (noyau as u64) << 2
        | (source & 3) << 4
}

/// A appeler au debut de chaque tic local, depuis l'interruption.
pub fn sonde_gel_tic(rip: u64, depuis_utilisateur: bool, source: u64) {
    let cpu = local_cpu();
    let maintenant = crate::kernel::timer::monotonic_ns();
    let pit = crate::kernel::timer::ticks();
    let etat = gel_etat_courant(cpu, depuis_utilisateur, source);
    let avant_ns = GEL_DERNIER_NS[cpu].swap(maintenant, Ordering::Relaxed);
    let avant_pit = GEL_DERNIER_PIT[cpu].swap(pit, Ordering::Relaxed);
    let avant_rip = GEL_DERNIER_RIP[cpu].swap(rip, Ordering::Relaxed);
    let avant_etat = GEL_DERNIER_ETAT[cpu].swap(etat, Ordering::Relaxed);
    if avant_ns == 0 || maintenant <= avant_ns {
        return;
    }
    let trou = maintenant - avant_ns;
    if trou < GEL_SEUIL_NS {
        return;
    }
    GEL_VUS.fetch_add(1, Ordering::Relaxed);
    GEL_PIRE_NS.fetch_max(trou, Ordering::Relaxed);
    let n = GEL_CURSEUR.fetch_add(1, Ordering::Relaxed);
    if n >= GEL_EPISODES {
        return;
    }
    let syscall = STALL_SYSCALL_NR[cpu].load(Ordering::Relaxed);
    let mots = [
        cpu as u64,
        avant_ns,
        maintenant,
        pit.saturating_sub(avant_pit),
        avant_rip,
        avant_etat,
        rip,
        etat,
        syscall,
        crate::kernel::timer::ticks(),
    ];
    for (i, m) in mots.iter().enumerate() {
        GEL_EPISODE[n][i].store(*m, Ordering::Relaxed);
    }
    EP_PRET[n].store(1, Ordering::Release);
}

fn gel_decrit(etat: u64) -> (u64, u64, &'static str, u8, u8) {
    let mode = if etat & 1 != 0 {
        "user"
    } else if etat & 4 != 0 {
        "fil-noyau"
    } else {
        "noyau"
    };
    (etat >> 16, (etat >> 8) & 0xff, mode, ((etat >> 1) & 1) as u8, ((etat >> 4) & 3) as u8)
}

/// Publie les episodes. Depuis une tache (`smpstat`), jamais depuis une IRQ.
pub fn publie_sonde_gel() {
    let vus = GEL_VUS.load(Ordering::Relaxed);
    let (ecritures, longues, pire) = compteurs_ecriture_registre();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[REGISTRE-ECRITURE] sections={} plus_de_20ms={} pire_ms={} destruction_recyclee_pire_ms={}",
        ecritures, longues, pire / 1_000_000, destruction_recyclee_pire_ns() / 1_000_000,
    ));
    let (commutes, commute_tid, attente, lecteurs_max) = compteurs_gardes_registre();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[REGISTRE-GARDES] gardes_commutes={} dernier_tid={} attente_lecteurs_pire_ms={} lecteurs_max_vu={}",
        commutes, commute_tid, attente / 1_000_000, lecteurs_max,
    ));
    crate::kernel::dmesg::log_fmt(format_args!(
        "[SONDE-GEL-RESUME] maintenant_ms={} seuil_ms={} episodes={} pire_ms={} retenus={}",
        crate::kernel::timer::monotonic_ns() / 1_000_000,
        GEL_SEUIL_NS / 1_000_000,
        vus,
        GEL_PIRE_NS.load(Ordering::Relaxed) / 1_000_000,
        (vus as usize).min(GEL_EPISODES),
    ));
    for n in 0..GEL_EPISODES {
        if EP_PRET[n].load(Ordering::Acquire) == 0 {
            continue;
        }
        let m: [u64; EP_MOTS] = core::array::from_fn(|i| GEL_EPISODE[n][i].load(Ordering::Relaxed));
        let (tid_a, site_a, mode_a, repos_a, src_a) = gel_decrit(m[5]);
        let (tid_p, site_p, mode_p, repos_p, src_p) = gel_decrit(m[7]);
        crate::kernel::dmesg::log_fmt(format_args!(
            "[SONDE-GEL] n={} cpu={} debut_ms={} trou_ms={} pit_livres={} \
avant: tid={} mode={} repos={} site={} src={} rip={:#x} \
apres: tid={} mode={} repos={} site={} src={} rip={:#x} syscall={}",
            n,
            m[0],
            m[1] / 1_000_000,
            (m[2] - m[1]) / 1_000_000,
            m[3],
            tid_a, mode_a, repos_a, site_a, src_a, m[4],
            tid_p, mode_p, repos_p, site_p, src_p, m[6],
            Absent(m[8]),
        ));
    }
}
