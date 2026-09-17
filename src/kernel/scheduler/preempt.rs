//! Deferred kernel preemption at explicit safe points.
//!
//! Bouchaud P0-NG1 deliberately does not switch stacks from arbitrary ring-0
//! interrupt contexts. Timer/IPI code requests a reschedule; code that reaches
//! a safe boundary performs it only when interrupts are enabled, the BKL is not
//! held, no ranked critical section is active and the current task is a normal
//! user task temporarily executing in the kernel.

use core::sync::atomic::{AtomicU64, Ordering};
use crate::arch::x86_64::{cpu, cpu_local::{self, CpuId}, smp};

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static SAFE_POINTS: AtomicU64 = AtomicU64::new(0);
static SWITCHES: AtomicU64 = AtomicU64::new(0);
static BLOCKED_BKL: AtomicU64 = AtomicU64::new(0);
static BLOCKED_PREEMPT: AtomicU64 = AtomicU64::new(0);
static BLOCKED_CONTEXT: AtomicU64 = AtomicU64::new(0);
static REQUEST_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
// BOUCHAUD_C2_REPORT_REFUSE_V1
//
// Instant du PREMIER refus servi a une demande de preemption encore pendante.
// Zero tant qu'aucun point sur ne l'a refusee.
//
// # Pourquoi ce compteur n'est pas `REQUEST_NS`
//
// `REQUEST_NS` date la DEMANDE. L'ecart entre la demande et le service
// contient donc tout le temps ou ce coeur n'a simplement rien execute : sur un
// coeur au repos, personne n'appelle `safe_point`, et la mesure grandit sans
// que personne n'attende. Une trace SMP4 mesuree l'a montre -- 2,1 s de
// « report » avec `blocked_*=0`, c'est-a-dire pas un seul refus, pendant que la
// latence prete->coeur plafonnait a 19,8 ms. Le chiffre ne mesurait pas un
// figement : il mesurait de l'inactivite.
//
// Ce qu'un budget doit borner, c'est le report SUBI : une preemption demandee,
// un point sur atteint, et le refus qui s'ensuit -- verrou global tenu,
// section critique, contexte d'interruption. Cela seul est du ressort des
// chantiers 1 et 2, et cela seul peut figer une interface.
static REFUS_NS: [AtomicU64; smp::MAX_CPUS] =
    [const { AtomicU64::new(0) }; smp::MAX_CPUS];
static MAX_DEFER_NS: AtomicU64 = AtomicU64::new(0);
static MAX_ATTENTE_NS: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_P0_REVEIL_CIBLE_V1
//
// LA DEMANDE CIBLEE, ET POURQUOI ELLE EST UN DRAPEAU SEPARE
//
// `need_resched` dit « il y aurait mieux a faire ». Il est pose a chaque
// publication, des milliers de fois par seconde, et trois mecanismes le
// refusent quand la tache courante est une tache noyau : l'IPI de
// replanification n'en fait rien, le balayage de quantum saute le coeur, et
// aucun point sur n'existe dans un fil noyau. C'est la raison exacte pour
// laquelle `usb-hid` a attendu 6,78 s sur le releve TRIGKEY.
//
// Lever ces refus POUR TOUT `need_resched` reviendrait a autoriser la
// commutation de pile depuis une IRQ ring 0 sur tout le systeme -- exactement
// ce que P0-NG1 a ferme volontairement.
//
// Ce drapeau-ci dit autre chose : « une tache SENSIBLE A LA LATENCE attend ce
// coeur, et la politique de reveil a juge que l'occupant doit ceder ». Il
// n'est pose que par `publish_ready`, pour une tache qui porte
// `latency_sensitive` et qui est restee sous son budget d'activation. Le
// nouveau chemin de preemption noyau ne s'ouvre donc que pour lui, et se
// referme des qu'il est servi.
//
// IL N'EST PAS CONSOMME QUAND LA PREEMPTION EST REFUSEE. Un fil noyau qui
// tient un verrou au moment de l'IPI n'a qu'a le rendre : la demande reste
// pendante, le balayage de quantum reexpedie un IPI au coeur tant qu'elle
// l'est, et la commutation a lieu au premier instant sur. La consommer au
// refus rendrait la tache invisible jusqu'a son PROCHAIN reveil -- qui
// n'arrivera jamais, puisqu'elle attend d'etre elue pour se rendormir.
static CIBLEE: [core::sync::atomic::AtomicBool; smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicBool::new(false) }; smp::MAX_CPUS];

static REVEILS_IMMEDIATS: AtomicU64 = AtomicU64::new(0);
static REVEILS_CIBLES: AtomicU64 = AtomicU64::new(0);
static REVEILS_DIFFERES: AtomicU64 = AtomicU64::new(0);
static REVEILS_EN_FILE: AtomicU64 = AtomicU64::new(0);
static IPI_REVEIL_ENVOYES: AtomicU64 = AtomicU64::new(0);
static PREEMPTIONS_NOYAU_ACCORDEES: AtomicU64 = AtomicU64::new(0);
static PREEMPTIONS_NOYAU_REFUSEES: AtomicU64 = AtomicU64::new(0);
static PLACEMENTS_DEPLACES: AtomicU64 = AtomicU64::new(0);

/// Ce que la politique de reveil a decide, et ce qui en est advenu.
#[derive(Clone, Copy, Debug, Default)]
pub struct StatsReveil {
    pub immediats: u64,
    pub cibles: u64,
    pub differes: u64,
    pub en_file: u64,
    pub ipi_envoyes: u64,
    pub preemptions_noyau: u64,
    pub preemptions_noyau_refusees: u64,
    pub placements_deplaces: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub requests: u64,
    pub safe_points: u64,
    pub switches: u64,
    pub blocked_bkl: u64,
    pub blocked_preempt: u64,
    pub blocked_context: u64,
    /// Plus long report SUBI : du premier refus au service effectif.
    pub max_defer_ns: u64,
    /// Plus longue attente demande->service, INACTIVITE COMPRISE. Diagnostic
    /// seul : sur un coeur au repos elle grandit sans que personne n'attende,
    /// et elle n'est donc bornee par aucun budget.
    pub max_attente_ns: u64,
}

/// Date le premier refus d'une demande encore pendante, et lui seul.
#[inline]
fn note_refus(index: usize) {
    let _ = REFUS_NS[index].compare_exchange(
        0,
        crate::kernel::timer::monotonic_ns(),
        Ordering::AcqRel,
        Ordering::Relaxed,
    );
}

#[inline]
fn local_id() -> Option<CpuId> { CpuId::from_index(smp::cpu_index()) }

pub fn disable() {
    if let Some(id) = local_id() { cpu_local::local(id).preempt_disable(); }
}

pub fn enable() {
    if let Some(id) = local_id() { cpu_local::local(id).preempt_enable(); }
}

pub fn request_local() { request_cpu(smp::cpu_index()); }

/// Une tache sensible a la latence attend ce coeur : son occupant doit ceder.
///
/// Pose le drapeau AVANT la demande ordinaire, pour qu'un IPI deja en vol ne
/// puisse pas trouver `need_resched` pose et la demande ciblee absente.
pub fn demande_ciblee(cpu_index: usize) {
    if cpu_index < smp::MAX_CPUS {
        CIBLEE[cpu_index].store(true, Ordering::Release);
    }
    request_cpu(cpu_index);
}

/// Y a-t-il une demande ciblee pendante sur ce coeur ? Lecture seule.
#[inline]
pub fn demande_ciblee_pendante() -> bool {
    local_id()
        .map(|id| CIBLEE[id.as_usize()].load(Ordering::Acquire))
        .unwrap_or(false)
}

/// Rend la demande ciblee de ce coeur. Appele quand la commutation a EU LIEU.
#[inline]
pub fn rend_demande_ciblee() {
    if let Some(id) = local_id() {
        CIBLEE[id.as_usize()].store(false, Ordering::Release);
    }
}

/// Les coeurs qui ont une demande ciblee pendante.
///
/// Le balayage de quantum s'en sert pour REEXPEDIER un IPI a un coeur qui
/// execute un fil noyau -- coeur que `running_user_cpu_mask` exclut par
/// construction. C'est la seule facon qu'une demande refusee a un instant
/// donne a d'etre reexaminee au suivant.
pub fn masque_cible() -> u64 {
    let mut masque = 0u64;
    let en_ligne = smp::schedulable_cpus().min(smp::MAX_CPUS).min(64);
    let mut cpu = 0usize;
    while cpu < en_ligne {
        if CIBLEE[cpu].load(Ordering::Acquire) {
            masque |= 1u64 << cpu;
        }
        cpu += 1;
    }
    masque
}

/// Ce coeur peut-il commuter depuis son contexte d'IRQ courant ?
///
/// Memes conditions que `safe_point`, MOINS la seule qui excluait les taches
/// noyau. Chacune reste indispensable :
///
///   * `preempt_count` non nul : quelqu'un a explicitement demande a ne pas
///     etre commute ;
///   * une section critique rangee est ouverte : commuter la laisserait
///     ouverte sur un autre coeur ;
///   * le gros verrou est tenu par ce coeur : le rendre ailleurs est
///     impossible, et `preempt_from_irq` l'interdit par assertion ;
///   * le gros verrou est tenu a une profondeur non nulle : meme cause ;
///   * un VERROU TOURNANT SIMPLE est tenu par ce coeur : couper son porteur
///     ferait tourner la tache entrante sur ce meme verrou, sur ce meme
///     coeur, pendant que la sortante attend un coeur pour le rendre. Ni
///     `lockdep` ni le masquage d'interruption ne l'auraient dit :
///     `SpinLock` ne fait ni l'un ni l'autre.
///
/// Ce qui n'y figure PAS : « la tache courante est-elle une tache noyau ».
/// Une tache noyau appelle deja `schedule()` d'elle-meme -- c'est ce que fait
/// `sleep_ticks` a chaque milliseconde --, et `preempt_from_irq` ne lit rien
/// d'autre de la tache sortante que ce que `switch_to` lit aussi. La commuter
/// depuis une IRQ, quand tout ce qui precede est verifie, est la MEME
/// operation.
///
/// Ce qui n'y figure pas non plus : la REENTRANCE. Elle n'a pas besoin d'etre
/// verifiee ici parce qu'elle l'est plus bas, et mieux :
/// `preempt_from_irq` ouvre `commence_transition_ordonnanceur()`, une porte
/// par coeur, et rend la main sans rien commuter si une transition est deja
/// ouverte. Une IRQ qui interrompt le scheduler ne peut donc pas en elire une
/// seconde, quel que soit le chemin par lequel elle est arrivee.
pub fn preemption_noyau_sure() -> bool {
    let Some(id) = local_id() else { return false; };
    let local = cpu_local::local(id);
    local.preempt_count() == 0
        && local.verrous_simples() == 0
        && crate::kernel::sync::lockdep::depth() == 0
        && crate::kernel::smp_lock::profondeur_locale() == 0
        && !crate::kernel::smp_lock::held_by_current_cpu()
}

/// Accorde-t-on la preemption d'un fil noyau sur ce coeur, maintenant ?
///
/// Rend `true` UNE fois, et seulement si une demande ciblee est pendante et
/// que le contexte est sur. La demande n'est pas rendue ici : elle l'est
/// quand la commutation a effectivement eu lieu, pour qu'un refus ne fasse
/// pas disparaitre une tache qui attend d'etre elue pour se rendormir.
pub fn accorde_preemption_noyau() -> bool {
    if !demande_ciblee_pendante() {
        return false;
    }
    if preemption_noyau_sure() {
        PREEMPTIONS_NOYAU_ACCORDEES.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    PREEMPTIONS_NOYAU_REFUSEES.fetch_add(1, Ordering::Relaxed);
    false
}

pub fn note_reveil_immediat() { REVEILS_IMMEDIATS.fetch_add(1, Ordering::Relaxed); }
pub fn note_reveil_cible() { REVEILS_CIBLES.fetch_add(1, Ordering::Relaxed); }
pub fn note_reveil_differe() { REVEILS_DIFFERES.fetch_add(1, Ordering::Relaxed); }
pub fn note_reveil_en_file() { REVEILS_EN_FILE.fetch_add(1, Ordering::Relaxed); }
pub fn note_ipi_reveil() { IPI_REVEIL_ENVOYES.fetch_add(1, Ordering::Relaxed); }
pub fn note_placement_deplace() { PLACEMENTS_DEPLACES.fetch_add(1, Ordering::Relaxed); }

pub fn stats_reveil() -> StatsReveil {
    StatsReveil {
        immediats: REVEILS_IMMEDIATS.load(Ordering::Relaxed),
        cibles: REVEILS_CIBLES.load(Ordering::Relaxed),
        differes: REVEILS_DIFFERES.load(Ordering::Relaxed),
        en_file: REVEILS_EN_FILE.load(Ordering::Relaxed),
        ipi_envoyes: IPI_REVEIL_ENVOYES.load(Ordering::Relaxed),
        preemptions_noyau: PREEMPTIONS_NOYAU_ACCORDEES.load(Ordering::Relaxed),
        preemptions_noyau_refusees: PREEMPTIONS_NOYAU_REFUSEES.load(Ordering::Relaxed),
        placements_deplaces: PLACEMENTS_DEPLACES.load(Ordering::Relaxed),
    }
}

pub fn request_cpu(cpu_index: usize) {
    let Some(id) = CpuId::from_index(cpu_index) else { return; };
    let local = cpu_local::local(id);
    if !local.need_resched() {
        REQUEST_NS[cpu_index].store(crate::kernel::timer::monotonic_ns(), Ordering::Release);
        REQUESTS.fetch_add(1, Ordering::Relaxed);
    }
    local.request_resched();
}

pub fn pending() -> bool {
    local_id().map(|id| cpu_local::local(id).need_resched()).unwrap_or(false)
}

/// Execute a deferred reschedule only at a fully preemptible kernel boundary.
pub fn safe_point() -> bool {
    let Some(id) = local_id() else { return false; };
    let index = id.as_usize();
    let local = cpu_local::local(id);
    if !local.need_resched() { return false; }
    SAFE_POINTS.fetch_add(1, Ordering::Relaxed);

    if !cpu::interrupts_enabled() || local.irq_depth() != 0 {
        BLOCKED_CONTEXT.fetch_add(1, Ordering::Relaxed);
        note_refus(index);
        return false;
    }
    if local.preempt_count() != 0 || crate::kernel::sync::lockdep::depth() != 0 {
        BLOCKED_PREEMPT.fetch_add(1, Ordering::Relaxed);
        note_refus(index);
        return false;
    }
    if crate::kernel::smp_lock::held_by_current_cpu() {
        BLOCKED_BKL.fetch_add(1, Ordering::Relaxed);
        note_refus(index);
        return false;
    }
    if !crate::kernel::task::in_user_task() || crate::kernel::task::current_is_kernel_task() {
        BLOCKED_CONTEXT.fetch_add(1, Ordering::Relaxed);
        note_refus(index);
        return false;
    }

    local.clear_resched();
    let requested = REQUEST_NS[index].swap(0, Ordering::AcqRel);
    let refuse = REFUS_NS[index].swap(0, Ordering::AcqRel);
    let maintenant = crate::kernel::timer::monotonic_ns();
    // Le BUDGET porte sur le report subi : sans refus, il n'y a rien a borner.
    if refuse != 0 {
        MAX_DEFER_NS.fetch_max(maintenant.saturating_sub(refuse), Ordering::Relaxed);
    }
    if requested != 0 {
        MAX_ATTENTE_NS.fetch_max(maintenant.saturating_sub(requested), Ordering::Relaxed);
    }
    let switched = crate::kernel::task::schedule();
    if switched { SWITCHES.fetch_add(1, Ordering::Relaxed); }
    switched
}

pub fn cond_resched() -> bool { safe_point() }

pub fn stats() -> Stats {
    Stats {
        requests: REQUESTS.load(Ordering::Relaxed),
        safe_points: SAFE_POINTS.load(Ordering::Relaxed),
        switches: SWITCHES.load(Ordering::Relaxed),
        blocked_bkl: BLOCKED_BKL.load(Ordering::Relaxed),
        blocked_preempt: BLOCKED_PREEMPT.load(Ordering::Relaxed),
        blocked_context: BLOCKED_CONTEXT.load(Ordering::Relaxed),
        max_defer_ns: MAX_DEFER_NS.load(Ordering::Relaxed),
        max_attente_ns: MAX_ATTENTE_NS.load(Ordering::Relaxed),
    }
}

pub fn log_reveil() {
    let r = stats_reveil();
    crate::serial_println!(
        "[SCHED-NG-REVEIL] immediats={} cibles={} differes={} en_file={} ipi={} preempt_noyau={}/{} deplaces={}",
        r.immediats, r.cibles, r.differes, r.en_file, r.ipi_envoyes,
        r.preemptions_noyau, r.preemptions_noyau.saturating_add(r.preemptions_noyau_refusees),
        r.placements_deplaces
    );
}

pub fn log_stats() {
    let s = stats();
    crate::serial_println!(
        "[SCHED-NG-PREEMPT] requests={} safe={} switches={} blocked_bkl={} blocked_preempt={} blocked_ctx={} max_defer_ns={} attente_service_max_ns={}",
        s.requests, s.safe_points, s.switches, s.blocked_bkl,
        s.blocked_preempt, s.blocked_context, s.max_defer_ns, s.max_attente_ns
    );
}
