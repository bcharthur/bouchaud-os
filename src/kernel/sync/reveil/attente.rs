// Consommateur événementiel.
//
// Le desktop est encore lancé par le trampoline kernel-thread historique, qui
// possède un KernelGuard racine. V9 le suspendait par checkpoints, mais son
// attente INTERFACE restait un chemin WaitQueue legacy.
//
// Ici, uniquement pour la source globale INTERFACE et le kernel-thread desktop
// a toute profondeur BKL locale non nulle :
//   1. suspendre toute la profondeur locale, exactement comme schedule() ;
//   2. attendre via WaitSource/WaitQueue V12 à depth=0 ;
//   3. restaurer UNE fois le guard au réveil.
//
// On supprime ainsi les millions de suspend/resume intermédiaires pendant un
// sommeil du bureau, sans casser le contrat historique à son retour.

impl Reveil {
    #[inline]
    pub fn billet(&self) -> Billet {
        Billet {
            source: self.source.ticket(),
        }
    }

    #[inline]
    fn est_attente_interface_desktop_detachable(&self) -> bool {
        core::ptr::eq(self, &INTERFACE)
            && crate::kernel::task::current_is_kernel_task()
            && crate::kernel::smp_lock::held_by_current_cpu()
            && crate::kernel::smp_lock::profondeur_locale() > 0
            && crate::arch::x86_64::cpu::interrupts_enabled()
            && crate::kernel::task::nom_pour_faute() == "desktop"
    }

    #[inline]
    fn note_fin(&self, wake: WaitSourceWake) -> Fin {
        match wake {
            WaitSourceWake::AlreadyChanged => {
                self.sommeils_evites.fetch_add(1, Ordering::Relaxed);
                Fin::DejaSignale
            }
            WaitSourceWake::Signaled => {
                self.sommeils.fetch_add(1, Ordering::Relaxed);
                self.reveils_signal.fetch_add(1, Ordering::Relaxed);
                Fin::Signale
            }
            WaitSourceWake::Deadline => {
                self.sommeils.fetch_add(1, Ordering::Relaxed);
                self.reveils_echeance.fetch_add(1, Ordering::Relaxed);
                Fin::Echeance
            }
        }
    }

    fn attends_interface_desktop_detache(
        &self,
        billet: Billet,
        echeance_ns: u64,
    ) -> Fin {
        INTERFACE_DETACHED_WAITS.fetch_add(1, Ordering::Relaxed);
        interface_phase(INTERFACE_WAIT_PREPARE);
        crate::kernel::task::stall_site_set(770, echeance_ns);

        let expected_depth = crate::kernel::smp_lock::profondeur_locale();
        let depth = crate::kernel::smp_lock::suspend_for_schedule();
        debug_assert_eq!(depth, expected_depth, "INTERFACE desktop: profondeur modifiee avant detach");
        debug_assert!(depth > 0, "INTERFACE desktop: detach sans BKL");
        if depth == 1 {
            INTERFACE_DETACHED_DEPTH1.fetch_add(1, Ordering::Relaxed);
        } else {
            INTERFACE_DETACHED_NESTED.fetch_add(1, Ordering::Relaxed);
        }
        INTERFACE_DETACHED_MAX_DEPTH.fetch_max(depth as u64, Ordering::Relaxed);

        interface_phase(INTERFACE_WAIT_SLEEP);
        crate::kernel::task::stall_site_set(771, echeance_ns);
        let sleep_start = crate::kernel::timer::monotonic_ns();
        let wake = self.source.wait_until(billet.source, echeance_ns);
        let sleep_end = crate::kernel::timer::monotonic_ns();

        let sleep_ns = sleep_end.saturating_sub(sleep_start);
        INTERFACE_DETACHED_SLEEP_NS.fetch_add(sleep_ns, Ordering::Relaxed);
        interface_update_max(&INTERFACE_DETACHED_SLEEP_MAX_NS, sleep_ns);

        // À cet instant WaitQueue V12 a rendu depth=0.
        if crate::kernel::smp_lock::profondeur_locale() != 0 {
            INTERFACE_DEPTH_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        }

        interface_phase(INTERFACE_WAIT_RESUME);
        crate::kernel::task::stall_site_set(772, depth as u64);
        let resume_start = crate::kernel::timer::monotonic_ns();
        crate::kernel::smp_lock::resume_after_schedule(depth);
        let done = crate::kernel::timer::monotonic_ns();

        let resume_ns = done.saturating_sub(resume_start);
        INTERFACE_RESUME_WAIT_NS.fetch_add(resume_ns, Ordering::Relaxed);
        interface_update_max(&INTERFACE_RESUME_WAIT_MAX_NS, resume_ns);

        if crate::kernel::smp_lock::profondeur_locale() != depth {
            INTERFACE_DEPTH_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        }

        interface_phase(INTERFACE_WAIT_RETURN);
        // Laisser 773 visible jusqu'au prochain `gui::reveil::note_tour()`.
        crate::kernel::task::stall_site_set(773, resume_ns);

        self.note_fin(wake)
    }

    pub fn attends(&self, billet: Billet, echeance_ns: u64) -> Fin {
        if self.est_attente_interface_desktop_detachable() {
            self.attends_interface_desktop_detache(billet, echeance_ns)
        } else {
            self.note_fin(self.source.wait_until(billet.source, echeance_ns))
        }
    }
}
