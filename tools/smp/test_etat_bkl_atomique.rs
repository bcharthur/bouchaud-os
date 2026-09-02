//! Modele hote de la machine d'etat BKL empaquetee.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const OWNER_BITS: u32 = 8;
const OWNER_MASK: u64 = (1u64 << OWNER_BITS) - 1;

fn encode(owner: usize, depth: usize) -> u64 {
    assert_eq!(owner == 0, depth == 0);
    ((depth as u64) << OWNER_BITS) | owner as u64
}

fn decode(raw: u64) -> (usize, usize) {
    ((raw & OWNER_MASK) as usize, (raw >> OWNER_BITS) as usize)
}

#[test]
fn owner_local_sans_profondeur_est_non_representable() {
    for cpu in 0..16 {
        for depth in 1..32 {
            let (owner, relu) = decode(encode(cpu + 1, depth));
            assert_eq!(owner, cpu + 1);
            assert_eq!(relu, depth);
            assert!(owner != 0 && relu != 0);
        }
    }
    assert_eq!(decode(encode(0, 0)), (0, 0));
}

#[test]
fn acquisitions_concurrentes_ne_publient_jamais_un_demi_etat() {
    let etat = Arc::new(AtomicU64::new(0));
    let mut fils = Vec::new();
    for cpu in 0..8usize {
        let etat = Arc::clone(&etat);
        fils.push(std::thread::spawn(move || {
            for _ in 0..20_000 {
                if etat.compare_exchange(0, encode(cpu + 1, 1), Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    let (owner, depth) = decode(etat.load(Ordering::Acquire));
                    assert_eq!((owner, depth), (cpu + 1, 1));
                    etat.compare_exchange(
                        encode(cpu + 1, 1),
                        0,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ).expect("un seul proprietaire peut liberer");
                } else {
                    let (owner, depth) = decode(etat.load(Ordering::Acquire));
                    assert_eq!(owner == 0, depth == 0);
                }
            }
        }));
    }
    for fils in fils { fils.join().unwrap(); }
    assert_eq!(etat.load(Ordering::Acquire), 0);
}

#[test]
fn falsification_deux_atomiques_expose_exactement_la_panne() {
    use std::sync::atomic::AtomicUsize;
    let owner = AtomicUsize::new(0);
    let depth = AtomicUsize::new(0);
    owner.store(1, Ordering::Release);
    // Observation volontaire entre les deux publications de l'ancien modele.
    assert_eq!(owner.load(Ordering::Acquire), 1);
    assert_eq!(depth.load(Ordering::Acquire), 0);
    depth.store(1, Ordering::Release);
}
