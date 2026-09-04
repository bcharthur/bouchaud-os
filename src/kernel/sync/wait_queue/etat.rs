// State and counters.

static WAITQ_BKL_ENTERS: AtomicU64 = AtomicU64::new(0);
static WAITQ_BKL_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static WAITQ_WAKE_SANS_VERROU: AtomicU64 = AtomicU64::new(0);

static WAITQ_DETACHED_WAITS: AtomicU64 = AtomicU64::new(0);
static WAITQ_LEGACY_WAITS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_WAIT_MAX_NS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_SCHEDULE_LOOPS: AtomicU64 = AtomicU64::new(0);
static WAITQ_DETACHED_BKL_RETURN_VIOLATIONS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn waitq_update_max(atom: &AtomicU64, value: u64) {
    let mut old = atom.load(Ordering::Relaxed);
    while value > old {
        match atom.compare_exchange_weak(old, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(now) => old = now,
        }
    }
}

// BOUCHAUD_C1_READINESS_SANS_REPRISE_V1
//
// CE QUE CETTE FONCTION PRENAIT, ET POURQUOI ELLE N'AVAIT PAS A LE PRENDRE
//
// Elle faisait `smp_lock::enter()` -- la DERNIERE acquisition du gros verrou
// attribuee au domaine `Readiness`. Elle n'etait appelee que depuis la branche
// LEGACY de `wait`/`wait_until`, c'est-a-dire exactement quand
// `profondeur_locale() != 0` : l'appelant TENAIT deja le verrou.
//
// Le gros verrou est reentrant et appartient au CPU : reprendre un verrou que
// ce CPU detient deja n'ajoute aucune exclusion. Cela ajoutait un compteur, une
// portee de domaine, deux lectures d'horloge et une paire enter/Drop -- par
// attente bloquante, sur le chemin de blocage d'un appel systeme.
//
// Ce qui restait UTILE est la mesure : combien d'attentes entrent encore avec
// un gros verrou HERITE de leur appelant. C'est la dette reelle du domaine, et
// c'est ce que compte maintenant cette fonction. `WAITQ_BKL_WAIT_NS` n'est plus
// alimente : il mesurait l'attente AVANT une acquisition qui n'a plus lieu, et
// le voir tomber a zero dans `[MM-NG6] waitq_bkl_wait_ns=` est precisement la
// mesure du gain.
#[inline]
fn note_attente_sous_bkl_herite() {
    debug_assert!(
        crate::kernel::smp_lock::profondeur_locale() != 0,
        "wait_queue: branche legacy sans gros verrou herite -- le parking \
         suppose que l'appelant le tient deja",
    );
    WAITQ_BKL_ENTERS.fetch_add(1, Ordering::Relaxed);
}

#[derive(Clone, Copy)]
pub struct WaitTicket(u64);

pub struct WaitQueue {
    /// Le protocole de parking vit dans `sync::rendezvous`, et c'est LE MEME
    /// code que `tools/smp/test_rendezvous.rs` met a l'epreuve. Le dupliquer
    /// ici rendrait le test decoratif : il prouverait une copie.
    point: crate::kernel::sync::rendezvous::Rendezvous,
}

struct Inscription<'a> {
    queue: &'a WaitQueue,
}

impl<'a> Inscription<'a> {
    fn nouvelle(queue: &'a WaitQueue) -> Self {
        queue.point.inscrit();
        Self { queue }
    }
}

impl Drop for Inscription<'_> {
    fn drop(&mut self) {
        self.queue.point.desinscrit();
    }
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self { point: crate::kernel::sync::rendezvous::Rendezvous::neuf() }
    }

    #[inline]
    fn key(&self) -> usize {
        self as *const Self as usize
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}
