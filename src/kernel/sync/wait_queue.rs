//! Event-driven task wait queue with generation-based lost-wakeup avoidance.

use core::sync::atomic::{AtomicU64, Ordering};

static WAITQ_BKL_ENTERS: AtomicU64 = AtomicU64::new(0);
static WAITQ_BKL_WAIT_NS: AtomicU64 = AtomicU64::new(0);
/// Notifications servies sans prendre le gros verrou, faute de dormeur.
///
/// Sans ce compteur, une baisse de `waitq_bkl_enters` serait ambigue : elle
/// pourrait signifier « on evite le verrou » comme « plus rien ne se reveille ».
/// La premiere est le but recherche, la seconde une panne. Les deux se
/// distinguent en regardant si ce compteur-ci monte d'autant.
static WAITQ_WAKE_SANS_VERROU: AtomicU64 = AtomicU64::new(0);

fn enter_bkl() -> crate::kernel::smp_lock::KernelGuard {
    let start = crate::kernel::timer::monotonic_ns();
    let guard = crate::kernel::smp_lock::enter();
    WAITQ_BKL_ENTERS.fetch_add(1, Ordering::Relaxed);
    WAITQ_BKL_WAIT_NS.fetch_add(
        crate::kernel::timer::monotonic_ns().saturating_sub(start),
        Ordering::Relaxed,
    );
    guard
}

pub fn bkl_stats() -> (u64, u64) {
    (
        WAITQ_BKL_ENTERS.load(Ordering::Relaxed),
        WAITQ_BKL_WAIT_NS.load(Ordering::Relaxed),
    )
}

/// Notifications qui n'ont pris ni verrou ni parcours de la table des taches.
pub fn wake_sans_verrou() -> u64 {
    WAITQ_WAKE_SANS_VERROU.load(Ordering::Relaxed)
}

/// A ticket is sampled while the caller still protects its resource condition.
#[derive(Clone, Copy)]
pub struct WaitTicket(u64);

pub struct WaitQueue {
    generation: AtomicU64,
    // BOUCHAUD_P1_WAITQ_WAITER_HINT_V1
    //
    // Nombre de taches reellement endormies sur cette queue. C'est un
    // RACCOURCI, pas la verite : la generation reste seule responsable de la
    // correction. Ce compteur ne sert qu'a repondre « personne n'attend » sans
    // prendre le gros verrou.
    //
    // Ce que cela evite : `wake_wait_queue` balaie TOUTE la table des taches,
    // sous BKL, a chaque notification. La queue de readiness globale
    // (`object::fd::READINESS`) est notifiee a chaque changement d'etat de
    // descripteur -- paquet reseau, ecriture de tube, minuteur. Sous Ladybird
    // l'immense majorite de ces balayages ne trouvait personne, et payait
    // quand meme le verrou et le parcours.
    //
    // En cas de doute, il penche du bon cote. Si une tache endormie disparait
    // sans rendre son inscription (tuee pendant son sommeil), le compteur
    // reste haut : le raccourci cesse de se declencher et on retombe
    // exactement sur le comportement d'avant. Jamais l'inverse.
    waiters: AtomicU64,
}

/// Inscription d'une tache sur la queue, retiree quoi qu'il arrive.
///
/// Le retrait passe par `Drop` parce que `park_current_on` a plusieurs sorties
/// -- notification, echeance -- et qu'un compteur decremente sur une seule
/// d'entre elles se serait desynchronise en silence.
struct Inscription<'a> {
    queue: &'a WaitQueue,
}

impl<'a> Inscription<'a> {
    /// A appeler le gros verrou EN MAIN : le retrait se fait sous le meme
    /// verrou que le balayage, donc jamais pendant qu'on le lit.
    fn nouvelle(queue: &'a WaitQueue) -> Self {
        queue.waiters.fetch_add(1, Ordering::SeqCst);
        Self { queue }
    }
}

impl Drop for Inscription<'_> {
    fn drop(&mut self) {
        self.queue.waiters.fetch_sub(1, Ordering::SeqCst);
    }
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            generation: AtomicU64::new(1),
            waiters: AtomicU64::new(0),
        }
    }

    /// Arm an upcoming wait before releasing the resource lock.
    pub fn ticket(&self) -> WaitTicket {
        WaitTicket(self.generation.load(Ordering::Acquire))
    }

    /// Sleep only if no producer has signalled since `ticket()`.
    ///
    /// # Aucun reveil perdu
    ///
    /// Le dormeur s'inscrit AVANT de relire la generation ; le notifiant
    /// incremente la generation AVANT de lire les inscriptions :
    ///
    /// ```text
    ///     dormeur                        notifiant
    ///     waiters += 1     (SeqCst)      generation += 1  (SeqCst)
    ///     lire generation  (SeqCst)      lire waiters     (SeqCst)
    /// ```
    ///
    /// Pour que le reveil se perde il faudrait que le dormeur ne voie pas la
    /// nouvelle generation ET que le notifiant ne voie pas l'inscription.
    /// L'ordre total des quatre acces `SeqCst` l'interdit : c'est le meme motif
    /// de tampon d'ecriture que le parking du BKL, et il exige le meme ordre --
    /// un `Release`/`Acquire` laisserait precisement passer ce cas.
    pub fn wait(&self, ticket: WaitTicket) {
        // Notification deja arrivee : ni verrou, ni compteur, ni parcours.
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            return;
        }
        let _kernel = enter_bkl();
        let _inscrit = Inscription::nouvelle(self);
        if self.generation.load(Ordering::SeqCst) != ticket.0 {
            return;
        }
        crate::kernel::task::park_current_on(self.key());
    }

    /// Attend une notification, mais jamais au-dela de `deadline_ns`.
    /// Rend `true` si une notification a precede l'echeance.
    pub fn wait_until(&self, ticket: WaitTicket, deadline_ns: u64) -> bool {
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            return true;
        }
        let _kernel = enter_bkl();
        let _inscrit = Inscription::nouvelle(self);
        if self.generation.load(Ordering::SeqCst) != ticket.0 {
            return true;
        }
        crate::kernel::task::park_current_on_until(self.key(), deadline_ns)
    }

    pub fn wake_one(&self) -> bool {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) == 0 {
            WAITQ_WAKE_SANS_VERROU.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let _kernel = enter_bkl();
        crate::kernel::task::wake_wait_queue(self.key(), 1) != 0
    }

    pub fn wake_all(&self) -> usize {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) == 0 {
            WAITQ_WAKE_SANS_VERROU.fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        let _kernel = enter_bkl();
        crate::kernel::task::wake_wait_queue(self.key(), usize::MAX)
    }

    fn key(&self) -> usize {
        self as *const Self as usize
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}
