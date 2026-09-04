// La table des taches vit desormais dans `registre.rs` : un tableau
// d'emplacements a adresse stable, lisibles sans verrou. `static mut TASKS`
// etait un `Vec` que rien ne protegeait -- c'est le gros verrou, pris par tous
// ses appelants, qui le rendait sur, et c'est precisement ce dont on sort.
/// Tous les processus vivants ou zombies.
static PROCESSES: SpinLock<Vec<Arc<Process>>> = SpinLock::new(Vec::new());


const NO_TASK: usize = usize::MAX;
const MAX_CPUS: usize = smp::MAX_CPUS;
static CURRENT: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(NO_TASK) }; MAX_CPUS];
/// Comptabilite du temps CPU de la tache en cours, tenue PAR CPU.
///
/// # Pourquoi elle ne vit plus sur la `Task`
///
/// `account_kernel_enter`/`account_kernel_exit` encadrent chaque appel systeme.
/// Ils passaient par `current()`, donc par `TASKS`, donc exigeaient le gros
/// verrou -- et un appel systeme LIBERE le prenait alors deux fois, une a
/// l'entree et une a la sortie, la ou un appel non libere le prend une seule
/// fois et le garde. La mesure du lot precedent l'a chiffre : 130 000
/// acquisitions contre 387 000 pour le meme travail, sans gain de debit. Tant
/// que la comptabilite reste la, liberer un appel court ne peut pas payer.
///
/// # Ce que ce bloc contient
///
/// Le strict minimum pour dater les frontieres et cumuler les deltas :
///
///  * `COMPTA_DEBUT_NS` -- instant de la derniere frontiere sur ce CPU ;
///  * `COMPTA_USER_NS` / `COMPTA_NOYAU_NS` -- deltas accumules, pas encore
///    replies dans la `Task` ;
///  * `COMPTA_EN_NOYAU` -- de quel cote du mur on se trouve maintenant.
///
/// Aucune `Task` n'est touchee : ni lecture, ni ecriture, ni reference.
///
/// # Quand cela redevient durable
///
/// Au changement de contexte, sous la transition locale qui garantit la
/// propriete des deux taches : [`account_slice_end`] replie les compteurs dans
/// la tache sortante, [`finalise_task_running`] les rearme pour l'entrante. Les
/// totaux `user_cpu_ns`, `kernel_cpu_ns` et `cpu_ns[cpu]` restent donc EXACTS ;
/// seule leur cadence de mise a jour change.
///
/// Et cela ne se voit nulle part, parce que le seul lecteur, `mesure_processus`,
/// ajoute deja la tranche en cours : `live = now - task.last_account_ns`.
/// `last_account_ns` marquant desormais le debut de la tranche non repliee,
/// `live` couvre exactement ce que les compteurs par CPU retiennent. La somme
/// rendue est identique a l'octet pres.
static COMPTA_DEBUT_NS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static COMPTA_USER_NS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static COMPTA_NOYAU_NS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static COMPTA_EN_NOYAU: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];

/// « Une tache qui etait sur ce CPU vient d'etre marquee zombie. »
///
/// `retire_current_if_zombie` s'execute a la sortie de CHAQUE appel systeme et
/// lisait `current().state`, donc la table des taches, donc le gros verrou --
/// pour une condition qui est fausse presque toujours. Le drapeau rend le
/// chemin commun a une seule lecture atomique ; le gros verrou n'est pris que
/// lorsqu'une retraite est reellement demandee.
///
/// Il est pose par [`marque_zombie`], qui tient le gros verrou et connait le
/// CPU de la tache (`on_cpu`), et efface par [`mark_task_running`] quand une
/// nouvelle tache prend le CPU. Une tache qui tourne ne peut pas changer de CPU
/// sans passer par un changement de contexte, et un zombie n'est jamais
/// reordonnance : le drapeau designe donc bien la tache courante de ce CPU.
static RETRAITE_DEMANDEE: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];

/// Preemptions IRQ refusees parce que ce CPU detenait deja le gros verrou.
///
/// Doit rester a zero : `preempt_now` n'est arme que si l'IRQ a interrompu du
/// ring 3, qui ne peut rien detenir. Le compteur est la pour que ce « doit »
/// soit une mesure et non une croyance -- y compris dans une construction sans
/// assertions de debogage, ou `debug_assert!` ne dit rien. Publie par
/// `[SMP-LOAD]`, donc lisible par `smpstat`.
static PREEMPT_IRQ_BKL_TENU: AtomicU64 = AtomicU64::new(0);

/// Fois ou le domaine CPU-local n'a PAS su repondre, et ou l'appelant a du
/// retomber sur `current_process()`, donc sur le gros verrou.
///
/// Sans ce compteur, « l'identite est servie sans verrou » est une intention.
/// Avec lui, c'est une mesure : si le repli est frequent, le gain annonce
/// n'existe pas, et on le voit dans `smpstat` au lieu de le deduire d'un
/// chronometre.
static IDENTITE_REPLI: AtomicU64 = AtomicU64::new(0);

/// Nombre de replis du domaine CPU-local vers le chemin sous gros verrou.
pub fn identite_repli() -> u64 {
    IDENTITE_REPLI.load(Ordering::Relaxed)
}

/// Nombre de preemptions IRQ refusees faute d'avoir pu prendre le verrou depuis
/// la profondeur zero alors que ce CPU le detenait.
pub fn preempt_irq_bkl_tenu() -> u64 {
    PREEMPT_IRQ_BKL_TENU.load(Ordering::Relaxed)
}

static CURRENT_PROCESS: [SpinLockIrq<Option<Arc<Process>>>; MAX_CPUS] =
    [const { SpinLockIrq::new(None) }; MAX_CPUS];
// BOUCHAUD_GATE0_POST_SWITCH_HANDOFF_V2
/// Tache dont CE CPU est en train d'abandonner la pile.
///
/// Invariant Gate 0 : tant que cette case n'est pas completee depuis la pile
/// entrante, la tache sortante garde `on_cpu >= 0`, `switching_out = true` et
/// n'apparait dans AUCUNE runqueue. La publication n'arrive qu'apres le
/// `mov rsp, rsi` de switch_context, jamais apres le seul `mov [rdi], rsp`.
static SWITCH_PENDING: [AtomicUsize; MAX_CPUS] =
    [const { AtomicUsize::new(NO_TASK) }; MAX_CPUS];
/// Porte locale du changement de contexte.
///
/// Le BKL serialisait aussi, par accident, une tache et l'IRQ qui l'interrompt
/// sur le MEME CPU. Une porte par CPU suffit pour cette propriete : elle est
/// prise avant l'election, reste publiee pendant le changement de pile, puis
/// est rendue par la continuation entrante. Aucun verrou RAII ne traverse
/// `switch_context`.
static TRANSITION_ORDONNANCEUR: [AtomicBool; MAX_CPUS] =
    [const { AtomicBool::new(false) }; MAX_CPUS];
static TRANSITIONS_ORDONNANCEUR: AtomicU64 = AtomicU64::new(0);
static TRANSITIONS_ORDONNANCEUR_REFUSEES: AtomicU64 = AtomicU64::new(0);
static DETACHEMENTS_BKL_LEGACY: AtomicU64 = AtomicU64::new(0);
static NEXT_TID: AtomicU32 = AtomicU32::new(100);
static mut KERNEL_CTX: [Context; MAX_CPUS] = [Context { rsp: 0 }; MAX_CPUS];
static NEED_RESCHED: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];
static CURRENT_IS_KERNEL: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];
static TOURS_INTERACTIFS: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static RUNQ_STEALS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static CPU_MIGRATIONS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static STEAL_ATTEMPTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static STEAL_REJECT_BALANCE: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static STEAL_REJECT_AFFINITY: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
// BOUCHAUD_C2_REFUS_DE_VOL_DISTINCTS_V1
//
// Une tache RETIREE de la file du donneur puis REMISE : elle n'etait plus
// eligible, ou avait deja change de coeur. C'est du travail pur perdu -- le
// retrait, l'examen et la remise -- et ce chemin n'incrementait RIEN. La
// campagne qui a suivi l'abaissement du seuil montre 22 retraits pour 4 vols
// aboutis : les 18 autres etaient donc invisibles.
static STEAL_REJECT_INELIGIBLE: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
// Course perdue : identite perimee entre le retrait et la lecture, ou tache
// revendiquee par un autre coeur. Distinct d'un refus d'EQUILIBRE -- ici il y
// avait bien du travail a prendre, quelqu'un a ete plus rapide.
static STEAL_REJECT_COURSE: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
/// Echeance avant laquelle un CPU ne rescane pas les donneurs apres un echec.
/// Le reveil d'une tache locale contourne naturellement ce chemin.
static STEAL_RETRY_AFTER_NS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

/// Temporisation apres un scan de donneurs qui n'avait rien a prendre.
///
/// Ce scan est en O(nombre de CPU) et se fait sous la transition locale a
/// chaque `pick_next` dont la file locale est vide -- c'est-a-dire en
/// permanence sur un CPU peu charge. Deux millisecondes, la meme valeur que le
/// refus de candidat qui existait deja : assez pour ne pas rescaner a chaque
/// commutation, assez court pour qu'un desequilibre reel soit repris avant un
/// quantum.
///
/// Cette temporisation ne peut pas retarder du travail LOCAL : la file locale
/// est videe avant elle, sans condition. Elle ne differe qu'un reequilibrage.
const STEAL_BACKOFF_STERILE_NS: u64 = 2_000_000;

#[derive(Clone, Copy)]
struct SmpSamplePrevious {
    t_ns: u64,
    ctx: u64,
    migrations: [u64; MAX_CPUS],
    steal_ok: [u64; MAX_CPUS],
    steal_try: [u64; MAX_CPUS],
    reject_balance: [u64; MAX_CPUS],
    reject_affinity: [u64; MAX_CPUS],
    page_faults: [u64; MAX_CPUS],
    tlb: u64,
    bkl_wait: u64,
    bkl_hold: u64,
    bkl_acq: u64,
    gpu_presents: u64,
    gpu_bytes: u64,
    irq_preemptions: u64,
    deferred_preemptions: u64,
}

static mut SMP_SAMPLE_PREVIOUS: Option<SmpSamplePrevious> = None;

static CONTEXT_SWITCHES: AtomicU64 = AtomicU64::new(0);
static IRQ_PREEMPTIONS: AtomicU64 = AtomicU64::new(0);
static DEFERRED_PREEMPTIONS: AtomicU64 = AtomicU64::new(0);
static WM_HEARTBEAT_TICK: AtomicU64 = AtomicU64::new(0);
static WM_WATCHDOG_ARMED: AtomicBool = AtomicBool::new(false);
static WM_LAST_WARNING_TICK: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_SMP4_STALL_PROBE_V1
// phase: 0=hors syscall, 1=attente BKL, 2=dans ABI avec BKL.
const STALL_NO_SYSCALL: u64 = u64::MAX;
// BOUCHAUD_P0_BKL_ENREGISTREUR_V1
//
// PID de la tache installee sur ce CPU, lisible SANS verrou.
//
// `CURRENT_PROCESS[cpu]` est derriere un `Mutex` : le lire depuis le chemin
// chaud du gros verrou pourrait bloquer sur le verrou qu'on est en train
// d'instrumenter. `install()` connait deja le pid ; il le depose ici, ce qui
// coute un `store` relaxe par changement de contexte.
static PID_LOCAL: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

/// PID de la tache installee sur `cpu`, sans verrou et sans `rdmsr`.
///
/// L'index est passe par l'appelant : `local_cpu()` lit GS via `rdmsr`, et
/// l'enregistreur de vol du BKL connait deja son CPU. Le refaire lire coutait
/// une sortie de machine virtuelle par transition enregistree.
pub fn pid_pour_sonde(cpu: usize) -> u64 {
    PID_LOCAL[cpu.min(MAX_CPUS - 1)].load(Ordering::Relaxed)
}

static STALL_SYSCALL_NR: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(STALL_NO_SYSCALL) }; MAX_CPUS];
static STALL_SYSCALL_PHASE: [AtomicU32; MAX_CPUS] =
    [const { AtomicU32::new(0) }; MAX_CPUS];
static STALL_SYSCALL_TICK: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

// BOUCHAUD_SMP4_OWNER_SITE_PROBE_V2
// Site noyau courant par CPU, uniquement pour diagnostic.
// 0=aucun/user, 21=page fault+BKL, 31=IPI+BKL, 41=preempt+BKL,
// 50=AP loop+BKL, 52=AP retour switch avant reacquire, 53=AP post-reacquire,
// 54=complete_switch_handoff, 55=activate_kernel, 61=timer+BKL.
static STALL_KERNEL_SITE: [AtomicU32; MAX_CPUS] =
    [const { AtomicU32::new(0) }; MAX_CPUS];
static STALL_KERNEL_AUX: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

// BOUCHAUD_SMP4_OWNER_PROVENANCE_PROBE_V3
// Heartbeat IPI : capture avant toute tentative de BKL. Si l'age IPI du
// CPU proprietaire explose, il ne prend plus ses interruptions.
static STALL_IPI_COUNT: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_IPI_TICK: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_IPI_RIP: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_IPI_USER: [AtomicU32; MAX_CPUS] =
    [const { AtomicU32::new(0) }; MAX_CPUS];
static STALL_IPI_BKL_HIT: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_IPI_BKL_MISS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

// Compteurs page-fault. begin/done mesurent le handler ; file begin/done
// encadrent exactement fs::backing::read_at dans le demand paging.
static STALL_PF_BEGIN: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_PF_DONE: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_PF_FAIL: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_PF_FILE_BEGIN: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_PF_FILE_DONE: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
