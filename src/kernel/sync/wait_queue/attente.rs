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

impl WaitQueue {
    pub fn wait(&self, ticket: WaitTicket) {
        if self.point.ticket() != ticket.0 {
            return;
        }

        let profondeur_avant = crate::kernel::smp_lock::profondeur_locale();

        if profondeur_avant == 0 {
            // Chemin detache : plus aucun gros verrou.
            let _inscrit = Inscription::nouvelle(self);
            WAITQ_DETACHED_WAITS.fetch_add(1, Ordering::Relaxed);
            let start = crate::kernel::timer::monotonic_ns();

            crate::kernel::task::prepare_park_current_on_detached(self.key(), None);

            // La relecture vient APRES la publication du parking : c'est ce qui
            // ferme la fenetre du reveil perdu.
            if self.point.ticket() != ticket.0 {
                crate::kernel::task::annule_park_courant();
                return;
            }

            let (_, loops) = crate::kernel::task::finish_park_current_on_detached(None);
            WAITQ_DETACHED_SCHEDULE_LOOPS.fetch_add(loops, Ordering::Relaxed);

            let elapsed = crate::kernel::timer::monotonic_ns().saturating_sub(start);
            WAITQ_DETACHED_WAIT_NS.fetch_add(elapsed, Ordering::Relaxed);
            waitq_update_max(&WAITQ_DETACHED_WAIT_MAX_NS, elapsed);

            if crate::kernel::smp_lock::profondeur_locale() != 0 {
                WAITQ_DETACHED_BKL_RETURN_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }

        // Chemin LEGACY : l'appelant tenait deja le gros verrou en entrant.
        // On le conserve tel quel, avec l'ancien ordre -- il est correct sous
        // verrou, et le migrer demande de migrer d'abord ses appelants.
        let _kernel = enter_bkl();
        let _inscrit = Inscription::nouvelle(self);
        if self.point.ticket() != ticket.0 {
            return;
        }
        WAITQ_LEGACY_WAITS.fetch_add(1, Ordering::Relaxed);
        crate::kernel::task::park_current_on(self.key());
    }

    pub fn wait_until(&self, ticket: WaitTicket, deadline_ns: u64) -> bool {
        if self.point.ticket() != ticket.0 {
            return true;
        }

        let profondeur_avant = crate::kernel::smp_lock::profondeur_locale();

        if profondeur_avant == 0 {
            let _inscrit = Inscription::nouvelle(self);
            WAITQ_DETACHED_WAITS.fetch_add(1, Ordering::Relaxed);
            let start = crate::kernel::timer::monotonic_ns();

            crate::kernel::task::prepare_park_current_on_detached(
                self.key(),
                Some(deadline_ns),
            );

            if self.point.ticket() != ticket.0 {
                crate::kernel::task::annule_park_courant();
                return true;
            }

            let (notified, loops) =
                crate::kernel::task::finish_park_current_on_detached(Some(deadline_ns));
            WAITQ_DETACHED_SCHEDULE_LOOPS.fetch_add(loops, Ordering::Relaxed);

            let elapsed = crate::kernel::timer::monotonic_ns().saturating_sub(start);
            WAITQ_DETACHED_WAIT_NS.fetch_add(elapsed, Ordering::Relaxed);
            waitq_update_max(&WAITQ_DETACHED_WAIT_MAX_NS, elapsed);

            if crate::kernel::smp_lock::profondeur_locale() != 0 {
                WAITQ_DETACHED_BKL_RETURN_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
            }
            return notified;
        }

        let _kernel = enter_bkl();
        let _inscrit = Inscription::nouvelle(self);
        if self.point.ticket() != ticket.0 {
            return true;
        }
        WAITQ_LEGACY_WAITS.fetch_add(1, Ordering::Relaxed);
        crate::kernel::task::park_current_on_until(self.key(), deadline_ns)
    }
}
