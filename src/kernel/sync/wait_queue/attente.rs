// Waiting side.
//
// BOUCHAUD_C1_ATTENTE_SANS_GROS_VERROU_V1
//
// LE PARKING SE PUBLIE AVANT LA DERNIERE RELECTURE
// ================================================
// L'ordre etait : inscrire -> relire la generation -> se declarer bloque.
// Il n'etait correct que parce que le gros verrou empechait tout reveilleur de
// tourner entre les deux derniers pas. Sans lui, la fenetre est reelle :
//
//   dormeur      relit la generation, ne voit rien de neuf
//   reveilleur   incremente la generation, voit waiters > 0,
//                tente la transition Bloque -> Pret : elle ECHOUE,
//                puisque le dormeur est encore Pret
//   dormeur      se declare bloque, et dort pour toujours
//
// L'ordre est donc inverse : on publie `Blocked` D'ABORD, on relit la
// generation ENSUITE, et on annule le parking si un reveil est deja passe.
//
//   dormeur      inscrit (waiters++), publie Blocked, relit la generation
//   reveilleur   incremente la generation, lit waiters, tente la transition
//
// Les deux cotes font une ecriture puis une lecture croisees, toutes deux en
// ordre sequentiel : au moins l'un voit l'autre. Si le dormeur voit la
// nouvelle generation, il annule et repart ; sinon le reveilleur voit
// `waiters > 0` ET un etat `Blocked`, et sa transition reussit. Aucun des deux
// chemins ne perd le reveil.
//
// Chemin a profondeur 0 :
// ticket -> inscrire -> Blocked -> relecture generation -> schedule -> retour.

// BOUCHAUD_CYCLE_DE_VIE_V1 : INTERRUPTIBLE PAR DEFAUT
// ==================================================
// `wait` / `wait_until` sont les attentes des appels systeme : descripteurs,
// futex, IPC native. Une tache condamnee n'y dort pas -- elle y meurt, comme
// un tueur l'y tuerait si elle etait deja parquee. Les attentes qui tiennent
// une ressource du noyau (verrou dormant, page en cours de chargement,
// controleur de disque, faute en cours) prennent `wait_noyau` /
// `wait_until_noyau` : une tache condamnee les termine normalement, puis
// meurt a sa frontiere. Voir `kernel::cycle_vie`.

impl WaitQueue {
    pub fn wait(&self, ticket: WaitTicket) {
        self.attend(ticket, None, true);
    }

    pub fn wait_until(&self, ticket: WaitTicket, deadline_ns: u64) -> bool {
        self.attend(ticket, Some(deadline_ns), true)
    }

    /// Attente non interruptible : l'appelant tient une ressource du noyau.
    pub fn wait_noyau(&self, ticket: WaitTicket) {
        self.attend(ticket, None, false);
    }

    /// Attente bornee non interruptible.
    pub fn wait_until_noyau(&self, ticket: WaitTicket, deadline_ns: u64) -> bool {
        self.attend(ticket, Some(deadline_ns), false)
    }

    fn attend(&self, ticket: WaitTicket, deadline_ns: Option<u64>, interruptible: bool) -> bool {
        if self.point.ticket() != ticket.0 {
            return true;
        }

        // Un seul chemin depuis que le gros verrou n'existe plus
        // (BOUCHAUD_BKL_SUPPRIME_V1) : la branche LEGACY servait les appelants
        // qui le tenaient en entrant.
        let inscrit = Inscription::nouvelle(self);
        WAITQ_DETACHED_WAITS.fetch_add(1, Ordering::Relaxed);
        let start = crate::kernel::timer::monotonic_ns();

        match crate::kernel::task::prepare_park_current_on_detached(
            self.key(),
            deadline_ns,
            interruptible,
        ) {
            crate::kernel::task::Parking::Gare => {}
            // La tache n'etait plus prete : elle ne dort pas.
            crate::kernel::task::Parking::Refuse => return true,
            crate::kernel::task::Parking::Condamnee => {
                // Rendre l'inscription AVANT de mourir : la pile ne sera plus
                // jamais deroulee, et un compte d'attentes perime ferait
                // balayer chaque reveil pour rien.
                drop(inscrit);
                crate::kernel::task::meurt_au_parking();
            }
        }

        // La relecture vient APRES la publication du parking : c'est ce qui
        // ferme la fenetre du reveil perdu.
        if self.point.ticket() != ticket.0 {
            crate::kernel::task::annule_park_courant();
            return true;
        }

        let (notified, loops) =
            crate::kernel::task::finish_park_current_on_detached(deadline_ns);
        WAITQ_DETACHED_SCHEDULE_LOOPS.fetch_add(loops, Ordering::Relaxed);

        let elapsed = crate::kernel::timer::monotonic_ns().saturating_sub(start);
        WAITQ_DETACHED_WAIT_NS.fetch_add(elapsed, Ordering::Relaxed);
        waitq_update_max(&WAITQ_DETACHED_WAIT_MAX_NS, elapsed);
        drop(inscrit);
        notified
    }
}
