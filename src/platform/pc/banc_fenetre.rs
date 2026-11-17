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
//! `tools/ci/run_fenetre_migration.sh` exige `fenetre_preempt=0
//! fenetre_verrou=0`, aucune panique, et des MIGRATIONS observees (sans elles,
//! le banc n'aurait rien prouve). Compile seulement sous
//! `banc-fenetre-migration` : jamais livre.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::kernel::sync::SpinLock;

const FILS: usize = 4;
const DUREE_S: u64 = 30;
/// Tours d'attente active dans la fenetre (quelques microsecondes sous TCG).
const PAUSE_TOURS: u32 = 4000;

static VERROU: SpinLock<u64> = SpinLock::new(0);
static TACHES: [AtomicUsize; FILS] = [const { AtomicUsize::new(usize::MAX) }; FILS];
static ACTIF: AtomicBool = AtomicBool::new(false);
static RANG: AtomicUsize = AtomicUsize::new(0);
static FINIS: AtomicUsize = AtomicUsize::new(0);
static TOURS: AtomicU64 = AtomicU64::new(0);
static MIGRATIONS: AtomicU64 = AtomicU64::new(0);
static PAUSES: AtomicU64 = AtomicU64::new(0);

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
    for _ in 0..PAUSE_TOURS {
        core::hint::spin_loop();
    }
}

fn fil() -> ! {
    let rang = RANG.fetch_add(1, Ordering::AcqRel);
    if rang < FILS {
        TACHES[rang].store(crate::kernel::task::indice_tache_courante(), Ordering::Release);
    }
    let fin = crate::kernel::timer::ticks() + DUREE_S * crate::kernel::timer::TICKS_PER_SECOND;
    let coeurs = crate::arch::x86_64::smp::schedulable_cpus().max(1);
    let mut coeur = crate::arch::x86_64::smp::cpu_index();
    let mut cible = rang;
    while crate::kernel::timer::ticks() < fin {
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
    }
    if rang < FILS {
        TACHES[rang].store(usize::MAX, Ordering::Release);
    }
    FINIS.fetch_add(1, Ordering::AcqRel);
    loop {
        crate::kernel::task::sleep_ticks(crate::kernel::timer::TICKS_PER_SECOND);
    }
}

fn bilan() -> ! {
    let limite = crate::kernel::timer::ticks() + (DUREE_S + 60) * crate::kernel::timer::TICKS_PER_SECOND;
    while FINIS.load(Ordering::Acquire) < FILS && crate::kernel::timer::ticks() < limite {
        crate::kernel::task::sleep_ticks(crate::kernel::timer::TICKS_PER_SECOND / 10);
    }
    ACTIF.store(false, Ordering::Release);
    let (fenetre_preempt, fenetre_verrou) = crate::kernel::scheduler::preempt::fenetres_migrees();
    crate::serial_println!(
        "BOUCHAUD_BANC_FENETRE fin fils_finis={}/{} tours={} pauses={} migrations={} preemptions_noyau={} fenetre_preempt={} fenetre_verrou={}",
        FINIS.load(Ordering::Acquire),
        FILS,
        TOURS.load(Ordering::Relaxed),
        PAUSES.load(Ordering::Relaxed),
        MIGRATIONS.load(Ordering::Relaxed),
        crate::kernel::scheduler::preempt::stats_reveil().preemptions_noyau,
        fenetre_preempt,
        fenetre_verrou,
    );
    loop {
        crate::kernel::task::sleep_ticks(crate::kernel::timer::TICKS_PER_SECOND);
    }
}

pub fn demarre() {
    crate::serial_println!("BOUCHAUD_BANC_FENETRE armement fils={} duree_s={}", FILS, DUREE_S);
    ACTIF.store(true, Ordering::Release);
    for _ in 0..FILS {
        if !crate::kernel::task::spawn_noyau(fil, "banc-fenetre") {
            crate::serial_println!("BOUCHAUD_BANC_FENETRE_ECHEC spawn");
            return;
        }
    }
    if !crate::kernel::task::spawn_noyau(bilan, "banc-fenetre-bilan") {
        crate::serial_println!("BOUCHAUD_BANC_FENETRE_ECHEC spawn_bilan");
    }
}
