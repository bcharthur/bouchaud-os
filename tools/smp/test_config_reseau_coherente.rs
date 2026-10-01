//! Preuve hote de BOUCHAUD_NET_CONFIG_GENERATION_V1.
//!
//! La configuration d'eth0 (adresse, passerelle, resolveur, masque) est UNE
//! valeur : `same_subnet` combine adresse et masque, `smol_tcp` adresse et
//! passerelle. Ce modele reproduit les deux protocoles de publication avec de
//! vrais fils et le meme ordonnancement memoire que le noyau :
//!
//!   * ANCIEN : quatre statiques ecrites une a une, en DEUX appels
//!     (`set_config(ip, gw, dns)` puis `pose_identite_reseau(.., masque)`).
//!     Un lecteur concurrent peut voir l'adresse de la generation N et le
//!     masque de N-1. Le test EXIGE d'en observer : sinon le banc ne touche
//!     pas la fenetre et ne prouve rien ;
//!   * NOUVEAU : sequence impaire pendant l'ecriture, champs atomiques,
//!     sequence paire a la fin ; le lecteur relit si la sequence a bouge.
//!     AUCUN melange de generations tolere.

use std::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// Chaque champ porte la generation dans son octet de poids faible : un
/// instantane coherent a les quatre egaux.
fn champs(generation: u32) -> [u32; 4] {
    let g = generation & 0xFF;
    [0x0A00_0000 | g, 0x0A00_0100 | g, 0x0A00_0200 | g, 0xFFFF_FF00 | g]
}

fn coherent(c: &[u32; 4]) -> bool {
    c.iter().all(|v| v & 0xFF == c[0] & 0xFF)
}

struct Ancien {
    champs: [AtomicU32; 4],
}

impl Ancien {
    fn publie(&self, generation: u32) {
        let c = champs(generation);
        // set_config(ip, gw, dns)
        for i in 0..3 {
            self.champs[i].store(c[i], Ordering::Relaxed);
        }
        std::hint::spin_loop();
        // pose_identite_reseau(.., masque)
        self.champs[3].store(c[3], Ordering::Relaxed);
    }
    fn lit(&self) -> [u32; 4] {
        core::array::from_fn(|i| self.champs[i].load(Ordering::Relaxed))
    }
}

struct Generation {
    sequence: AtomicU64,
    champs: [AtomicU32; 4],
    ecrivains: Mutex<()>,
}

impl Generation {
    fn publie(&self, generation: u32) {
        let _seul = self.ecrivains.lock().unwrap();
        let c = champs(generation);
        let s = self.sequence.load(Ordering::Relaxed);
        self.sequence.store(s + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        for i in 0..3 {
            self.champs[i].store(c[i], Ordering::Relaxed);
        }
        std::hint::spin_loop();
        self.champs[3].store(c[3], Ordering::Relaxed);
        self.sequence.store(s + 2, Ordering::Release);
    }
    fn lit(&self) -> [u32; 4] {
        loop {
            let avant = self.sequence.load(Ordering::Acquire);
            if avant & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let c = core::array::from_fn(|i| self.champs[i].load(Ordering::Relaxed));
            fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Relaxed) == avant {
                return c;
            }
        }
    }
}

/// Deux ecrivains (veilleur de lien, banc RX) et `lecteurs` fils de lecture ;
/// rend le nombre d'instantanes incoherents vus.
fn course<P, L>(publie: P, lit: L, lecteurs: usize, publications: u32) -> u64
where
    P: Fn(u32) + Send + Sync + 'static,
    L: Fn() -> [u32; 4] + Send + Sync + 'static,
{
    let publie = Arc::new(publie);
    let lit = Arc::new(lit);
    let fini = Arc::new(AtomicBool::new(false));
    let melanges = Arc::new(AtomicU64::new(0));
    let mut fils = Vec::new();
    for _ in 0..lecteurs {
        let (lit, fini, melanges) = (lit.clone(), fini.clone(), melanges.clone());
        fils.push(thread::spawn(move || {
            while !fini.load(Ordering::Relaxed) {
                if !coherent(&lit()) {
                    melanges.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    let ecrivains: Vec<_> = (0..2u32)
        .map(|e| {
            let publie = publie.clone();
            thread::spawn(move || {
                for g in 0..publications {
                    publie(g * 2 + e);
                }
            })
        })
        .collect();
    for e in ecrivains {
        e.join().unwrap();
    }
    fini.store(true, Ordering::Relaxed);
    for f in fils {
        f.join().unwrap();
    }
    melanges.load(Ordering::Relaxed)
}

#[test]
fn l_ancienne_publication_montre_des_configurations_melangees() {
    let a = Arc::new(Ancien { champs: Default::default() });
    let (a1, a2) = (a.clone(), a.clone());
    let melanges = course(move |g| a1.publie(g), move || a2.lit(), 4, 200_000);
    eprintln!("ancien : {} instantane(s) melange(s)", melanges);
    assert!(melanges > 0, "le banc n'atteint pas la fenetre : aucun melange avec l'ancien protocole");
}

#[test]
fn la_generation_ne_montre_jamais_de_melange() {
    for _ in 0..3 {
        let g = Arc::new(Generation {
            sequence: AtomicU64::new(0),
            champs: Default::default(),
            ecrivains: Mutex::new(()),
        });
        let (g1, g2) = (g.clone(), g.clone());
        let melanges = course(move |n| g1.publie(n), move || g2.lit(), 4, 200_000);
        assert_eq!(melanges, 0, "instantane melange malgre la generation");
    }
}
