#![allow(dead_code)]

#[path = "../../src/kernel/sync/rendezvous.rs"]
mod rendezvous;

use rendezvous::{Dormeur, Rendezvous};
use std::sync::atomic::{AtomicU8, Ordering};

struct Modele {
    etat: AtomicU8, // 0 pret, 1 bloque
}

impl Modele {
    fn new() -> Self { Self { etat: AtomicU8::new(0) } }
    fn bloque(&self) -> bool { self.etat.load(Ordering::SeqCst) == 1 }
}

impl Dormeur for Modele {
    fn publie_parking(&self) {
        self.etat.store(1, Ordering::SeqCst);
    }

    fn tente_reveil(&self) -> bool {
        self.etat
            .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn annule_parking(&self) {
        self.etat.store(0, Ordering::SeqCst);
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn seed() -> u64 {
    std::env::var("BOUCHAUD_PROP_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0xB0C4_A0D5_11)
}

#[test]
fn property_un_signal_apres_parking_reveille_toujours() {
    let mut rng = Rng(seed());
    for _ in 0..20_000 {
        let rv = Rendezvous::neuf();
        let d = Modele::new();
        let ticket = rv.ticket();

        rv.inscrit();

        // Une partie des iterations signale avant la publication du parking:
        // le dormeur doit alors detecter la generation changee et s'annuler.
        if rng.next() & 1 == 0 {
            rv.signale_seul();
        }

        let doit_dormir = rv.doit_dormir(ticket, &d);
        if doit_dormir {
            let n = rv.signale(1, std::iter::once(&d));
            assert_eq!(n, 1);
            assert!(!d.bloque(), "un signal apres parking a ete perdu");
        } else {
            assert!(!d.bloque(), "parking annule mais etat encore bloque");
        }
        rv.desinscrit();
    }
}

#[test]
fn property_les_signaux_ne_creent_pas_de_double_reveil() {
    let mut rng = Rng(seed() ^ 0x9E37_79B9);
    for _ in 0..10_000 {
        let rv = Rendezvous::neuf();
        let d = Modele::new();
        let ticket = rv.ticket();
        rv.inscrit();
        if rv.doit_dormir(ticket, &d) {
            let first = rv.signale(1, std::iter::once(&d));
            let second = rv.signale(1, std::iter::once(&d));
            assert_eq!(first, 1);
            assert_eq!(second, 0);
        }
        rv.desinscrit();

        // Mélange supplémentaire pour varier les graines/ordres d'exécution.
        for _ in 0..(rng.next() & 7) {
            std::hint::spin_loop();
        }
    }
}
