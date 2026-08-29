// Etat du pont scheduler <-> BKL.
//
// Ce fichier contient uniquement l'état atomique qui décrit les continuations
// ayant suspendu le BKL et qui doivent restaurer leur profondeur.
//
// IMPORTANT : ces items sont `include!` dans le même module `bkl`. La
// fragmentation est physique, pas une modification de visibilité/API.

// BOUCHAUD_P0_BKL_RESUME_PRIORITY_V3
//
// Une continuation qui revient de `suspend_for_schedule()` n'est pas un nouvel
// entrant : elle avait deja le BKL et doit restaurer sa profondeur avant de
// pouvoir terminer le chemin noyau qui l'a suspendue. La mettre en concurrence
// avec les nouveaux `enter()` permet a une rafale de syscalls de lui passer
// devant indefiniment ("barging"). C'est exactement le motif qui faisait
// remonter `reprise_max_ns` sous charge Ladybird.
//
// Chaque bit publie un CPU dont la continuation est effectivement entree dans
// `resume_after_schedule(depth > 0)`. Les nouveaux entrants cedent la priorite
// tant qu'un de ces bits existe. Le bit est retire uniquement APRES acquisition
// reussie ; il ne peut donc pas disparaitre en laissant une continuation
// endormie sans reveilleur.
//
// SeqCst est volontaire ici : ce bitmap participe au meme protocole de
// vivacite que OWNER/PARKED. Il est minuscule (MAX_CPUS <= 16) et n'est touche
// que lors d'une reprise ou d'une acquisition de BKL.
static RESUME_WAITERS: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_BKL_HEALTH_V4
//
// Observabilite du protocole de priorite. Aucun de ces compteurs n'entre dans
// la correction du verrou : ils expliquent seulement pourquoi un waiter dort
// ou pourquoi un nouvel entrant a ete differe.
static RESUME_SINCE_NS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static RESUME_WAITERS_PEAK: AtomicU32 = AtomicU32::new(0);
static RESUME_PUBLICATIONS: AtomicU64 = AtomicU64::new(0);
static RESUME_MIGRATIONS: AtomicU64 = AtomicU64::new(0);
static PRIORITY_DEFERRALS: AtomicU64 = AtomicU64::new(0);
static PRIORITY_ROLLBACKS: AtomicU64 = AtomicU64::new(0);
static PRIORITY_WAKEUPS: AtomicU64 = AtomicU64::new(0);
static PRIORITY_WAKE_SUPPRESSED: AtomicU64 = AtomicU64::new(0);
static PRIORITY_PARK_FREE_OWNER: AtomicU64 = AtomicU64::new(0);


// Diagnostic scheduler V5 : dernière reprise réussie et reprise active.
static RESUME_ACTIVE_DEPTH: [AtomicUsize; MAX_CPUS] =
    [const { AtomicUsize::new(0) }; MAX_CPUS];
static RESUME_ACTIVE_ATTEMPTS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

static LAST_RESUME_OK_NS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static LAST_RESUME_WAIT_NS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static LAST_RESUME_DEPTH: [AtomicUsize; MAX_CPUS] =
    [const { AtomicUsize::new(0) }; MAX_CPUS];
static LAST_RESUME_ATTEMPTS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

static LAST_SUSPEND_NS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static LAST_SUSPEND_DEPTH: [AtomicUsize; MAX_CPUS] =
    [const { AtomicUsize::new(0) }; MAX_CPUS];

static SCHED_SUSPEND_NONZERO: AtomicU64 = AtomicU64::new(0);
static SCHED_SUSPEND_ZERO: AtomicU64 = AtomicU64::new(0);
static SCHED_SWITCH_BEFORE: AtomicU64 = AtomicU64::new(0);
static SCHED_SWITCH_AFTER: AtomicU64 = AtomicU64::new(0);
static SCHED_RESUME_BEGIN: AtomicU64 = AtomicU64::new(0);
static SCHED_RESUME_OK: AtomicU64 = AtomicU64::new(0);
static SCHED_RESUME_WAIT_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static SCHED_RESUME_WAIT_MAX_NS: AtomicU64 = AtomicU64::new(0);
static SCHED_RESUME_ATTEMPTS_TOTAL: AtomicU64 = AtomicU64::new(0);
