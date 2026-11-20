//! BOUCHAUD_FENETRE_MIGRATION_V1 (#283) -- temoin de la fenetre de migration.
//!
//! `preempt::disable()` et `SpinLock::lock()` posent une marque PAR COEUR
//! (`preempt_count`, `verrous_simples`) en deux temps : lire l'indice du
//! coeur, puis incrementer son compteur. Un fil noyau est preemptible sous IRQ
//! (preemption ciblee) tant que la marque n'est pas posee. Commute entre les
//! deux temps, il reprenait sur un autre coeur et marquait celui qu'il avait
//! quitte :
//!
//!   * `preempt_count` : le coeur quitte restait a +1 pour toujours (plus
//!     aucune preemption), le nouveau passait sous zero au `enable()` --
//!     `cpu_local: preempt_count underflow`, Ladybird perf avant-simple du
//!     15/11 (main@65f1e65, coeur 3) ;
//!   * `verrous_simples` : le fil prenait le verrou sur un coeur NON marque,
//!     donc preemptible verrou tenu -- l'interblocage que la marque existe
//!     pour empecher.
//!
//! Ce temoin exerce les VRAIS chemins (`preempt::disable/enable`,
//! `SpinLock::lock`) depuis des fils noyau, sous demandes de preemption
//! ciblees vers tous les coeurs, et elargit la fenetre -- pour ces seuls fils
//! -- par une attente active placee entre la lecture du coeur et la marque.
//! Corrige, cette attente tombe dans la section a interruptions coupees et ne
//! peut plus etre interrompue ; non corrige, elle laisse passer l'IRQ.
//!
//! `tools/ci/run_fenetre_migration.sh` exige `fenetres_ouvertes=0` (chaque
//! fenetre traversee interruptions masquees : rien ne peut y commuter le fil),
//! `fenetre_preempt=0 fenetre_verrou=0`, des preemptions noyau accordees et
//! aucune panique. Les migrations sont publiees. Temoin negatif (fenetre
//! rouverte, meme banc) : `fenetres_ouvertes` > 0, et en SMP4 la panique
//! `preempt_count underflow`. Compile seulement sous `banc-fenetre-migration` :
//! jamais livre.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::kernel::sync::SpinLock;

/// Plafond des fils du temoin. Le banc en lance UN DE PLUS que de coeurs
/// (borne a ce plafond) : il reste ainsi toujours une tache en attente dans
/// une file, condition du vol de travail.
const FILS_MAX: usize = 9;
const DUREE_S: u64 = 30;
/// Un lot sur `SOMMEIL_TOUS_LES` se termine par un court sommeil : un coeur
/// se libere et peut voler la tache qui attend ailleurs. Les migrations
/// restent un HASARD de l'ordonnanceur (integration 44040309462 : SMP2 0,
/// SMP4 1 ; local : SMP4 0 ou 2 par run, SMP2 toujours 0) : elles sont
/// publiees, pas exigees. Ce qui est exige est `fenetres_ouvertes=0`.
const SOMMEIL_TOUS_LES: u64 = 8;
/// Tours d'attente active dans la fenetre (quelques microsecondes sous TCG).
const PAUSE_TOURS: u32 = 4000;

static VERROU: SpinLock<u64> = SpinLock::new(0);
static TACHES: [AtomicUsize; FILS_MAX] = [const { AtomicUsize::new(usize::MAX) }; FILS_MAX];
static FILS: AtomicUsize = AtomicUsize::new(0);
static ACTIF: AtomicBool = AtomicBool::new(false);
static RANG: AtomicUsize = AtomicUsize::new(0);
static FINIS: AtomicUsize = AtomicUsize::new(0);
static TOURS: AtomicU64 = AtomicU64::new(0);
/// Migrations PENDANT un lot : le fil a ete preempte puis repris ailleurs
/// sans jamais s'endormir -- exactement le chemin du defaut.
static MIGRATIONS: AtomicU64 = AtomicU64::new(0);
/// Migrations a travers un sommeil (placement au reveil) : comptees a part,
/// elles ne prouvent rien sur la fenetre.
static MIGRATIONS_SOMMEIL: AtomicU64 = AtomicU64::new(0);
static PAUSES: AtomicU64 = AtomicU64::new(0);
/// Fenetres traversees INTERRUPTIONS OUVERTES. Seule une IRQ peut commuter un
/// fil noyau, et rien ne bloque entre la lecture du coeur et la marque : la
/// fenetre est fermee si et seulement si les interruptions y sont masquees.
/// C'est l'invariant de BOUCHAUD_FENETRE_MIGRATION_V1, et il se verifie a
/// CHAQUE fenetre, sans attendre qu'une migration tombe au bon moment : nul
/// sur le code corrige, non nul sur l'ancien (une partie des pauses tombe
/// deja sous un verrou tenu, donc interruptions masquees).
static FENETRES_OUVERTES: AtomicU64 = AtomicU64::new(0);

/// Appele entre la lecture du coeur et la pose de la marque. N'attend que
/// pour les fils du temoin : le reste du noyau ne voit qu'un test.
#[inline]
pub fn pause() {
    if !ACTIF.load(Ordering::Relaxed) {
        return;
    }
    let moi = crate::kernel::task::indice_tache_courante();
    if !TACHES.iter().any(|t| t.load(Ordering::Relaxed) == moi) {
        return;
    }
    PAUSES.fetch_add(1, Ordering::Relaxed);
    if x86_64::instructions::interrupts::are_enabled() {
        FENETRES_OUVERTES.fetch_add(1, Ordering::Relaxed);
    }
    for _ in 0..PAUSE_TOURS {
        core::hint::spin_loop();
    }
}

fn fil() -> ! {
    let rang = RANG.fetch_add(1, Ordering::AcqRel);
    if rang < FILS_MAX {
        TACHES[rang].store(crate::kernel::task::indice_tache_courante(), Ordering::Release);
    }
    let fin = crate::kernel::timer::ticks() + DUREE_S * crate::kernel::timer::TICKS_PER_SECOND;
    let coeurs = crate::arch::x86_64::smp::schedulable_cpus().max(1);
    let mut coeur = crate::arch::x86_64::smp::cpu_index();
    let mut cible = rang;
    let mut lot = 0u64;
    while crate::kernel::timer::ticks() < fin {
        lot += 1;
        for _ in 0..16 {
            crate::kernel::scheduler::preempt::disable();
            crate::kernel::scheduler::preempt::enable();
            let mut garde = VERROU.lock();
            *garde += 1;
            drop(garde);
            TOURS.fetch_add(1, Ordering::Relaxed);
        }
        let ici = crate::arch::x86_64::smp::cpu_index();
        if ici != coeur {
            MIGRATIONS.fetch_add(1, Ordering::Relaxed);
            coeur = ici;
        }
        // Une demande ciblee vers chaque coeur a tour de role : c'est elle
        // qui autorise la preemption d'un fil noyau sous IRQ.
        cible = (cible + 1) % coeurs;
        crate::kernel::scheduler::preempt::demande_ciblee(cible);
        if lot % SOMMEIL_TOUS_LES == (rang as u64) % SOMMEIL_TOUS_LES {
            // Une demande ne livre pas elle-même d'IRQ. En SMP2, les tirs
            // périodiques peuvent tous tomber dans les fenêtres protégées.
            // Exercer aussi l'IRQ réelle ici, hors verrou, avant de dormir.
            use crate::arch::x86_64::smp;
            use crate::kernel::scheduler::preempt;
            preempt::demande_ciblee(smp::cpu_index());
            smp::ipi_vers_soi(smp::RESCHEDULE_VECTOR);
            let limite_irq = crate::kernel::timer::monotonic_ns().saturating_add(1_000_000);
            while preempt::demande_ciblee_pendante()
                && crate::kernel::timer::monotonic_ns() < limite_irq
            {
                core::hint::spin_loop();
            }
            crate::kernel::task::sleep_ticks(1);
            let reveil = crate::arch::x86_64::smp::cpu_index();
            if reveil != coeur {
                MIGRATIONS_SOMMEIL.fetch_add(1, Ordering::Relaxed);
                coeur = reveil;
            }
        }
    }
    if rang < FILS_MAX {
        TACHES[rang].store(usize::MAX, Ordering::Release);
    }
    FINIS.fetch_add(1, Ordering::AcqRel);
    loop {
        crate::kernel::task::sleep_ticks(crate::kernel::timer::TICKS_PER_SECOND);
    }
}

fn bilan() -> ! {
    let limite = crate::kernel::timer::ticks() + (DUREE_S + 60) * crate::kernel::timer::TICKS_PER_SECOND;
    let fils = FILS.load(Ordering::Acquire);
    while FINIS.load(Ordering::Acquire) < fils && crate::kernel::timer::ticks() < limite {
        crate::kernel::task::sleep_ticks(crate::kernel::timer::TICKS_PER_SECOND / 10);
    }
    ACTIF.store(false, Ordering::Release);
    let (fenetre_preempt, fenetre_verrou) = crate::kernel::scheduler::preempt::fenetres_migrees();
    crate::serial_println!(
        "BOUCHAUD_BANC_FENETRE fin fils_finis={}/{} tours={} pauses={} fenetres_ouvertes={} migrations={} migrations_sommeil={} preemptions_noyau={} fenetre_preempt={} fenetre_verrou={}",
        FINIS.load(Ordering::Acquire),
        fils,
        TOURS.load(Ordering::Relaxed),
        PAUSES.load(Ordering::Relaxed),
        FENETRES_OUVERTES.load(Ordering::Relaxed),
        MIGRATIONS.load(Ordering::Relaxed),
        MIGRATIONS_SOMMEIL.load(Ordering::Relaxed),
        crate::kernel::scheduler::preempt::stats_reveil().preemptions_noyau,
        fenetre_preempt,
        fenetre_verrou,
    );
    loop {
        crate::kernel::task::sleep_ticks(crate::kernel::timer::TICKS_PER_SECOND);
    }
}

pub fn demarre() {
    let fils = (crate::arch::x86_64::smp::schedulable_cpus().max(1) + 1).min(FILS_MAX);
    FILS.store(fils, Ordering::Release);
    crate::serial_println!("BOUCHAUD_BANC_FENETRE armement fils={} duree_s={}", fils, DUREE_S);
    ACTIF.store(true, Ordering::Release);
    // Chaque fil NAIT sur un coeur different (le dernier double le premier) :
    // lances par `spawn_noyau`, tous naissaient sur la file du coeur 0
    // (`[SMP-TASK] rq=0`) et n'en bougeaient plus -- aucune migration en
    // SMP2, une seule en SMP4 (integration 44040309462).
    let coeurs = crate::arch::x86_64::smp::schedulable_cpus().max(1);
    for i in 0..fils {
        if !crate::kernel::task::spawn_noyau_migrable_sur(fil, "banc-fenetre", i % coeurs) {
            crate::serial_println!("BOUCHAUD_BANC_FENETRE_ECHEC spawn");
            return;
        }
    }
    if !crate::kernel::task::spawn_noyau(bilan, "banc-fenetre-bilan") {
        crate::serial_println!("BOUCHAUD_BANC_FENETRE_ECHEC spawn_bilan");
    }
}
