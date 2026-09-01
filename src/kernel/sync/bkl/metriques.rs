// BOUCHAUD_SMP4_OWNER_PROVENANCE_PROBE_V3
// Provenance du Big Kernel Lock. Aucun log n'est emis dans le hot path :
// on ne fait que mettre a jour des atomiques. Le PIT imprime un snapshot
// une fois par seconde, ce qui limite fortement l'effet Heisenberg.
// kind acquire : 1=enter, 2=try_enter, 3=resume_after_schedule.
// kind release : 1=Drop/release_one, 2=suspend_for_schedule.
static PROBE_ACQUIRE_SEQ: AtomicU64 = AtomicU64::new(0);
static PROBE_RELEASE_SEQ: AtomicU64 = AtomicU64::new(0);
static PROBE_REENTER_SEQ: AtomicU64 = AtomicU64::new(0);
static PROBE_OWNER_GEN: AtomicU64 = AtomicU64::new(0);
static PROBE_OWNER_SINCE: AtomicU64 = AtomicU64::new(0);
static PROBE_OWNER_KIND: AtomicU32 = AtomicU32::new(0);
static PROBE_OWNER_TASK: AtomicUsize = AtomicUsize::new(usize::MAX);
static PROBE_OWNER_SYSCALL: AtomicU64 = AtomicU64::new(u64::MAX);
static PROBE_OWNER_SYSCALL_PHASE: AtomicU32 = AtomicU32::new(0);
static PROBE_OWNER_SITE: AtomicU32 = AtomicU32::new(0);
static PROBE_OWNER_AUX: AtomicU64 = AtomicU64::new(0);
static PROBE_LAST_RELEASE_TICK: AtomicU64 = AtomicU64::new(0);
static PROBE_LAST_RELEASE_CPU: AtomicUsize = AtomicUsize::new(usize::MAX);
static PROBE_LAST_RELEASE_KIND: AtomicU32 = AtomicU32::new(0);
static PROBE_LAST_RELEASE_GEN: AtomicU64 = AtomicU64::new(0);
static TOTAL_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static TOTAL_HOLD_NS: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_P2_BKL_PAR_APPEL_V1
//
// POURQUOI UN TOTAL NE SUFFIT PAS
// -------------------------------
// « BKL detenu 99,96 % de la fenetre » dit qu'il y a un probleme, pas OU. Le
// maximum et sa provenance donnent UN coupable — celui d'une seule tenue. Ils
// ne disent pas si ce coupable est isole ou systematique, ni ce que les autres
// appels coutent a cote.
//
// Ce tableau attribue chaque nanoseconde de detention et d'attente a l'appel
// systeme qui la provoque. C'est ce qui permet d'affirmer un avant/apres :
// « madvise tenait 5,2 s par fenetre, il en tient 12 ms » est une mesure ;
// « c'est plus rapide » n'en est pas une.
//
// Rien n'est formate sur le chemin chaud : ce sont des `fetch_add` relaxes sur
// un index deja connu. Le texte est fabrique au releve, une fois par seconde.
const APPELS_SUIVIS: usize = 512;
static HOLD_PAR_APPEL: [AtomicU64; APPELS_SUIVIS] =
    [const { AtomicU64::new(0) }; APPELS_SUIVIS];
static ATTENTE_PAR_APPEL: [AtomicU64; APPELS_SUIVIS] =
    [const { AtomicU64::new(0) }; APPELS_SUIVIS];
static ACQ_PAR_APPEL: [AtomicU64; APPELS_SUIVIS] =
    [const { AtomicU64::new(0) }; APPELS_SUIVIS];
static MAX_HOLD_PAR_APPEL: [AtomicU64; APPELS_SUIVIS] =
    [const { AtomicU64::new(0) }; APPELS_SUIVIS];

/// Index du seau d'un numero d'appel systeme.
///
/// `u64::MAX` designe « hors appel systeme » — une IRQ, un fil noyau — et tombe
/// dans le dernier seau, qui est donc celui du noyau lui-meme.
#[inline]
fn seau(syscall_nr: u64) -> usize {
    if syscall_nr == u64::MAX {
        APPELS_SUIVIS - 1
    } else {
        (syscall_nr as usize).min(APPELS_SUIVIS - 2)
    }
}

/// Seau « hors appel systeme ».
pub const SEAU_NOYAU: usize = APPELS_SUIVIS - 1;

/// `(hold_ns, attente_ns, acquisitions, max_hold_ns)` pour un appel systeme.
pub fn stats_par_appel(syscall_nr: u64) -> (u64, u64, u64, u64) {
    let index = seau(syscall_nr);
    (
        HOLD_PAR_APPEL[index].load(Ordering::Relaxed),
        ATTENTE_PAR_APPEL[index].load(Ordering::Relaxed),
        ACQ_PAR_APPEL[index].load(Ordering::Relaxed),
        MAX_HOLD_PAR_APPEL[index].load(Ordering::Relaxed),
    )
}

/// Nombre de seaux, pour un appelant qui veut les parcourir tous.
pub fn nombre_de_seaux() -> usize {
    APPELS_SUIVIS
}

/// Les mesures du seau `index`, sans passer par un numero d'appel.
pub fn stats_du_seau(index: usize) -> (u64, u64, u64, u64) {
    let index = index.min(APPELS_SUIVIS - 1);
    (
        HOLD_PAR_APPEL[index].load(Ordering::Relaxed),
        ATTENTE_PAR_APPEL[index].load(Ordering::Relaxed),
        ACQ_PAR_APPEL[index].load(Ordering::Relaxed),
        MAX_HOLD_PAR_APPEL[index].load(Ordering::Relaxed),
    )
}

#[inline]
fn note_attente(syscall_nr: u64, attente: u64, origine: u32, cpu: usize) {
    let index = seau(syscall_nr);
    TOTAL_WAIT_NS.fetch_add(attente, Ordering::Relaxed);
    ATTENTE_PAR_APPEL[index].fetch_add(attente, Ordering::Relaxed);
    COMPTES.note_attente(attente, origine, cpu, index);
}
/// Plus longue tenue continue du verrou, et ou elle s'est produite.
///
/// Un cumul ne dit rien d'une panne de vivacite : mille tenues d'une
/// microseconde et une tenue de deux secondes donnent la meme somme. C'est le
/// MAXIMUM qui distingue un noyau qui travaille d'un noyau qui gele, et c'est
/// donc lui qu'une non-regression peut affirmer.
static PLUS_LONGUE_TENUE_NS: AtomicU64 = AtomicU64::new(0);
static PLUS_LONGUE_TENUE_SITE: AtomicU32 = AtomicU32::new(0);

// BOUCHAUD_P1_BKL_MAX_HOLD_PROVENANCE_V1
//
// `max_hold_site` valait 0 pour la tenue de 2,94 s -- donc inutilisable.
//
// La raison : le site attribue a une tenue n'est ecrit que par
// `stall_site_set`, et il est remis a zero a chaque acquisition. Un code qui
// tient le verrou SANS jamais marquer de site -- ce que font justement
// `execve`, `poll` et `futex` sur leurs chemins longs -- laisse donc zero.
// Chercher le coupable la ou il ne s'annonce pas ne pouvait pas marcher.
//
// La provenance de l'ACQUISITION, elle, est toujours connue : qui, sur quel
// CPU, dans quel appel systeme, a quelle phase. Elle est deja lue a chaque
// acquisition pour la sonde de blocage ; il suffisait de la garder.
//
// Rien n'est formate ni alloue sur ce chemin : ce sont des entiers, ranges
// dans des atomiques. Le texte est fabrique au releve, une fois par seconde.
static TENUE_CPU: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(usize::MAX) }; MAX_CPUS];
static TENUE_TACHE: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(usize::MAX) }; MAX_CPUS];
static TENUE_SYSCALL: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(u64::MAX) }; MAX_CPUS];
static TENUE_PHASE: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static TENUE_SITE_ACQ: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static TENUE_ORIGINE: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];

static MAX_TENUE_CPU: AtomicUsize = AtomicUsize::new(usize::MAX);
static MAX_TENUE_TACHE: AtomicUsize = AtomicUsize::new(usize::MAX);
static MAX_TENUE_SYSCALL: AtomicU64 = AtomicU64::new(u64::MAX);
static MAX_TENUE_PHASE: AtomicU32 = AtomicU32::new(0);
static MAX_TENUE_SITE_ACQ: AtomicU32 = AtomicU32::new(0);
static MAX_TENUE_ORIGINE: AtomicU32 = AtomicU32::new(0);

// BOUCHAUD_P1_BKL_MAX_MONOTONE_V2
//
// Generation du releve maximum, pour que duree et provenance aillent ENSEMBLE.
//
// Impaire = ecriture en cours, paire = etat publie. C'est un seqlock a un seul
// ecrivain a la fois : la course d'ecriture est resolue par le
// compare-exchange sur la duree, pas par cette generation.
static MAX_TENUE_GEN: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_P1_BKL_MAX_COHERENT_V3
//
// Le CAS sur la duree ne serialise PAS l'ecriture de la provenance :
// plusieurs CPU peuvent successivement battre le maximum puis ecrire leurs
// metadonnees en parallele. Un seqlock n'est correct qu'avec un writer unique.
//
// Ce verrou ne couvre que le chemin RARE "nouveau record". Le chemin normal
// d'une liberation du BKL ne paie qu'un load Relaxed.
static MAX_TENUE_WRITER: AtomicUsize = AtomicUsize::new(0);

/// Publie une tenue si elle bat le record, avec sa provenance.
///
/// # Pourquoi ce n'est pas `if tenue > max { max = tenue }`
///
/// Entre la lecture et l'ecriture, un autre CPU peut publier une tenue PLUS
/// LONGUE que la notre : on l'ecraserait avec la notre, et `max_hold_ns`
/// DIMINUERAIT. Un maximum qui diminue n'est pas un maximum, et c'est
/// exactement le genre de metrique qui fait chercher au mauvais endroit.
///
/// La boucle de compare-exchange rend la mise a jour reellement monotone : on
/// ne remplace que ce qu'on a lu, et on relit si quelqu'un est passe entre-temps.
///
/// # Pourquoi une generation en plus
///
/// Gagner le compare-exchange donne le droit d'ecrire la duree. Ecrire la
/// PROVENANCE prend six stores de plus, pendant lesquels un lecteur pourrait
/// voir la nouvelle duree avec l'ancienne provenance -- un journal qui accuse
/// le mauvais appel systeme. La generation encadre ces six stores : le lecteur
/// qui la voit impaire, ou changer, recommence.
fn publie_si_plus_longue(cpu: usize, tenue: u64) {
    // Fast path : la quasi-totalite des liberations ne battent pas le record.
    if tenue <= PLUS_LONGUE_TENUE_NS.load(Ordering::Relaxed) {
        return;
    }

    // Le CPU vient desormais de l'INTERVALLE, pas de l'appelant : il vaut
    // `AUCUN` si la comptabilite s'est decrochee. Indexer avec serait un
    // depassement de tableau -- on prefere une provenance absente a un panic.
    if cpu >= MAX_CPUS {
        return;
    }

    // Un nouveau record est rare. Serialiser uniquement ce chemin garantit
    // l'invariant fondamental du seqlock : un seul writer a la fois.
    while MAX_TENUE_WRITER
        .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        spin_loop();
    }

    // Un autre CPU a pu publier mieux pendant notre attente.
    if tenue <= PLUS_LONGUE_TENUE_NS.load(Ordering::Relaxed) {
        MAX_TENUE_WRITER.store(0, Ordering::Release);
        return;
    }

    MAX_TENUE_GEN.fetch_add(1, Ordering::AcqRel); // -> impaire
    MAX_TENUE_CPU.store(cpu, Ordering::Relaxed);
    MAX_TENUE_TACHE.store(TENUE_TACHE[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    MAX_TENUE_SYSCALL.store(TENUE_SYSCALL[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    MAX_TENUE_PHASE.store(TENUE_PHASE[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    MAX_TENUE_SITE_ACQ.store(TENUE_SITE_ACQ[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    MAX_TENUE_ORIGINE.store(TENUE_ORIGINE[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    PLUS_LONGUE_TENUE_SITE.store(
        crate::kernel::task::stall_site_de_la_tenue(),
        Ordering::Relaxed,
    );
    // Duree et provenance appartiennent au meme snapshot.
    PLUS_LONGUE_TENUE_NS.store(tenue, Ordering::Relaxed);
    MAX_TENUE_GEN.fetch_add(1, Ordering::AcqRel); // -> paire

    MAX_TENUE_WRITER.store(0, Ordering::Release);
}
static TOTAL_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
/// Acquisitions ventilees par origine : 1 = `enter`, 2 = `try_enter*` (IRQ),
/// 3 = `resume_after_schedule` (reprise d'une pile apres un changement de
/// contexte). Un total seul ne dit pas OU passe le verrou ; ce detail-la, si.
static ACQ_PAR_ORIGINE: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

// BOUCHAUD_P0_BKL_COMPTABILITE_V1
//
// L'INTERVALLE DE TENUE APPARTIENT AU VERROU, PAS A UN CPU
// -------------------------------------------------------
// Ici vivait `ACQUIRED_AT_NS: [AtomicU64; MAX_CPUS]` : un horodatage par CPU.
// Le verrou est pourtant EXCLUSIF -- il n'y a jamais qu'une tenue en cours --
// et la continuation qui le detient peut changer de CPU. Une case laissee non
// nulle par une acquisition dont la liberation n'a pas eu lieu sur le meme CPU
// restait en place, et une liberation SANS RAPPORT la consommait bien plus
// tard : la duree publiee couvrait alors tout ce qui s'etait passe entre les
// deux. C'est ce qui a produit `hold_pct=183 %` -- impossible pour un verrou
// exclusif -- et une pointe de 29 secondes attribuee a `resume_after_schedule`.
//
// `Comptes` tient UN seul intervalle, celui du verrou. La sonde de liberation
// s'executant avant que `OWNER` ne repasse a `FREE`, les intervalles factures
// sont deux a deux disjoints : leur somme est majoree par le temps ecoule, et
// `hold_pct <= 100` devient une propriete de la structure.
// `tools/smp/test_bkl_comptes.rs` en fait un test, et falsifie l'ancien schema.
static COMPTES: Comptes = Comptes::neuf();

#[inline]
fn probe_note_reenter() {
    PROBE_REENTER_SEQ.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn probe_note_acquire(cpu: usize, kind: u32) {
    TOTAL_ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
    crate::kernel::task::stall_site_tenue_reset();
    ACQ_PAR_ORIGINE[(kind as usize).min(3)].fetch_add(1, Ordering::Relaxed);
    // GEN=0 signifie uniquement "transition de metadata". Le token OWNER
    // est deja installe par le CAS ; le PIT sait donc ignorer ce tres court
    // intervalle plutot que de fabriquer un faux snapshot coherent.
    PROBE_OWNER_GEN.store(0, Ordering::Release);
    let generation = PROBE_ACQUIRE_SEQ.fetch_add(1, Ordering::AcqRel) + 1;
    let (task, syscall_nr, syscall_phase, site, aux) =
        crate::kernel::task::stall_probe_local_context();
    PROBE_OWNER_SINCE.store(crate::kernel::timer::ticks(), Ordering::Release);
    PROBE_OWNER_KIND.store(kind, Ordering::Release);
    PROBE_OWNER_TASK.store(task, Ordering::Release);
    PROBE_OWNER_SYSCALL.store(syscall_nr, Ordering::Release);
    PROBE_OWNER_SYSCALL_PHASE.store(syscall_phase, Ordering::Release);
    PROBE_OWNER_SITE.store(site, Ordering::Release);
    PROBE_OWNER_AUX.store(aux, Ordering::Release);
    PROBE_OWNER_GEN.store(generation, Ordering::Release);

    // BOUCHAUD_P1_BKL_MAX_HOLD_PROVENANCE_V1 : la meme lecture, conservee par
    // CPU pour pouvoir l'attribuer a la tenue quand elle se terminera.
    TENUE_CPU[cpu].store(cpu, Ordering::Relaxed);
    TENUE_TACHE[cpu].store(task, Ordering::Relaxed);
    TENUE_SYSCALL[cpu].store(syscall_nr, Ordering::Relaxed);
    TENUE_PHASE[cpu].store(syscall_phase, Ordering::Relaxed);
    TENUE_SITE_ACQ[cpu].store(site, Ordering::Relaxed);
    TENUE_ORIGINE[cpu].store(kind, Ordering::Relaxed);
    ACQ_PAR_APPEL[seau(syscall_nr)].fetch_add(1, Ordering::Relaxed);

    // Le seau est fige ICI : c'est l'acquereur qui tiendra le verrou pendant
    // tout l'intervalle, meme si une IRQ le preempte ensuite.
    COMPTES.ouvre(
        crate::kernel::timer::monotonic_ns(),
        seau(syscall_nr),
        cpu,
        kind,
    );

    // BOUCHAUD_C1_ATTRIBUTION_DOMAINE_V1
    //
    // A QUEL CHEMIN cette prise appartient-elle. Un total ne dit pas lesquelles
    // restent legitimes ; or la sortie du gros verrou se fait chemin par
    // chemin, et un chemin sorti doit ensuite le RESTER. Sans attribution, un
    // appelant qui reprend le verrou dans un sous-systeme deja migre est
    // indiscernable du bruit de fond des chemins non encore traites.
    //
    // Rien de plus qu'un `fetch_add` : on s'execute interruptions masquees, et
    // le journal se fabrique une fois par seconde, ailleurs.
    if let Some(fautif) = crate::kernel::sync::registre_domaines().note_acquisition(cpu) {
        crate::kernel::sync::signale_regression_domaine(fautif);
    }
}

#[inline]
fn probe_note_release(cpu: usize, kind: u32) {
    let now_ns = crate::kernel::timer::monotonic_ns();
    // `ferme` ne rend une duree QUE si un intervalle etait reellement ouvert.
    // Une liberation orpheline incremente un compteur d'anomalie et ne facture
    // rien -- l'ancien code ajoutait `now - 0`, c'est-a-dire tout le temps
    // ecoule depuis le demarrage, a chaque fois.
    if let Some(t) = COMPTES.ferme(now_ns, cpu) {
        TOTAL_HOLD_NS.fetch_add(t.ns, Ordering::Relaxed);
        // BOUCHAUD_P2_BKL_PAR_APPEL_V1 : l'appel systeme attribue est celui
        // note a l'ACQUISITION, pas celui en cours maintenant. Les deux
        // different quand une IRQ preempte, et c'est bien l'acquereur qui a
        // tenu le verrou pendant tout l'intervalle. Le seau voyage avec
        // l'intervalle, et non dans une case indexee par le CPU qui libere :
        // apres une migration, cette case-la decrit une AUTRE tenue.
        let index = t.seau;
        HOLD_PAR_APPEL[index].fetch_add(t.ns, Ordering::Relaxed);
        let mut record = MAX_HOLD_PAR_APPEL[index].load(Ordering::Relaxed);
        while t.ns > record {
            match MAX_HOLD_PAR_APPEL[index].compare_exchange_weak(
                record, t.ns, Ordering::Relaxed, Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observe) => record = observe,
            }
        }
        // La provenance est celle de l'ACQUEREUR : c'est lui qui a tenu.
        publie_si_plus_longue(t.cpu_acquisition, t.ns);
    }
    let generation = PROBE_OWNER_GEN.load(Ordering::Acquire);
    PROBE_RELEASE_SEQ.fetch_add(1, Ordering::AcqRel);
    PROBE_LAST_RELEASE_TICK.store(crate::kernel::timer::ticks(), Ordering::Release);
    PROBE_LAST_RELEASE_CPU.store(cpu, Ordering::Release);
    PROBE_LAST_RELEASE_KIND.store(kind, Ordering::Release);
    PROBE_LAST_RELEASE_GEN.store(generation, Ordering::Release);
}

/// Acquisitions par origine : (`enter`, `try_enter`, `resume_after_schedule`).
pub fn acquisitions_par_origine() -> (u64, u64, u64) {
    (
        ACQ_PAR_ORIGINE[1].load(Ordering::Relaxed),
        ACQ_PAR_ORIGINE[2].load(Ordering::Relaxed),
        ACQ_PAR_ORIGINE[3].load(Ordering::Relaxed),
    )
}

/// Plus longue tenue continue observee, et le site noyau marque a ce
/// moment-la (voir `task::stall_site_set`).
pub fn plus_longue_tenue() -> (u64, u32) {
    loop {
        let debut = MAX_TENUE_GEN.load(Ordering::Acquire);
        if debut % 2 != 0 {
            spin_loop();
            continue;
        }
        let releve = (
            PLUS_LONGUE_TENUE_NS.load(Ordering::Relaxed),
            PLUS_LONGUE_TENUE_SITE.load(Ordering::Relaxed),
        );
        if MAX_TENUE_GEN.load(Ordering::Acquire) == debut {
            return releve;
        }
        spin_loop();
    }
}

/// Les grandeurs de la comptabilite du verrou, chacune avec son unite.
///
/// Elles ne se comparent PAS entre elles, et c'est tout l'interet de les
/// separer : `tenue_ns` est du temps de muraille, majore par la fenetre ;
/// `attente_ns` et `reprise_ns` sont du temps de CPU, qu'un cumul sur quatre
/// coeurs peut legitimement porter au quadruple de la fenetre ; le reste
/// compte des evenements. Les avoir confondues est ce qui a produit un
/// `hold_pct` de 183 % sur un verrou exclusif.
pub struct ComptesBkl {
    /// Temps REELLEMENT proprietaire. Intervalles disjoints par construction :
    /// `tenue_ns <= temps ecoule`, toujours.
    pub tenue_ns: u64,
    /// Temps passe a attendre avant d'acquerir, tous CPU confondus.
    pub attente_ns: u64,
    /// La plus longue attente UNIQUE avant acquisition, et ou elle a eu lieu.
    ///
    /// C'est sur elle que porte le critere « aucune acquisition ne doit
    /// prendre plus de 50 ms » : mille attentes d'une microseconde et une
    /// attente de deux secondes donnent le meme cumul, et seule la seconde est
    /// un figement.
    pub attente_max_ns: u64,
    /// 1 = `enter`, 3 = `resume_after_schedule`.
    pub attente_max_origine: u32,
    pub attente_max_cpu: usize,
    pub attente_max_seau: usize,
    /// La part de cette attente subie par une pile REPRISE apres commutation.
    pub reprise_ns: u64,
    /// La plus longue attente unique dans `resume_after_schedule`.
    pub reprise_max_ns: u64,
    pub spins: u64,
    /// Tours d'attente qui n'ont pas pu se garer, interruptions masquees. Une
    /// part elevee veut dire que le parking ne s'applique PAS au chemin
    /// observe, et qu'il tourne a vide comme avant.
    pub spins_irq_masquees: u64,
    pub parks: u64,
    pub wake_ipis: u64,
    /// Reveils qui n'ont pas abouti a une acquisition. Proche de `wake_ipis`,
    /// c'est un troupeau : on reveille tout le monde pour un seul gagnant.
    pub reveils_sans_acquisition: u64,
    /// Tenues fermees par un autre CPU que celui qui les a ouvertes. Non nul,
    /// c'est la preuve directe qu'une continuation a migre verrou en main --
    /// exactement ce que l'ancienne comptabilite par CPU ne pouvait pas voir.
    pub liberations_migrees: u64,
    /// Les trois anomalies. Non nulles, les durees ci-dessus ne decrivent plus
    /// la machine, et il faut les lire comme telles au lieu d'y croire.
    pub sans_debut: u64,
    pub sur_tenue: u64,
    pub horloge_a_rebours: u64,
    /// CPU proprietaire a l'instant du releve, ou `usize::MAX`.
    pub proprietaire: usize,
}

/// Instantane de la comptabilite. Aucune serialisation : c'est du diagnostic,
/// et un compteur lu une nanoseconde trop tot ne change aucune conclusion.
pub fn comptes() -> ComptesBkl {
    log_health_snapshot();
    let (sans_debut, sur_tenue, horloge_a_rebours) = COMPTES.anomalies();
    let (attente_max_ns, attente_max_origine, attente_max_cpu, attente_max_seau) =
        COMPTES.attente_max();
    ComptesBkl {
        attente_max_ns,
        attente_max_origine,
        attente_max_cpu,
        attente_max_seau,
        tenue_ns: COMPTES.tenue_ns(),
        attente_ns: COMPTES.attente_ns(),
        reprise_ns: COMPTES.reprise_ns(),
        reprise_max_ns: COMPTES.reprise_max_ns(),
        spins: COMPTES.spins(),
        spins_irq_masquees: COMPTES.spins_irq_masquees(),
        parks: COMPTES.parks(),
        wake_ipis: COMPTES.wake_ipis(),
        reveils_sans_acquisition: COMPTES.reveils_sans_acquisition(),
        liberations_migrees: COMPTES.liberations_migrees(),
        sans_debut,
        sur_tenue,
        horloge_a_rebours,
        proprietaire: COMPTES.proprietaire(),
    }
}

/// Parkings subis en attendant que `cpu` rende le verrou.
///
/// Repond a « sur QUI attend-on ». Un coeur qui domine cette ventilation est
/// le detenteur a instrumenter ; une repartition plate est de la contention
/// ordinaire, et se traite autrement.
pub fn parks_sur(cpu: usize) -> u64 {
    COMPTES.parks_sur(cpu)
}

/// IPI de reveil recus par `cpu`. Repond a « QUI paie les reveils ».
pub fn wakes_vers(cpu: usize) -> u64 {
    COMPTES.wakes_vers(cpu)
}

/// Provenance de la plus longue tenue : (cpu, tache, syscall, phase, site
/// d'acquisition, origine de l'acquisition).
///
/// `origine` reprend le codage de `probe_note_acquire` : 1 = `enter`,
/// 2 = `try_enter`, 3 = `resume_after_schedule`.
pub fn provenance_plus_longue_tenue() -> (usize, usize, u64, u32, u32, u32) {
    // Ne jamais rendre un releve incoherent : cette fonction est du diagnostic,
    // une provenance fausse est pire qu'une attente de quelques instructions.
    loop {
        let debut = MAX_TENUE_GEN.load(Ordering::Acquire);
        if debut % 2 != 0 {
            spin_loop();
            continue;
        }
        let releve = (
            MAX_TENUE_CPU.load(Ordering::Relaxed),
            MAX_TENUE_TACHE.load(Ordering::Relaxed),
            MAX_TENUE_SYSCALL.load(Ordering::Relaxed),
            MAX_TENUE_PHASE.load(Ordering::Relaxed),
            MAX_TENUE_SITE_ACQ.load(Ordering::Relaxed),
            MAX_TENUE_ORIGINE.load(Ordering::Relaxed),
        );
        if MAX_TENUE_GEN.load(Ordering::Acquire) == debut {
            return releve;
        }
        spin_loop();
    }
}

/// Etat et cout du parking : (CPU actuellement arretes, parkings, IPI de reveil).
///
/// `parked` est un instantane, les deux autres des cumuls. Ensemble ils disent
/// si le parking sert (parks > 0) et ce qu'il coute (wake_ipis rapporte aux
/// acquisitions). Un `parked` durablement non nul avec des acquisitions qui
/// n'avancent plus serait, lui, la signature d'un reveil perdu.
pub fn park_stats() -> (u32, u64, u64) {
    (
        PARKED.load(Ordering::SeqCst).count_ones(),
        TOTAL_PARKS.load(Ordering::Relaxed),
        TOTAL_WAKE_IPIS.load(Ordering::Relaxed),
    )
}

/// Nombre de continuations actuellement en train de restaurer leur profondeur
/// BKL apres un changement de contexte. Hors transition, cette valeur doit
/// rapidement revenir a zero.
pub fn resume_waiters_count() -> u32 {
    RESUME_WAITERS.load(Ordering::SeqCst).count_ones()
}

pub fn contention_stats() -> (u64, u64, u64) {
    (
        TOTAL_WAIT_NS.load(Ordering::Relaxed),
        TOTAL_HOLD_NS.load(Ordering::Relaxed),
        TOTAL_ACQUISITIONS.load(Ordering::Relaxed),
    )
}

pub fn owner_cpu() -> Option<usize> {
    match OWNER.load(Ordering::Acquire) {
        FREE => None,
        value => Some(value - 1),
    }
}

// BOUCHAUD_SMP4_STALL_PROBE_V1 -- lectures atomiques uniquement, utilisables
// depuis le PIT avant toute tentative de BKL.

#[derive(Clone, Copy)]
pub struct StallBklProvenance {
    pub owner_token: usize,
    pub generation: u64,
    pub coherent: bool,
    pub since_tick: u64,
    pub acquire_seq: u64,
    pub release_seq: u64,
    pub reenter_seq: u64,
    pub acquire_kind: u32,
    pub task: usize,
    pub syscall_nr: u64,
    pub syscall_phase: u32,
    pub site: u32,
    pub aux: u64,
    pub last_release_tick: u64,
    pub last_release_cpu: usize,
    pub last_release_kind: u32,
    pub last_release_gen: u64,
}

pub fn stall_probe_provenance() -> StallBklProvenance {
    for _ in 0..4 {
        let g1 = PROBE_OWNER_GEN.load(Ordering::Acquire);
        let owner = OWNER.load(Ordering::Acquire);
        let snapshot = StallBklProvenance {
            owner_token: owner,
            generation: g1,
            coherent: false,
            since_tick: PROBE_OWNER_SINCE.load(Ordering::Acquire),
            acquire_seq: PROBE_ACQUIRE_SEQ.load(Ordering::Acquire),
            release_seq: PROBE_RELEASE_SEQ.load(Ordering::Acquire),
            reenter_seq: PROBE_REENTER_SEQ.load(Ordering::Acquire),
            acquire_kind: PROBE_OWNER_KIND.load(Ordering::Acquire),
            task: PROBE_OWNER_TASK.load(Ordering::Acquire),
            syscall_nr: PROBE_OWNER_SYSCALL.load(Ordering::Acquire),
            syscall_phase: PROBE_OWNER_SYSCALL_PHASE.load(Ordering::Acquire),
            site: PROBE_OWNER_SITE.load(Ordering::Acquire),
            aux: PROBE_OWNER_AUX.load(Ordering::Acquire),
            last_release_tick: PROBE_LAST_RELEASE_TICK.load(Ordering::Acquire),
            last_release_cpu: PROBE_LAST_RELEASE_CPU.load(Ordering::Acquire),
            last_release_kind: PROBE_LAST_RELEASE_KIND.load(Ordering::Acquire),
            last_release_gen: PROBE_LAST_RELEASE_GEN.load(Ordering::Acquire),
        };
        let g2 = PROBE_OWNER_GEN.load(Ordering::Acquire);
        let owner2 = OWNER.load(Ordering::Acquire);
        if g1 == g2 && owner == owner2 && (owner == FREE || g1 != 0) {
            return StallBklProvenance { coherent: true, ..snapshot };
        }
    }
    StallBklProvenance {
        owner_token: OWNER.load(Ordering::Acquire),
        generation: PROBE_OWNER_GEN.load(Ordering::Acquire),
        coherent: false,
        since_tick: PROBE_OWNER_SINCE.load(Ordering::Acquire),
        acquire_seq: PROBE_ACQUIRE_SEQ.load(Ordering::Acquire),
        release_seq: PROBE_RELEASE_SEQ.load(Ordering::Acquire),
        reenter_seq: PROBE_REENTER_SEQ.load(Ordering::Acquire),
        acquire_kind: PROBE_OWNER_KIND.load(Ordering::Acquire),
        task: PROBE_OWNER_TASK.load(Ordering::Acquire),
        syscall_nr: PROBE_OWNER_SYSCALL.load(Ordering::Acquire),
        syscall_phase: PROBE_OWNER_SYSCALL_PHASE.load(Ordering::Acquire),
        site: PROBE_OWNER_SITE.load(Ordering::Acquire),
        aux: PROBE_OWNER_AUX.load(Ordering::Acquire),
        last_release_tick: PROBE_LAST_RELEASE_TICK.load(Ordering::Acquire),
        last_release_cpu: PROBE_LAST_RELEASE_CPU.load(Ordering::Acquire),
        last_release_kind: PROBE_LAST_RELEASE_KIND.load(Ordering::Acquire),
        last_release_gen: PROBE_LAST_RELEASE_GEN.load(Ordering::Acquire),
    }
}
/// Numero de la derniere acquisition. Deux releves egaux a une seconde
/// d'intervalle veulent dire que PERSONNE n'a pris le verrou entre-temps : avec
/// un proprietaire non nul, c'est la meme tenue qui dure.
pub fn stall_probe_acquire_seq() -> u64 {
    PROBE_ACQUIRE_SEQ.load(Ordering::Acquire)
}

pub fn stall_probe_owner_token() -> usize {
    OWNER.load(Ordering::Acquire)
}

pub fn stall_probe_depth(cpu: usize) -> usize {
    DEPTH[cpu.min(MAX_CPUS - 1)].load(Ordering::Acquire)
}

