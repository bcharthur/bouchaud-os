// ---------------------------------------------------------------------------
// BOUCHAUD_CONTINUATION_SYNCHRONE_V1 : le banc `continuation-banc`
// ---------------------------------------------------------------------------
//
// Reproduit, en QEMU et sans bureau, le protocole de l'incident TRIGKEY :
// `RUN_NOYAU_RETOUR nom=desktop fil_mort=0` a T+60,519 s.
//
//   1. `run_noyau(pilote)` : une racine noyau epinglee au coeur zero ;
//   2. la racine reste VIVANTE et dort entre deux lancements ;
//   3. elle lance `fils` taches noyau courtes, epinglees au coeur zero, qui
//      meurent pendant qu'elle dort -- donc sur un coeur zero ou rien d'autre
//      n'est pret ;
//   4. avant de sortir, elle lance une tache qui RESTE prete 3 s sur le
//      coeur zero : la mort de la racine doit rendre la main quand meme ;
//   5. `run_noyau` doit revenir UNE fois, racine morte, code 7, AVANT la fin
//      de cette tache.
//
// Le critere 5 est causal, pas chronometrique : sous QEMU, la sortie de la
// racine journalise une douzaine de lignes a 25-50 ms chacune sur la liaison
// serie, et un chronometre mesurerait la liaison. Une continuation perdue, elle,
// n'est reprise qu'a la mort suivante sur le coeur zero -- celle de
// l'occupante, au plus tot : c'est ce que le critere refuse.
//
// Une seule ligne de verdict, puis extinction : le banc est fait pour un
// demarrage dedie (autorun), pas pour une session vivante.

static BANC_FILS_LANCES: AtomicU32 = AtomicU32::new(0);
static BANC_FILS_FINIS: AtomicU32 = AtomicU32::new(0);
static BANC_FILS_DEMANDES: AtomicU32 = AtomicU32::new(0);
static BANC_PILOTE_FINI: AtomicBool = AtomicBool::new(false);
static BANC_SORTIE_PILOTE_MS: AtomicU64 = AtomicU64::new(0);
static BANC_OCCUPE_FIN_MS: AtomicU64 = AtomicU64::new(0);

/// Code de sortie de la racine du banc, et seulement d'elle.
const BANC_CODE_RACINE: i32 = 7;
/// Duree pendant laquelle l'occupante reste prete sur le coeur zero.
const BANC_OCCUPATION_MS: u64 = 3_000;

/// Lance un fil noyau EPINGLE au coeur zero (non migrable).
fn lance_fil_noyau_coeur_zero(entree: fn() -> !, nom: &str) -> bool {
    let Some(process) = new_process(nom, 0) else {
        return false;
    };
    let task = Task::new_kernel(process, entree);
    register(task);
    true
}

fn fil_court_banc() -> ! {
    BANC_FILS_FINIS.fetch_add(1, Ordering::AcqRel);
    exit_current(0)
}

/// Reste pret sur le coeur zero sans jamais bloquer, puis meurt.
fn fil_occupe_banc() -> ! {
    let fin = crate::kernel::timer::monotonic_ms() + BANC_OCCUPATION_MS;
    while crate::kernel::timer::monotonic_ms() < fin {
        yield_now();
    }
    BANC_OCCUPE_FIN_MS.store(crate::kernel::timer::monotonic_ms(), Ordering::Release);
    exit_current(0)
}

fn pilote_banc_continuation() -> ! {
    let demandes = BANC_FILS_DEMANDES.load(Ordering::Acquire);
    for _ in 0..demandes {
        if lance_fil_noyau_coeur_zero(fil_court_banc, "cont-court") {
            BANC_FILS_LANCES.fetch_add(1, Ordering::AcqRel);
        }
        // La racine DORT : le fil court meurt sur un coeur zero vide.
        sleep_ticks(crate::kernel::timer::ms_to_ticks(20).max(1));
    }
    // ... et reste vivante, endormie, jusqu'a la mort du DERNIER fil court :
    // chacune de ces morts trouve une racine vivante.
    let lances = BANC_FILS_LANCES.load(Ordering::Acquire);
    while BANC_FILS_FINIS.load(Ordering::Acquire) < lances {
        sleep_ticks(crate::kernel::timer::ms_to_ticks(20).max(1));
    }
    let _ = lance_fil_noyau_coeur_zero(fil_occupe_banc, "cont-occupe");
    BANC_PILOTE_FINI.store(true, Ordering::Release);
    BANC_SORTIE_PILOTE_MS.store(crate::kernel::timer::monotonic_ms(), Ordering::Release);
    exit_current(BANC_CODE_RACINE)
}

/// `continuation-banc [fils]` : rend 0 si l'invariant tient, 1 sinon, puis
/// eteint la machine avec le meme verdict.
pub fn banc_continuation(fils: u32) -> i32 {
    if in_user_task() {
        crate::println!("[RUN-CONT] refuse : depuis le contexte d'amorcage seulement");
        return -1;
    }
    let fils = fils.clamp(1, 200);
    BANC_FILS_DEMANDES.store(fils, Ordering::Release);
    BANC_FILS_LANCES.store(0, Ordering::Release);
    BANC_FILS_FINIS.store(0, Ordering::Release);
    BANC_PILOTE_FINI.store(false, Ordering::Release);
    BANC_SORTIE_PILOTE_MS.store(0, Ordering::Release);
    BANC_OCCUPE_FIN_MS.store(0, Ordering::Release);
    let detours_avant = detours_idle();

    let code = run_noyau(pilote_banc_continuation, "cont-banc");
    // La reprise elle-meme, pas le retour de `run_noyau` : celui-ci journalise
    // d'abord chaque processus restant, et la liaison serie de QEMU coute
    // ~25 ms par ligne.
    let retour_ms = derniere_reprise_continuation_ms();

    let pilote_fini = BANC_PILOTE_FINI.load(Ordering::Acquire);
    let sortie_ms = BANC_SORTIE_PILOTE_MS.load(Ordering::Acquire);
    let latence_ms = if pilote_fini { retour_ms.saturating_sub(sortie_ms) } else { 0 };
    let lances = BANC_FILS_LANCES.load(Ordering::Acquire);
    let finis = BANC_FILS_FINIS.load(Ordering::Acquire);
    // Sorties menees a l'idle pendant que la continuation etait garee : le
    // cas de l'incident, provoque ici. Avant ce lot, chacune d'elles reprenait
    // la continuation d'une racine vivante.
    let detours = detours_idle() - detours_avant;
    // L'occupante n'a pas encore fini AU MOMENT DE LA REPRISE : la reprise n'a
    // pas attendu une mort etrangere pour avoir lieu.
    let fin_occupe = BANC_OCCUPE_FIN_MS.load(Ordering::Acquire);
    let occupe_fini = fin_occupe != 0 && fin_occupe <= retour_ms;
    let ok = pilote_fini
        && code == BANC_CODE_RACINE
        && lances == fils
        && finis == fils
        && retour_ms != 0
        && !occupe_fini;
    crate::kernel::dmesg::log_fmt(format_args!(
        "[RUN-CONT] v=1 {} cpus={} fils={}/{}/{} pilote_fini={} code={} latence_ms={} \
detours_idle={} occupe_fini_avant_retour={}",
        if ok { "OK" } else { "FAIL" },
        crate::arch::x86_64::smp::schedulable_cpus(),
        finis,
        lances,
        fils,
        pilote_fini as u8,
        code,
        latence_ms,
        detours,
        occupe_fini as u8,
    ));
    let (sortie, raison) = if ok {
        (crate::kernel::power::EXIT_OK, "continuation-banc-ok")
    } else {
        (crate::kernel::power::EXIT_FAIL, "continuation-banc-fail")
    };
    crate::kernel::power::shutdown_avec_raison(sortie, raison)
}
