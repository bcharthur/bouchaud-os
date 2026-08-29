// Waiting side.
//
// Depth-0 path:
// ticket -> BKL -> register -> generation recheck -> Blocked -> DROP BKL
// -> schedule at depth 0 -> short cleanup BKL -> return at depth 0.

impl WaitQueue {
    pub fn wait(&self, ticket: WaitTicket) {
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            return;
        }

        let profondeur_avant = crate::kernel::smp_lock::profondeur_locale();
        let kernel = enter_bkl();
        let _inscrit = Inscription::nouvelle(self);

        if self.generation.load(Ordering::SeqCst) != ticket.0 {
            return;
        }

        if profondeur_avant == 0 {
            WAITQ_DETACHED_WAITS.fetch_add(1, Ordering::Relaxed);
            let start = crate::kernel::timer::monotonic_ns();

            crate::kernel::task::prepare_park_current_on_detached(self.key(), None);
            drop(kernel);

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

        WAITQ_LEGACY_WAITS.fetch_add(1, Ordering::Relaxed);
        crate::kernel::task::park_current_on(self.key());
    }

    pub fn wait_until(&self, ticket: WaitTicket, deadline_ns: u64) -> bool {
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            return true;
        }

        let profondeur_avant = crate::kernel::smp_lock::profondeur_locale();
        let kernel = enter_bkl();
        let _inscrit = Inscription::nouvelle(self);

        if self.generation.load(Ordering::SeqCst) != ticket.0 {
            return true;
        }

        if profondeur_avant == 0 {
            WAITQ_DETACHED_WAITS.fetch_add(1, Ordering::Relaxed);
            let start = crate::kernel::timer::monotonic_ns();

            crate::kernel::task::prepare_park_current_on_detached(
                self.key(),
                Some(deadline_ns),
            );
            drop(kernel);

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

        WAITQ_LEGACY_WAITS.fetch_add(1, Ordering::Relaxed);
        crate::kernel::task::park_current_on_until(self.key(), deadline_ns)
    }
}
