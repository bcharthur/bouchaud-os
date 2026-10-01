// Consommateur événementiel.
//
// Le bureau avait ici un chemin a lui : il tenait le gros verrou, et son
// attente INTERFACE le suspendait avant de dormir puis le reprenait. Ni le
// verrou ni le chemin n'existent plus (BOUCHAUD_BKL_SUPPRIME_V1) : toutes les
// attentes passent par la meme WaitSource.

impl Reveil {
    #[inline]
    pub fn billet(&self) -> Billet {
        Billet {
            source: self.source.ticket(),
        }
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

    pub fn attends(&self, billet: Billet, echeance_ns: u64) -> Fin {
        self.note_fin(self.source.wait_until(billet.source, echeance_ns))
    }
}
