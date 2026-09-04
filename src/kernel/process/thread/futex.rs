// --- Wait-word compatibility bridge -----------------------------------------
//
// The task subsystem no longer owns futex state. Linux/POSIX calls still use
// the historical `task::futex_*` API, but the real mechanism is the Bouchaud
// native wait-word core in `kernel::sync::wait_word`.
//
// BOUCHAUD_C1_FUTEX_MESURE_V1
//
// LE DOMAINE `FUTEX` N'ETAIT MESURE PAR RIEN
//
// `Domaine::Futex` existait dans la table des contrats, portait
// `Contrat::EnMigration`, et n'etait ouvert NULLE PART dans le noyau. Son
// compteur d'acquisitions valait donc zero -- non pas parce que le chemin
// etait sorti du gros verrou, mais parce que personne ne regardait. Un zero
// qu'aucune mesure ne peut faire monter ne prouve rien, et le declarer `Migre`
// aurait ete une decoration.
//
// Ce qui rend le domaine mesurable n'est pas une portee -- elle enjamberait
// l'attente bloquante, donc une COMMUTATION, et la pile de domaines est par CPU
// (voir `tools/verifie-portee-sans-commutation.py`). Ce sont des compteurs.
//
// La dette reelle est comptee. Ces deux fonctions SUSPENDENT un gros verrou
// qu'elles n'ont pas pris : c'est l'appelant -- la table de politique des
// appels systeme Linux -- qui le tenait. `[BKL-FUTEX] herites=` dit combien
// d'operations arrivent encore ainsi. Tant que ce chiffre n'est pas nul, le
// futex n'est pas sorti du gros verrou : il en herite un, et le contrat reste
// honnetement `EnMigration`.

use core::sync::atomic::Ordering as OrdreFutex;

static FUTEX_ATTENTES: AtomicU64 = AtomicU64::new(0);
static FUTEX_REVEILS: AtomicU64 = AtomicU64::new(0);
/// Operations entrees avec un gros verrou HERITE de leur appelant.
static FUTEX_BKL_HERITES: AtomicU64 = AtomicU64::new(0);
/// Profondeur heritee maximale observee.
static FUTEX_BKL_PROFONDEUR_MAX: AtomicU64 = AtomicU64::new(0);

#[inline]
fn note_heritage(profondeur: usize) {
    if profondeur != 0 {
        FUTEX_BKL_HERITES.fetch_add(1, OrdreFutex::Relaxed);
        FUTEX_BKL_PROFONDEUR_MAX.fetch_max(profondeur as u64, OrdreFutex::Relaxed);
    }
}

/// attentes, reveils, operations sous verrou herite, profondeur heritee max.
pub fn futex_bkl_stats() -> (u64, u64, u64, u64) {
    (
        FUTEX_ATTENTES.load(OrdreFutex::Relaxed),
        FUTEX_REVEILS.load(OrdreFutex::Relaxed),
        FUTEX_BKL_HERITES.load(OrdreFutex::Relaxed),
        FUTEX_BKL_PROFONDEUR_MAX.load(OrdreFutex::Relaxed),
    )
}

pub fn futex_wait(uaddr: u64, expected: u32, timeout_ms: u64) -> bool {
    FUTEX_ATTENTES.fetch_add(1, OrdreFutex::Relaxed);
    // The Linux syscall may still enter through the conservative outer-BKL
    // policy. Explicitly suspend it for the native wait. V13 can therefore be
    // benchmarked without weakening the default syscall safety table globally.
    let depth = smp_lock::suspend_for_schedule();
    note_heritage(depth);
    let result = crate::kernel::sync::wait_word_wait(uaddr, expected, timeout_ms);
    smp_lock::resume_after_schedule(depth);
    matches!(
        result,
        crate::kernel::sync::WaitWordWake::Signaled
            | crate::kernel::sync::WaitWordWake::ValueChanged
    )
}

pub fn futex_wake(uaddr: u64, count: u32) -> u32 {
    FUTEX_REVEILS.fetch_add(1, OrdreFutex::Relaxed);
    let depth = smp_lock::suspend_for_schedule();
    note_heritage(depth);
    let result = crate::kernel::sync::wait_word_wake(uaddr, count);
    smp_lock::resume_after_schedule(depth);
    result
}
