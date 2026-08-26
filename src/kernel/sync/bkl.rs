//! Verrou noyau global reentrant pour le premier scheduler SMP de Bouchaud OS.
//!
//! Les structures historiques du noyau (`Rc<RefCell<_>>`, allocateur, RAMFS,
//! pilotes) ont ete ecrites avec l'invariant UP. Le passage immediat a des
//! verrous fins partout serait un chantier beaucoup plus vaste que le scheduler
//! lui-meme. Cette couche fournit donc un Big Kernel Lock reentrant par CPU :
//! plusieurs coeurs executent du ring 3 en parallele, mais un seul manipule les
//! structures noyau globales a la fois.
//!
//! Le scheduler peut relacher temporairement le verrou autour d'un changement de
//! contexte. Le nombre de prises reentrantes est restaure lorsque la tache reprend.
//!
//! IMPORTANT SMP/IRQ : OWNER et DEPTH forment ensemble l'etat du verrou.
//! Une IRQ locale ne doit jamais observer l'etat intermediaire entre leur mise a
//! jour. On masque donc les IRQ uniquement pendant ces tres courtes transitions.
//! On ne garde jamais les IRQ masquees pendant l'attente d'un OWNER distant.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use x86_64::instructions::interrupts;

pub const MAX_CPUS: usize = 16;
const FREE: usize = 0;

static OWNER: AtomicUsize = AtomicUsize::new(FREE);
static DEPTH: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

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
/// Plus longue tenue continue du verrou, et ou elle s'est produite.
///
/// Un cumul ne dit rien d'une panne de vivacite : mille tenues d'une
/// microseconde et une tenue de deux secondes donnent la meme somme. C'est le
/// MAXIMUM qui distingue un noyau qui travaille d'un noyau qui gele, et c'est
/// donc lui qu'une non-regression peut affirmer.
static PLUS_LONGUE_TENUE_NS: AtomicU64 = AtomicU64::new(0);
static PLUS_LONGUE_TENUE_SITE: AtomicU32 = AtomicU32::new(0);
static TOTAL_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
/// Acquisitions ventilees par origine : 1 = `enter`, 2 = `try_enter*` (IRQ),
/// 3 = `resume_after_schedule` (reprise d'une pile apres un changement de
/// contexte). Un total seul ne dit pas OU passe le verrou ; ce detail-la, si.
static ACQ_PAR_ORIGINE: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static ACQUIRED_AT_NS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

// BOUCHAUD_P1_BKL_PARK_WAKE_V1
//
// Masque des CPU arretes en attendant ce verrou, et de quoi le mesurer.
// `PARKED` est la seule donnee que le liberateur consulte : tant qu'il vaut
// zero, une liberation ne coute rien de plus qu'avant.
static PARKED: AtomicU64 = AtomicU64::new(0);
static TOTAL_PARKS: AtomicU64 = AtomicU64::new(0);
static TOTAL_WAKE_IPIS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn cpu() -> usize {
    crate::arch::x86_64::usermode::cpu_index().min(MAX_CPUS - 1)
}

#[inline]
fn token(cpu: usize) -> usize {
    cpu + 1
}

#[inline]
fn probe_note_reenter() {
    PROBE_REENTER_SEQ.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn probe_note_acquire(cpu: usize, kind: u32) {
    TOTAL_ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
    crate::kernel::task::stall_site_tenue_reset();
    ACQ_PAR_ORIGINE[(kind as usize).min(3)].fetch_add(1, Ordering::Relaxed);
    ACQUIRED_AT_NS[cpu].store(crate::kernel::timer::monotonic_ns(), Ordering::Relaxed);
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
    let _ = cpu;
}

#[inline]
fn probe_note_release(cpu: usize, kind: u32) {
    let now_ns = crate::kernel::timer::monotonic_ns();
    let acquired = ACQUIRED_AT_NS[cpu].swap(0, Ordering::Relaxed);
    let tenue = now_ns.saturating_sub(acquired);
    TOTAL_HOLD_NS.fetch_add(tenue, Ordering::Relaxed);
    if acquired != 0 && tenue > PLUS_LONGUE_TENUE_NS.load(Ordering::Relaxed) {
        PLUS_LONGUE_TENUE_NS.store(tenue, Ordering::Relaxed);
        PLUS_LONGUE_TENUE_SITE.store(
            crate::kernel::task::stall_site_de_la_tenue(),
            Ordering::Relaxed,
        );
    }
    let generation = PROBE_OWNER_GEN.load(Ordering::Acquire);
    PROBE_RELEASE_SEQ.fetch_add(1, Ordering::AcqRel);
    PROBE_LAST_RELEASE_TICK.store(crate::kernel::timer::ticks(), Ordering::Release);
    PROBE_LAST_RELEASE_CPU.store(cpu, Ordering::Release);
    PROBE_LAST_RELEASE_KIND.store(kind, Ordering::Release);
    PROBE_LAST_RELEASE_GEN.store(generation, Ordering::Release);
}

// BOUCHAUD_BKL_ADAPTIVE_IDLE
//
// Sous Ladybird plusieurs processus entrent en noyau en parallele. Le BKL
// serialise encore ces passages : un spin pur faisait alors consommer un vCPU
// entier a chaque contender. On garde un court spin actif pour les sections
// critiques tres breves, puis on parque le CPU avec HLT si IF est actif.
const BKL_ACTIVE_SPINS: usize = 64;

// BOUCHAUD_P1_BKL_PARK_WAKE_V1
//
// CE QUI A CHANGE, ET POURQUOI
// ----------------------------
// Ce parking datait d'un noyau ou un IPI de quantum partait toutes les 4 ms a
// TOUS les AP. Le dormeur n'avait donc pas besoin d'etre reveille : il l'etait
// de toute facon. BOUCHAUD_P0_TARGETED_SCHED_IPI_V1 a supprime cette diffusion
// -- a juste titre, elle coutait ~250 IPI/s par coeur inutilement -- et le
// parking s'est retrouve sans reveilleur.
//
// Ce qui reste ne suffit pas :
//   * le PIT ne bat que sur le BSP ;
//   * l'IPI de quantum ne vise que les AP qui executent une tache UTILISATEUR
//     (`running_user_cpu_mask`), donc jamais un AP dont `CURRENT` vaut NO_TASK
//     -- exactement l'etat de sa boucle idle, qui appelle pourtant `enter()` ;
//   * `publish_ready` ne reveille que le CPU auquel il destine une tache, et
//     seulement s'il l'a vu `is_idle` -- un CPU peut donc s'arreter juste
//     apres ce test.
//
// Un AP pouvait ainsi s'arreter sur un verrou libre. Les autres chemins le
// rattrapaient en pratique, mais aucun ne le garantissait : c'est la
// definition d'un reveil perdu.
//
// LE PROTOCOLE
// ------------
// Symetrique de celui du scheduler (V14), avec le liberateur pour reveilleur :
//
//     dormeur                          liberateur
//     -------                          ----------
//     CLI                              OWNER <- FREE      (SeqCst)
//     PARKED |= bit    (SeqCst)        lire PARKED        (SeqCst)
//     relire OWNER     (SeqCst)        reveiller ce qui y est
//     libre ? -> repartir sans dormir
//     STI; HLT
//
// AUCUN REVEIL PERDU
// ------------------
// Les quatre acces sont SeqCst, donc totalement ordonnes. Supposons que le
// dormeur s'arrete (il n'a pas vu FREE) et qu'un liberateur R ne voie pas son
// bit. Alors la lecture de PARKED par R precede la pose du bit, et comme R
// ecrit FREE avant de lire PARKED :
//
//     R.store(FREE) < R.load(PARKED) < dormeur.pose(bit) < dormeur.load(OWNER)
//
// La relecture du dormeur voit donc FREE -- il ne dort pas -- ou un
// proprietaire O acquis APRES. Dans ce second cas O relira PARKED apres sa
// propre acquisition, donc apres la pose du bit, et le reveillera. Un
// `Release`/`Acquire` ne suffirait pas ici : c'est un motif de tampon
// d'ecriture, que seul l'ordre total interdit.
//
// Le `sti; hlt` ferme la derniere fenetre : un IPI arrive entre la pose du bit
// et le `hlt` reste pendant dans l'APIC local, et l'ombre du `sti` garantit que
// le `hlt` le prend au lieu de le perdre.
#[inline]
fn wait_for_owner_change(cpu: usize, active_spins: &mut usize) {
    if *active_spins < BKL_ACTIVE_SPINS {
        *active_spins += 1;
        spin_loop();
        return;
    }

    *active_spins = 0;

    // Ne jamais faire STI depuis un contexte qui avait IF=0 (ex. IRQ).
    // Dans ce cas rare on conserve le spin actif : un tel contexte ne peut pas
    // dormir, et il n'a donc pas besoin d'etre reveille.
    if !interrupts::are_enabled() {
        spin_loop();
        return;
    }

    let bit = 1u64 << cpu;
    crate::arch::x86_64::cpu::prepare_lock_park();
    PARKED.fetch_or(bit, Ordering::SeqCst);

    if OWNER.load(Ordering::SeqCst) == FREE {
        // Libere entre le spin et la publication : repartir tout de suite
        // plutot que dormir en attendant un reveil qui n'aura plus lieu.
        PARKED.fetch_and(!bit, Ordering::SeqCst);
        crate::arch::x86_64::cpu::abort_lock_park();
        return;
    }

    TOTAL_PARKS.fetch_add(1, Ordering::Relaxed);
    crate::arch::x86_64::cpu::commit_lock_park();
    PARKED.fetch_and(!bit, Ordering::SeqCst);
}

/// Rappelle les CPU arretes sur ce verrou. A appeler APRES `OWNER <- FREE`.
///
/// Tous, et non le premier venu : ils sont au plus `MAX_CPUS - 1`, ils ne font
/// rien d'autre, et n'en reveiller qu'un ferait dependre la vivacite d'un choix
/// -- si le CPU choisi se rendort sans avoir pris le verrou, plus personne ne
/// releve. `TOTAL_WAKE_IPIS` rend le cout mesurable : s'il devient grand devant
/// le nombre d'acquisitions, c'est qu'un reveil cible vaudrait la peine.
#[inline]
fn wake_parked_waiters(releasing_cpu: usize) {
    let parked = PARKED.load(Ordering::SeqCst);
    if parked == 0 {
        return;
    }

    // Ne parcourir que les bits poses : sous contention normale il y en a un,
    // parfois deux. Balayer les seize CPU a chaque liberation couterait plus
    // cher que le reveil lui-meme.
    let mut restants = parked & !(1u64 << releasing_cpu);
    while restants != 0 {
        let target = restants.trailing_zeros() as usize;
        restants &= restants - 1;
        TOTAL_WAKE_IPIS.fetch_add(1, Ordering::Relaxed);
        crate::arch::x86_64::cpu::wake_parked_cpu(target);
    }
}

/// Serialise les transitions OWNER/DEPTH contre les IRQ du CPU courant.
/// L'etat IF precedent est restaure exactement au Drop.
struct LocalIrqGuard {
    restore_enabled: bool,
}

impl LocalIrqGuard {
    #[inline]
    fn acquire() -> Self {
        let restore_enabled = interrupts::are_enabled();
        interrupts::disable();
        Self { restore_enabled }
    }
}

impl Drop for LocalIrqGuard {
    #[inline]
    fn drop(&mut self) {
        if self.restore_enabled {
            interrupts::enable();
        }
    }
}

pub struct KernelGuard {
    cpu: usize,
    active: bool,
}

impl Drop for KernelGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        // BOUCHAUD_SMP_NG2_BKL_MIGRATION_HOTFIX_V1
        //
        // Un KernelGuard vit sur la pile noyau de la tache. Depuis NG2 cette
        // pile peut reprendre sur un autre CPU apres suspend_for_schedule().
        // resume_after_schedule() restaure alors DEPTH/OWNER sur le NOUVEAU
        // CPU. Le champ `self.cpu` ne represente plus le proprietaire actuel:
        // il indique seulement le CPU sur lequel le guard a ete cree.
        //
        // Liberer `self.cpu` apres une migration donne DEPTH[ancien_cpu] == 0
        // et provoque exactement: "smp_lock: release sans acquisition".
        // La profondeur BKL suit la continuation; son Drop doit donc liberer le
        // CPU physique/logique sur lequel cette continuation s'execute maintenant.
        let release_cpu = cpu();
        release_one(release_cpu);
    }
}

fn release_one(cpu: usize) {
    // OWNER + DEPTH doivent changer atomiquement vis-a-vis d'une IRQ locale.
    let _irq = LocalIrqGuard::acquire();

    let depth = DEPTH[cpu].load(Ordering::Relaxed);
    debug_assert!(depth > 0, "smp_lock: release sans acquisition");
    debug_assert_eq!(
        OWNER.load(Ordering::Acquire),
        token(cpu),
        "smp_lock: release par un CPU non proprietaire"
    );

    if depth > 1 {
        DEPTH[cpu].store(depth - 1, Ordering::Relaxed);
        return;
    }

    DEPTH[cpu].store(0, Ordering::Relaxed);
    probe_note_release(cpu, 1);
    // SeqCst, et non Release : c'est l'ordre total avec la lecture de PARKED
    // ci-dessous qui interdit le reveil perdu. Voir wait_for_owner_change.
    OWNER.store(FREE, Ordering::SeqCst);
    wake_parked_waiters(cpu);
}

pub fn enter() -> KernelGuard {
    let cpu = cpu();
    let mine = token(cpu);
    let mut active_spins = 0usize;
    let wait_start = crate::kernel::timer::monotonic_ns();

    loop {
        // Ne masquer les IRQ que pour le snapshot + la transition locale.
        {
            let _irq = LocalIrqGuard::acquire();
            let owner = OWNER.load(Ordering::Acquire);

            if owner == mine {
                DEPTH[cpu].fetch_add(1, Ordering::Relaxed);
                probe_note_reenter();
                return KernelGuard { cpu, active: true };
            }

            if owner == FREE
                && OWNER
                    .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                // Aucun handler local ne peut voir OWNER=mine avec DEPTH=0.
                DEPTH[cpu].store(1, Ordering::Relaxed);
                probe_note_acquire(cpu, 1);
                TOTAL_WAIT_NS.fetch_add(
                    crate::kernel::timer::monotonic_ns().saturating_sub(wait_start),
                    Ordering::Relaxed,
                );
                return KernelGuard { cpu, active: true };
            }
        }

        // Spin court puis HLT : ne plus bruler un coeur entier sur contention.
        wait_for_owner_change(cpu, &mut active_spins);
    }
}

/// Variante non bloquante, utile aux IPI de preemption : un IPI ne doit pas
/// immobiliser un coeur utilisateur entier si un autre CPU est deja dans le noyau.
pub fn try_enter() -> Option<KernelGuard> {
    let cpu = cpu();
    let mine = token(cpu);
    let _irq = LocalIrqGuard::acquire();

    let owner = OWNER.load(Ordering::Acquire);
    if owner == mine {
        DEPTH[cpu].fetch_add(1, Ordering::Relaxed);
        probe_note_reenter();
        return Some(KernelGuard { cpu, active: true });
    }

    if owner != FREE {
        return None;
    }

    if OWNER
        .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        DEPTH[cpu].store(1, Ordering::Relaxed);
        probe_note_acquire(cpu, 2);
        Some(KernelGuard { cpu, active: true })
    } else {
        None
    }
}

/// Comme [`try_enter`], mais **refuse la reentrance**.
///
/// # Pourquoi elle existe
///
/// Le BKL appartient a un CPU, pas a une tache. Un changement de contexte
/// effectue alors que `OWNER` designe encore ce CPU donnerait la propriete du
/// verrou a la tache ENTRANTE, qui ne l'a jamais demandee, pendant que la pile
/// de la tache sortante croit toujours la detenir. Les deux se croiraient
/// proprietaires ; la premiere a relacher libererait le verrou sous les pieds
/// de l'autre.
///
/// `try_enter` est reentrante, et c'est ce qu'il faut a ses autres appelants :
/// les gestionnaires d'interruption qui veulent seulement toucher un compteur
/// sous verrou, qu'ils l'aient deja ou non. Mais la preemption depuis une IRQ,
/// elle, va COMMUTER : elle doit acquerir depuis la profondeur zero, ou ne pas
/// acquerir du tout.
///
/// Rendre `None` quand ce CPU est deja proprietaire n'est donc pas un echec :
/// c'est la reponse « pas maintenant », que l'appelant traduit en preemption
/// differee.
pub fn try_enter_depuis_zero() -> Option<KernelGuard> {
    let cpu = cpu();
    let mine = token(cpu);
    let _irq = LocalIrqGuard::acquire();

    // Un `OWNER` non libre couvre les deux cas de refus d'un seul test : un
    // autre CPU le detient, ou c'est nous -- et nous, c'est precisement le cas
    // qu'il ne faut pas approfondir. Aucune fenetre entre le test et la prise :
    // les interruptions sont masquees et seul le proprietaire ecrit `OWNER`.
    if OWNER.load(Ordering::Acquire) != FREE {
        return None;
    }

    if OWNER
        .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        DEPTH[cpu].store(1, Ordering::Relaxed);
        probe_note_acquire(cpu, 2);
        Some(KernelGuard { cpu, active: true })
    } else {
        None
    }
}

pub fn held_by_current_cpu() -> bool {
    let cpu = cpu();
    let _irq = LocalIrqGuard::acquire();
    OWNER.load(Ordering::Acquire) == token(cpu)
        && DEPTH[cpu].load(Ordering::Relaxed) > 0
}

/// Libere completement le BKL avant un switch de contexte et rend la profondeur
/// a restaurer lorsque cette pile noyau reprendra.
pub fn suspend_for_schedule() -> usize {
    let cpu = cpu();
    let mine = token(cpu);

    // C'etait la race observee : auparavant DEPTH passait a 0 avant OWNER,
    // avec IRQ encore actives. Le PIT pouvait alors entrer reentrant, puis
    // liberer OWNER avant notre assertion.
    let _irq = LocalIrqGuard::acquire();

    let depth = DEPTH[cpu].load(Ordering::Relaxed);
    if depth == 0 {
        return 0;
    }

    #[cfg(debug_assertions)]

    debug_assert_eq!(
        OWNER.load(Ordering::Acquire),
        mine,
        "smp_lock: suspend sans ownership"
    );

    DEPTH[cpu].store(0, Ordering::Relaxed);
    probe_note_release(cpu, 2);
    OWNER.store(FREE, Ordering::SeqCst);
    // Un changement de contexte libere le verrou aussi reellement qu'un Drop :
    // l'oublier ici laisserait dormir un CPU jusqu'a la prochaine liberation
    // ordinaire, qui peut ne jamais venir si c'est lui qui devait la produire.
    wake_parked_waiters(cpu);
    depth
}

/// Reprend le BKL avec exactement la profondeur qu'avait la tache avant son
/// changement de contexte.
pub fn resume_after_schedule(depth: usize) {
    if depth == 0 {
        return;
    }

    let cpu = cpu();
    let mine = token(cpu);
    let wait_start = crate::kernel::timer::monotonic_ns();

    loop {
        {
            let _irq = LocalIrqGuard::acquire();

            if OWNER
                .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Aucun handler local ne peut observer OWNER=mine avant que
                // la profondeur de la pile reprise soit restauree.
                DEPTH[cpu].store(depth, Ordering::Relaxed);
                probe_note_acquire(cpu, 3);
                TOTAL_WAIT_NS.fetch_add(
                    crate::kernel::timer::monotonic_ns().saturating_sub(wait_start),
                    Ordering::Relaxed,
                );
                return;
            }
        }

        // BOUCHAUD_P0_TARGETED_IPI_LIVENESS_V13
        //
        // Do NOT HLT while resuming a suspended scheduler/kernel continuation.
        // With targeted scheduler IPIs there is no longer a 4 ms broadcast
        // heartbeat guaranteed to wake this CPU after BKL release.
        //
        // The CPU was explicitly woken because it has useful work. Busy-wait
        // here until OWNER becomes free; ordinary enter() keeps the adaptive
        // HLT policy for unrelated BKL contention.
        spin_loop();
    }
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
    (
        PLUS_LONGUE_TENUE_NS.load(Ordering::Relaxed),
        PLUS_LONGUE_TENUE_SITE.load(Ordering::Relaxed),
    )
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
pub fn stall_probe_owner_token() -> usize {
    OWNER.load(Ordering::Acquire)
}

pub fn stall_probe_depth(cpu: usize) -> usize {
    DEPTH[cpu.min(MAX_CPUS - 1)].load(Ordering::Acquire)
}
