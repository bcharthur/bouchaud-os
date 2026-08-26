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
    let mut record = PLUS_LONGUE_TENUE_NS.load(Ordering::Relaxed);
    loop {
        if tenue <= record {
            return;
        }
        match PLUS_LONGUE_TENUE_NS.compare_exchange_weak(
            record,
            tenue,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actuel) => record = actuel,
        }
    }

    // Gagne : on est seul a ecrire la provenance de CETTE duree.
    MAX_TENUE_GEN.fetch_add(1, Ordering::AcqRel); // -> impaire
    MAX_TENUE_CPU.store(cpu, Ordering::Relaxed);
    MAX_TENUE_TACHE.store(TENUE_TACHE[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    MAX_TENUE_SYSCALL.store(TENUE_SYSCALL[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    MAX_TENUE_PHASE.store(TENUE_PHASE[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    MAX_TENUE_SITE_ACQ.store(TENUE_SITE_ACQ[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    MAX_TENUE_ORIGINE.store(TENUE_ORIGINE[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
    // Le site marque PENDANT la tenue : quand il existe il est plus precis que
    // la provenance de l'acquisition ; quand il vaut zero -- le cas des tenues
    // longues -- la provenance prend le relais.
    PLUS_LONGUE_TENUE_SITE.store(
        crate::kernel::task::stall_site_de_la_tenue(),
        Ordering::Relaxed,
    );
    MAX_TENUE_GEN.fetch_add(1, Ordering::AcqRel); // -> paire
}
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

    // BOUCHAUD_P1_BKL_MAX_HOLD_PROVENANCE_V1 : la meme lecture, conservee par
    // CPU pour pouvoir l'attribuer a la tenue quand elle se terminera.
    TENUE_CPU[cpu].store(cpu, Ordering::Relaxed);
    TENUE_TACHE[cpu].store(task, Ordering::Relaxed);
    TENUE_SYSCALL[cpu].store(syscall_nr, Ordering::Relaxed);
    TENUE_PHASE[cpu].store(syscall_phase, Ordering::Relaxed);
    TENUE_SITE_ACQ[cpu].store(site, Ordering::Relaxed);
    TENUE_ORIGINE[cpu].store(kind, Ordering::Relaxed);
}

#[inline]
fn probe_note_release(cpu: usize, kind: u32) {
    let now_ns = crate::kernel::timer::monotonic_ns();
    let acquired = ACQUIRED_AT_NS[cpu].swap(0, Ordering::Relaxed);
    let tenue = now_ns.saturating_sub(acquired);
    TOTAL_HOLD_NS.fetch_add(tenue, Ordering::Relaxed);
    if acquired != 0 {
        publie_si_plus_longue(cpu, tenue);
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

impl KernelGuard {
    /// Identite d'un garde pour l'enregistreur de vol : son ADRESSE sur la pile
    /// noyau. Deux gardes imbriques d'une meme tache ont le meme `cpu` et la
    /// meme tache ; seule leur adresse les distingue, et c'est justement ce
    /// qu'il faut pour savoir lequel a ete relache deux fois.
    #[inline]
    fn identite(&self) -> u64 {
        self as *const KernelGuard as u64
    }
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
        {
            let owner = OWNER.load(Ordering::Relaxed);
            let depth = DEPTH[release_cpu].load(Ordering::Relaxed);
            enregistreur::note(
                enregistreur::GUARD_DROP,
                release_cpu,
                owner,
                owner,
                depth,
                depth,
                self.cpu,
                self.identite(),
            );
        }
        release_one(release_cpu);
    }
}

fn release_one(cpu: usize) {
    // OWNER + DEPTH doivent changer atomiquement vis-a-vis d'une IRQ locale.
    let _irq = LocalIrqGuard::acquire();

    let depth = DEPTH[cpu].load(Ordering::Relaxed);
    let owner = OWNER.load(Ordering::Acquire);

    // Enregistrer AVANT les assertions : c'est cette transition-la qui explique
    // la violation, et une assertion qui panique n'y reviendrait jamais.
    enregistreur::note(
        enregistreur::RELEASE,
        cpu,
        owner,
        if depth > 1 { owner } else { FREE },
        depth,
        depth.saturating_sub(1),
        usize::MAX,
        token(cpu) as u64,
    );

    debug_assert!(depth > 0, "smp_lock: release sans acquisition");
    debug_assert_eq!(
        owner,
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
                let avant = DEPTH[cpu].fetch_add(1, Ordering::Relaxed);
                probe_note_reenter();
                enregistreur::note(
                    enregistreur::REENTER, cpu, owner, owner,
                    avant, avant + 1, usize::MAX, 0,
                );
                return KernelGuard { cpu, active: true };
            }

            if owner == FREE
                && OWNER
                    .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                // Aucun handler local ne peut voir OWNER=mine avec DEPTH=0.
                let avant = DEPTH[cpu].load(Ordering::Relaxed);
                DEPTH[cpu].store(1, Ordering::Relaxed);
                probe_note_acquire(cpu, 1);
                enregistreur::note(
                    enregistreur::ENTER, cpu, FREE, mine, avant, 1, usize::MAX, 0,
                );
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
        let avant = DEPTH[cpu].fetch_add(1, Ordering::Relaxed);
        probe_note_reenter();
        enregistreur::note(
            enregistreur::REENTER, cpu, owner, owner, avant, avant + 1, usize::MAX, 1,
        );
        return Some(KernelGuard { cpu, active: true });
    }

    if owner != FREE {
        return None;
    }

    if OWNER
        .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let avant = DEPTH[cpu].load(Ordering::Relaxed);
        DEPTH[cpu].store(1, Ordering::Relaxed);
        probe_note_acquire(cpu, 2);
        enregistreur::note(
            enregistreur::TRY_ENTER, cpu, FREE, mine, avant, 1, usize::MAX, 1,
        );
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
        let avant = DEPTH[cpu].load(Ordering::Relaxed);
        DEPTH[cpu].store(1, Ordering::Relaxed);
        probe_note_acquire(cpu, 2);
        enregistreur::note(
            enregistreur::TRY_ENTER, cpu, FREE, mine, avant, 1, usize::MAX, 2,
        );
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

    enregistreur::note(
        enregistreur::SUSPEND, cpu, mine, FREE, depth, 0, usize::MAX, depth as u64,
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

    {
        let owner = OWNER.load(Ordering::Relaxed);
        enregistreur::note(
            enregistreur::RESUME_BEGIN,
            cpu,
            owner,
            owner,
            DEPTH[cpu].load(Ordering::Relaxed),
            depth,
            usize::MAX,
            depth as u64,
        );
    }

    loop {
        {
            let _irq = LocalIrqGuard::acquire();

            if OWNER
                .compare_exchange(FREE, mine, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Aucun handler local ne peut observer OWNER=mine avant que
                // la profondeur de la pile reprise soit restauree.
                let avant = DEPTH[cpu].load(Ordering::Relaxed);
                DEPTH[cpu].store(depth, Ordering::Relaxed);
                probe_note_acquire(cpu, 3);
                enregistreur::note(
                    enregistreur::RESUME_OK, cpu, FREE, mine,
                    avant, depth, usize::MAX, depth as u64,
                );
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

/// Provenance de la plus longue tenue : (cpu, tache, syscall, phase, site
/// d'acquisition, origine de l'acquisition).
///
/// `origine` reprend le codage de `probe_note_acquire` : 1 = `enter`,
/// 2 = `try_enter`, 3 = `resume_after_schedule`.
pub fn provenance_plus_longue_tenue() -> (usize, usize, u64, u32, u32, u32) {
    // Lecture seqlock : reessayer tant que la generation bouge ou est impaire.
    // Quatre tours suffisent -- l'ecriture ne fait que six stores -- et au-dela
    // on rend ce qu'on a plutot que de boucler dans un chemin de journal.
    for _ in 0..4 {
        let debut = MAX_TENUE_GEN.load(Ordering::Acquire);
        if debut % 2 != 0 {
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
    }
    (
        MAX_TENUE_CPU.load(Ordering::Relaxed),
        MAX_TENUE_TACHE.load(Ordering::Relaxed),
        MAX_TENUE_SYSCALL.load(Ordering::Relaxed),
        MAX_TENUE_PHASE.load(Ordering::Relaxed),
        MAX_TENUE_SITE_ACQ.load(Ordering::Relaxed),
        MAX_TENUE_ORIGINE.load(Ordering::Relaxed),
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

// BOUCHAUD_P0_BKL_ENREGISTREUR_V1
//
// POURQUOI UN ENREGISTREUR DE VOL
// -------------------------------
// L'assertion `release par un CPU non proprietaire` dit QUE l'etat est casse
// (`DEPTH[cpu] > 0` alors que `OWNER == FREE`), jamais QUI l'a casse. Les
// sondes existantes ne gardent que le DERNIER evenement de chaque sorte : elles
// donnent une photo, pas la sequence. Or ce qu'il faut ici est precisement la
// sequence -- la transition qui a decouple les deux moities de l'etat.
//
// CE QU'IL N'A PAS LE DROIT DE FAIRE
// ----------------------------------
// Allouer -- l'allocateur est sous ce verrou. Prendre un verrou -- ce serait le
// verrou lui-meme. Formater -- `core::fmt` peut fauter et appelle du code
// arbitraire. Il ne fait donc que des `store` relaxes dans un anneau statique,
// et tout le texte est fabrique au moment du vidage, une seule fois.
//
// POURQUOI UN ANNEAU GLOBAL ET NON PAR CPU
// ----------------------------------------
// La question posee est un ENTRELACEMENT entre CPU. Un anneau par CPU donne
// quatre listes qu'on ne peut plus remettre dans l'ordre : c'est exactement
// l'information qui manque. Le `fetch_add` global coute une ligne de cache
// partagee, et c'est le prix de l'ordre total.
//
// POURQUOI IL N'EXISTE QU'EN `debug_assertions`
// ---------------------------------------------
// Il n'est la que pour expliquer un `debug_assert!`. Le compiler en release
// ferait payer une ligne de cache partagee a chaque prise du gros verrou pour
// une trace que personne ne lira jamais.
#[cfg(debug_assertions)]
pub mod enregistreur {
    use super::*;
    use core::sync::atomic::AtomicBool;

    /// Nature de la transition. Les valeurs sont stables : elles sont relues
    /// telles quelles dans le vidage.
    pub const ENTER: u8 = 1;
    pub const REENTER: u8 = 2;
    pub const TRY_ENTER: u8 = 3;
    pub const GUARD_DROP: u8 = 4;
    pub const RELEASE: u8 = 5;
    pub const SUSPEND: u8 = 6;
    pub const RESUME_BEGIN: u8 = 7;
    pub const RESUME_OK: u8 = 8;
    pub const SWITCH_BEFORE: u8 = 9;
    pub const SWITCH_AFTER: u8 = 10;

    fn nom(kind: u8) -> &'static str {
        match kind {
            ENTER => "ENTER",
            REENTER => "REENTER",
            TRY_ENTER => "TRY_ENTER",
            GUARD_DROP => "GUARD_DROP",
            RELEASE => "RELEASE",
            SUSPEND => "SUSPEND",
            RESUME_BEGIN => "RESUME_BEGIN",
            RESUME_OK => "RESUME_OK",
            SWITCH_BEFORE => "SWITCH_BEFORE",
            SWITCH_AFTER => "SWITCH_AFTER",
            _ => "?",
        }
    }

    /// 256 transitions gardees, 64 videes. La marge sert a ne pas perdre le
    /// contexte quand la violation est precedee d'une rafale d'IRQ.
    const TAILLE: usize = 256;
    /// Nombre de transitions imprimees sur violation.
    const VIDAGE: usize = 64;

    /// Une case de l'anneau. Huit `u64` = 64 octets, soit une ligne de cache :
    /// deux cases voisines ne se disputent jamais la meme.
    struct Case {
        /// Numero d'ordre global. Ecrit en DERNIER : une case dont le `seq` est
        /// a jour a tous ses autres champs a jour.
        seq: AtomicU64,
        /// kind | cpu | owner_avant | owner_apres | depth_avant | depth_apres
        /// | cpu_du_garde | phase, un octet chacun.
        etat: AtomicU64,
        /// index de tache (32 bits bas) | tid (32 bits hauts).
        tache: AtomicU64,
        pid: AtomicU64,
        syscall: AtomicU64,
        /// Selon `kind` : profondeur sauvegardee, `from|to`, ou identite du garde.
        aux: AtomicU64,
        /// RSP au moment de la transition : identifie la CONTINUATION, ce qu'un
        /// numero de tache ne fait pas -- une meme tache a plusieurs cadres.
        rsp: AtomicU64,
        /// `TSS.rsp0` courant : la pile noyau que ce CPU est cense servir.
        /// Deux CPU qui affichent la meme valeur sont sur la meme pile.
        kstack: AtomicU64,
    }

    impl Case {
        const fn vide() -> Self {
            Self {
                seq: AtomicU64::new(0),
                etat: AtomicU64::new(0),
                tache: AtomicU64::new(0),
                pid: AtomicU64::new(0),
                syscall: AtomicU64::new(0),
                aux: AtomicU64::new(0),
                rsp: AtomicU64::new(0),
                kstack: AtomicU64::new(0),
            }
        }
    }

    static ANNEAU: [Case; TAILLE] = [const { Case::vide() }; TAILLE];
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    /// Gele l'anneau pendant le vidage. Sans lui, les autres CPU -- qui ne sont
    /// arretes qu'APRES le releve -- ecraseraient les cases qu'on est en train
    /// de lire, et le vidage montrerait la fin de l'histoire a la place du
    /// debut.
    static GEL: AtomicBool = AtomicBool::new(false);

    #[inline]
    fn rsp_courant() -> u64 {
        let rsp: u64;
        unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack)) };
        rsp
    }

    #[inline]
    fn octet(valeur: usize) -> u64 {
        (valeur & 0xFF) as u64
    }

    /// Enregistre une transition. Aucun format, aucune allocation, aucun verrou.
    ///
    /// `garde_cpu` vaut `usize::MAX` quand la notion n'a pas de sens (tout sauf
    /// `GUARD_DROP`), et le vidage l'imprime alors `-`.
    #[inline]
    pub fn note(
        kind: u8,
        cpu: usize,
        owner_avant: usize,
        owner_apres: usize,
        depth_avant: usize,
        depth_apres: usize,
        garde_cpu: usize,
        aux: u64,
    ) {
        if GEL.load(Ordering::Relaxed) {
            return;
        }
        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
        let case = &ANNEAU[(seq as usize) % TAILLE];

        let (index, syscall_nr, phase, _site, _aux) =
            crate::kernel::task::stall_probe_local_context();
        let (tid, kstack) = {
            let per_cpu = crate::arch::x86_64::usermode::per_cpu();
            (per_cpu.current, per_cpu.kernel_rsp)
        };

        case.etat.store(
            octet(kind as usize)
                | (octet(cpu) << 8)
                | (octet(owner_avant) << 16)
                | (octet(owner_apres) << 24)
                | (octet(depth_avant) << 32)
                | (octet(depth_apres) << 40)
                | (octet(garde_cpu) << 48)
                | (octet(phase as usize) << 56),
            Ordering::Relaxed,
        );
        case.tache
            .store((index as u64 & 0xFFFF_FFFF) | (tid << 32), Ordering::Relaxed);
        case.pid.store(
            crate::kernel::task::pid_local_pour_sonde(),
            Ordering::Relaxed,
        );
        case.syscall.store(syscall_nr, Ordering::Relaxed);
        case.aux.store(aux, Ordering::Relaxed);
        case.rsp.store(rsp_courant(), Ordering::Relaxed);
        case.kstack.store(kstack, Ordering::Relaxed);
        // En dernier, et en Release : un lecteur qui voit ce `seq` voit tout
        // le reste de la case.
        case.seq.store(seq, Ordering::Release);
    }

    /// Imprime les dernieres transitions et GELE l'anneau definitivement.
    ///
    /// Appele depuis le `panic_handler`, donc dans un contexte ou plus rien ne
    /// doit etre suppose valide : on ne lit que des atomiques et on n'appelle
    /// que la sortie serie.
    pub fn vide() {
        if GEL.swap(true, Ordering::AcqRel) {
            // Deja vide par un autre CPU : ne pas entrelacer deux vidages.
            return;
        }
        let derniere = SEQUENCE.load(Ordering::Acquire);
        if derniere == 0 {
            crate::serial_println!("[BKL-FR] anneau vide");
            return;
        }
        let premiere = derniere.saturating_sub(VIDAGE as u64 - 1).max(1);
        crate::serial_println!(
            "[BKL-FR] {} transitions (seq {}..{}), la plus recente en dernier",
            derniere - premiere + 1,
            premiere,
            derniere,
        );
        crate::serial_println!(
            "[BKL-FR] seq kind cpu owner(av->ap) depth(av->ap) garde tache tid pid syscall/phase aux rsp kstack"
        );

        for seq in premiere..=derniere {
            let case = &ANNEAU[(seq as usize) % TAILLE];
            // Une case dont le `seq` ne correspond plus a ete recyclee entre
            // notre calcul et notre lecture. On le DIT au lieu d'imprimer des
            // champs qui appartiennent a une autre transition.
            if case.seq.load(Ordering::Acquire) != seq {
                crate::serial_println!("[BKL-FR] {} <recyclee>", seq);
                continue;
            }
            let etat = case.etat.load(Ordering::Relaxed);
            let tache = case.tache.load(Ordering::Relaxed);
            let garde = ((etat >> 48) & 0xFF) as usize;
            crate::serial_println!(
                "[BKL-FR] {} {} cpu={} owner={}->{} depth={}->{} garde={} tache={} tid={} pid={} sys={}/{} aux={:#x} rsp={:#x} kstack={:#x}",
                seq,
                nom((etat & 0xFF) as u8),
                (etat >> 8) & 0xFF,
                (etat >> 16) & 0xFF,
                (etat >> 24) & 0xFF,
                (etat >> 32) & 0xFF,
                (etat >> 40) & 0xFF,
                if garde == 0xFF { -1i64 } else { garde as i64 },
                (tache & 0xFFFF_FFFF) as u32,
                tache >> 32,
                case.pid.load(Ordering::Relaxed),
                case.syscall.load(Ordering::Relaxed),
                (etat >> 56) & 0xFF,
                case.aux.load(Ordering::Relaxed),
                case.rsp.load(Ordering::Relaxed),
                case.kstack.load(Ordering::Relaxed),
            );
        }
        crate::serial_println!("[BKL-FR] fin");
    }
}

#[cfg(not(debug_assertions))]
pub mod enregistreur {
    pub const ENTER: u8 = 1;
    pub const REENTER: u8 = 2;
    pub const TRY_ENTER: u8 = 3;
    pub const GUARD_DROP: u8 = 4;
    pub const RELEASE: u8 = 5;
    pub const SUSPEND: u8 = 6;
    pub const RESUME_BEGIN: u8 = 7;
    pub const RESUME_OK: u8 = 8;
    pub const SWITCH_BEFORE: u8 = 9;
    pub const SWITCH_AFTER: u8 = 10;

    #[inline(always)]
    pub fn note(
        _kind: u8, _cpu: usize, _owner_avant: usize, _owner_apres: usize,
        _depth_avant: usize, _depth_apres: usize, _garde_cpu: usize, _aux: u64,
    ) {}

    #[inline(always)]
    pub fn vide() {
        crate::serial_println!("[BKL-FR] non compile (release)");
    }
}

/// Vide l'enregistreur de vol du gros verrou. Appele par le `panic_handler`.
pub fn vide_enregistreur() {
    enregistreur::vide();
}

/// Marque le changement de contexte lui-meme, de part et d'autre du
/// `switch_context`. Sans ces deux points, une transition BKL qui suit une
/// commutation ne peut pas etre rattachee a la pile qui l'a produite.
pub fn note_switch(avant: bool, from: usize, to: usize) {
    let cpu = cpu();
    let owner = OWNER.load(Ordering::Relaxed);
    let depth = DEPTH[cpu].load(Ordering::Relaxed);
    let kind = if avant {
        enregistreur::SWITCH_BEFORE
    } else {
        enregistreur::SWITCH_AFTER
    };
    enregistreur::note(
        kind,
        cpu,
        owner,
        owner,
        depth,
        depth,
        usize::MAX,
        ((from as u64 & 0xFFFF_FFFF) << 32) | (to as u64 & 0xFFFF_FFFF),
    );
}
