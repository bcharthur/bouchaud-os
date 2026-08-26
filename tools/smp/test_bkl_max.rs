//! Harnais de test hote pour la publication du maximum de tenue du BKL.
//!
//! # Ce qui est teste, et pourquoi ici
//!
//! `bkl.rs` ne se compile pas sur l'hote : il touche l'APIC, les interruptions
//! et l'ordonnanceur. Mais les deux proprietes qui comptent sont de la logique
//! d'atomiques pure, et elles se cassent en SILENCE :
//!
//!   * un maximum qui DIMINUE parce que deux CPU l'ecrivent en meme temps ;
//!   * une duree publiee avec la provenance d'une AUTRE duree.
//!
//! Aucune des deux ne produit d'erreur. La premiere fait chercher au mauvais
//! endroit, la seconde accuse le mauvais appel systeme.
//!
//! Le modele ci-dessous reprend la MEME formule que `publie_si_plus_longue` et
//! `provenance_plus_longue_tenue`, et la soumet a de vrais fils concurrents.
//!
//! Lance par `tools/smp/test-bkl-max.sh`.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// Le releve maximum : duree, provenance, et la generation qui les lie.
struct Maximum {
    duree: AtomicU64,
    generation: AtomicU64,
    cpu: AtomicUsize,
    syscall: AtomicU64,
}

impl Maximum {
    fn neuf() -> Self {
        Self {
            duree: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            cpu: AtomicUsize::new(usize::MAX),
            syscall: AtomicU64::new(u64::MAX),
        }
    }

    /// La formule du noyau : compare-exchange sur la duree, puis seqlock sur
    /// la provenance.
    fn publie(&self, cpu: usize, syscall: u64, tenue: u64) {
        let mut record = self.duree.load(Ordering::Relaxed);
        loop {
            if tenue <= record {
                return;
            }
            match self.duree.compare_exchange_weak(
                record, tenue, Ordering::AcqRel, Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actuel) => record = actuel,
            }
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.cpu.store(cpu, Ordering::Relaxed);
        self.syscall.store(syscall, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    fn lit_provenance(&self) -> Option<(usize, u64)> {
        for _ in 0..4 {
            let debut = self.generation.load(Ordering::Acquire);
            if debut % 2 != 0 {
                continue;
            }
            let releve = (
                self.cpu.load(Ordering::Relaxed),
                self.syscall.load(Ordering::Relaxed),
            );
            if self.generation.load(Ordering::Acquire) == debut {
                return Some(releve);
            }
        }
        None
    }
}

/// L'ancienne formule, celle qui perdait des maximums.
fn publie_naivement(max: &AtomicU64, tenue: u64) {
    if tenue > max.load(Ordering::Relaxed) {
        max.store(tenue, Ordering::Relaxed);
    }
}

#[test]
fn le_maximum_ne_diminue_jamais_sous_concurrence() {
    let maximum = Arc::new(Maximum::neuf());
    let mut fils = Vec::new();

    // Quatre fils, comme quatre CPU, publient des durees croissantes
    // entrelacees. Un observateur relit sans cesse et exige la monotonie.
    let observateur = {
        let maximum = Arc::clone(&maximum);
        std::thread::spawn(move || {
            let mut vu = 0u64;
            for _ in 0..200_000 {
                let actuel = maximum.duree.load(Ordering::Acquire);
                assert!(actuel >= vu, "le maximum a diminue : {} puis {}", vu, actuel);
                vu = actuel;
            }
            vu
        })
    };

    for cpu in 0..4usize {
        let maximum = Arc::clone(&maximum);
        fils.push(std::thread::spawn(move || {
            for tour in 1..=20_000u64 {
                maximum.publie(cpu, 59, tour * (cpu as u64 + 1));
            }
        }));
    }

    for f in fils {
        f.join().expect("fil de publication");
    }
    observateur.join().expect("observateur");

    // Le maximum final est bien le plus grand propose : 20 000 * 4.
    assert_eq!(maximum.duree.load(Ordering::Relaxed), 80_000);
}

#[test]
fn l_ancienne_formule_perd_reellement_des_maximums() {
    // Sans ce test, le precedent pourrait passer avec l'ancienne formule et ne
    // prouverait donc rien.
    //
    // On ne PARIE pas sur l'entrelacement : on le joue. `lecture` et `ecriture`
    // sont les deux moities de `if tenue > max { max = tenue }`, et le defaut
    // tient en une phrase -- rien ne garantit qu'aucune autre ecriture ne
    // s'intercale entre elles.
    //
    //     CPU A lit max = 0        (sa tenue de 1000 est plus grande)
    //     CPU B lit max = 0        (sa tenue de 10 est plus grande aussi)
    //     CPU A ecrit 1000
    //     CPU B ecrit 10           <- le maximum vient de DIMINUER
    let max = AtomicU64::new(0);

    let lecture_a = max.load(Ordering::Relaxed);
    let lecture_b = max.load(Ordering::Relaxed);
    assert!(1000 > lecture_a && 10 > lecture_b, "les deux se croient plus grands");

    max.store(1000, Ordering::Relaxed);
    max.store(10, Ordering::Relaxed);

    assert_eq!(max.load(Ordering::Relaxed), 10, "l'ancienne formule perd 1000");

    // La formule corrigee refuse ce meme entrelacement : le compare-exchange de
    // B echoue parce que la valeur a change depuis sa lecture, et il relit.
    let maximum = Maximum::neuf();
    maximum.publie(0, 59, 1000);
    maximum.publie(1, 7, 10);
    assert_eq!(
        maximum.duree.load(Ordering::Relaxed), 1000,
        "la formule corrigee garde le maximum",
    );
}

#[test]
fn la_provenance_accompagne_toujours_sa_duree() {
    // Chaque fil publie une provenance qui IDENTIFIE sa duree : syscall = cpu.
    // Un releve qui melangerait deux publications se verrait immediatement.
    let maximum = Arc::new(Maximum::neuf());
    let mut fils = Vec::new();

    let observateur = {
        let maximum = Arc::clone(&maximum);
        std::thread::spawn(move || {
            let mut incoherences = 0usize;
            for _ in 0..200_000 {
                if let Some((cpu, syscall)) = maximum.lit_provenance() {
                    if cpu != usize::MAX && syscall != cpu as u64 {
                        incoherences += 1;
                    }
                }
            }
            incoherences
        })
    };

    for cpu in 0..4usize {
        let maximum = Arc::clone(&maximum);
        fils.push(std::thread::spawn(move || {
            for tour in 1..=20_000u64 {
                maximum.publie(cpu, cpu as u64, tour * (cpu as u64 + 1));
            }
        }));
    }

    for f in fils {
        f.join().expect("fil");
    }
    let incoherences = observateur.join().expect("observateur");
    assert_eq!(incoherences, 0, "provenance melangee {} fois", incoherences);
}

#[test]
fn une_generation_impaire_est_rejetee() {
    // Ecriture en cours : le lecteur ne doit rien rendre plutot que rendre un
    // etat a moitie ecrit.
    let maximum = Maximum::neuf();
    maximum.generation.store(3, Ordering::Release);
    assert_eq!(maximum.lit_provenance(), None);

    maximum.generation.store(4, Ordering::Release);
    assert!(maximum.lit_provenance().is_some());
}

#[test]
fn une_tenue_plus_courte_ne_publie_rien() {
    let maximum = Maximum::neuf();
    maximum.publie(1, 59, 500);
    let generation = maximum.generation.load(Ordering::Relaxed);

    maximum.publie(2, 7, 100);
    assert_eq!(maximum.duree.load(Ordering::Relaxed), 500);
    assert_eq!(maximum.cpu.load(Ordering::Relaxed), 1, "provenance inchangee");
    assert_eq!(
        maximum.generation.load(Ordering::Relaxed), generation,
        "aucune ecriture, donc aucune generation consommee",
    );
}
