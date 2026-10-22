//! BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 -- la file des producteurs qui
//! attendent de la place dans l'anneau de la console (src/drivers/serial/
//! file_place.rs), avec de vrais fils et des graines (BOUCHAUD_PROP_SEED).
//!
//! Avant : chaque producteur retentait pour son compte ; la place liberee par
//! l'un etait prise par un autre, et le premier renoncait a l'echeance d'une
//! seconde (TCG SMP4, port a 5 us/octet : 323 lignes perdues). Ces proprietes
//! sont celles dont depend l'absence de perte : ordre d'arrivee, une seule
//! tete, aucune sortie qui bloque les suivants, une tete disparue passee.

#![allow(dead_code)]

#[path = "../../src/drivers/serial/file_place.rs"]
mod file_place;

use file_place::{tient, FilePlace};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    fn sous(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn seed() -> u64 {
    std::env::var("BOUCHAUD_PROP_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| s.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        .unwrap_or(0xB0C4_A0D5_11)
}

fn patiente(tours: u64) {
    for _ in 0..tours {
        std::hint::spin_loop();
    }
}

const BORNE: Duration = Duration::from_secs(20);

/// Sans saut ni abandon : chaque ticket est servi dans l'ordre d'arrivee, une
/// seule tete a la fois, et la file se vide.
#[test]
fn property_ordre_d_arrivee_et_une_seule_tete() {
    let mut rng = Rng(seed());
    for _ in 0..8 {
        let fils = 2 + rng.sous(7) as usize;
        let lignes = 20 + rng.sous(80) as usize;
        let file = Arc::new(FilePlace::neuve());
        let en_tete = Arc::new(AtomicUsize::new(0));
        let servis = Arc::new(Mutex::new(Vec::new()));
        let mut h = Vec::new();
        for f in 0..fils {
            let (file, en_tete, servis) = (file.clone(), en_tete.clone(), servis.clone());
            let mut r = Rng(rng.next() | 1);
            h.push(std::thread::spawn(move || {
                let debut = Instant::now();
                for _ in 0..lignes {
                    let t = file.prend();
                    while !file.a_son_tour(t) {
                        assert!(debut.elapsed() < BORNE, "fil {f} : ticket {t} jamais servi");
                        std::thread::yield_now();
                    }
                    assert_eq!(en_tete.fetch_add(1, Ordering::SeqCst), 0, "deux tetes en meme temps");
                    servis.lock().unwrap().push(t);
                    patiente(r.sous(2_000));
                    en_tete.fetch_sub(1, Ordering::SeqCst);
                    file.sort(t);
                }
            }));
        }
        for x in h {
            x.join().unwrap();
        }
        let servis = servis.lock().unwrap();
        let attendu: Vec<u64> = (0..(fils * lignes) as u64).collect();
        assert_eq!(*servis, attendu, "servis hors de l'ordre d'arrivee");
        assert!(file.vide());
        assert_eq!(file.servi(), (fils * lignes) as u64);
    }
}

/// La console devient synchrone : tous les attendants sortent, en desordre.
/// Aucun ne reste bloque, `servi` ne recule jamais et ne depasse jamais un
/// ticket distribue.
#[test]
fn property_abandons_ne_bloquent_personne() {
    let mut rng = Rng(seed());
    for _ in 0..8 {
        let fils = 2 + rng.sous(7) as usize;
        let file = Arc::new(FilePlace::neuve());
        let active = Arc::new(AtomicBool::new(true));
        let fini = Arc::new(AtomicUsize::new(0));
        // Un observateur verifie la monotonie de `servi` pendant toute la course.
        let observe = {
            let file = file.clone();
            let fini = fini.clone();
            std::thread::spawn(move || {
                let mut avant = 0;
                while fini.load(Ordering::SeqCst) < fils {
                    let s = file.servi();
                    assert!(s >= avant, "servi a recule : {avant} -> {s}");
                    avant = s;
                    std::thread::yield_now();
                }
            })
        };
        let mut h = Vec::new();
        for _ in 0..fils {
            let (file, active, fini) = (file.clone(), active.clone(), fini.clone());
            let mut r = Rng(rng.next() | 1);
            h.push(std::thread::spawn(move || {
                let debut = Instant::now();
                let mut rangees = 0u64;
                loop {
                    let t = file.prend();
                    loop {
                        assert!(debut.elapsed() < BORNE, "ticket {t} bloque apres l'abandon");
                        if !active.load(Ordering::SeqCst) {
                            break;
                        }
                        if file.a_son_tour(t) {
                            rangees += 1;
                            patiente(r.sous(500));
                            break;
                        }
                        std::thread::yield_now();
                    }
                    file.sort(t);
                    if !active.load(Ordering::SeqCst) {
                        break;
                    }
                }
                fini.fetch_add(1, Ordering::SeqCst);
                rangees
            }));
        }
        std::thread::sleep(Duration::from_micros(200 + rng.sous(3_000)));
        active.store(false, Ordering::SeqCst);
        for x in h {
            x.join().unwrap();
        }
        observe.join().unwrap();
        assert!(file.vide(), "file non vide apres l'abandon general");
    }
}

/// Une tete qui disparait sans sortir (producteur detruit au milieu de son
/// attente) : les suivants la sautent et finissent ; aucun saut ne depasse le
/// ticket de celui qui saute, et la file finit vide sauf ce ticket mort.
#[test]
fn property_tete_disparue_sautee() {
    let mut rng = Rng(seed());
    for _ in 0..4 {
        let fils = 2 + rng.sous(5) as usize;
        let lignes = 10 + rng.sous(30) as usize;
        let file = Arc::new(FilePlace::neuve());
        // Le ticket mort : pris, jamais rendu.
        let mort = file.prend();
        let sauts = Arc::new(AtomicU64::new(0));
        let mut h = Vec::new();
        for _ in 0..fils {
            let (file, sauts) = (file.clone(), sauts.clone());
            h.push(std::thread::spawn(move || {
                let debut = Instant::now();
                for _ in 0..lignes {
                    let t = file.prend();
                    let mut vu = file.servi();
                    let mut depuis = Instant::now();
                    while !file.a_son_tour(t) {
                        assert!(debut.elapsed() < BORNE, "ticket {t} jamais servi, tete morte non sautee");
                        let s = file.servi();
                        assert!(s <= t, "servi {s} au-dela du ticket {t} encore en attente");
                        if s != vu {
                            vu = s;
                            depuis = Instant::now();
                        } else if depuis.elapsed() > Duration::from_millis(5) && file.saute(vu, t) {
                            sauts.fetch_add(1, Ordering::SeqCst);
                            depuis = Instant::now();
                        }
                        std::thread::yield_now();
                    }
                    file.sort(t);
                }
            }));
        }
        for x in h {
            x.join().unwrap();
        }
        assert!(sauts.load(Ordering::SeqCst) >= 1, "la tete morte {mort} n'a jamais ete sautee");
        assert!(file.vide());
    }
}

#[test]
fn saut_jamais_au_dela_de_son_ticket() {
    let file = FilePlace::neuve();
    let a = file.prend();
    let b = file.prend();
    assert!(!file.saute(b, a), "a ne doit pas pouvoir sauter b, plus loin que lui");
    assert!(!file.saute(a, a), "on ne se saute pas soi-meme");
    assert!(file.saute(a, b));
    assert!(file.a_son_tour(b));
    // Un vu perime ne saute rien.
    assert!(!file.saute(a, b));
    assert_eq!(file.servi(), 1);
}

/// La console a ete synchrone un instant : `b` l'a vu et est sorti, `a`
/// (arrive avant lui) non. `a` ne doit pas rester bloque derriere un `servi`
/// qui l'a depasse.
#[test]
fn sortie_d_un_suivant_libere_les_precedents() {
    let file = FilePlace::neuve();
    let a = file.prend();
    let b = file.prend();
    let c = file.prend();
    file.sort(b);
    assert!(file.a_son_tour(a), "a bloque : servi l'a depasse");
    assert!(file.a_son_tour(c));
    file.sort(a);
    file.sort(c);
    assert!(file.vide());
}

#[test]
fn sortie_tardive_ne_fait_pas_reculer() {
    let file = FilePlace::neuve();
    let a = file.prend();
    let b = file.prend();
    let c = file.prend();
    file.sort(c);
    assert_eq!(file.servi(), 3);
    file.sort(a);
    file.sort(b);
    assert_eq!(file.servi(), 3);
    assert!(file.vide());
}

/// La reserve d'urgence : un producteur qui peut attendre laisse toujours
/// `reserve` octets a ceux qui ne le peuvent pas.
#[test]
fn property_reserve_toujours_laissee() {
    let mut rng = Rng(seed());
    for _ in 0..100_000 {
        let capacite = 1 + rng.sous(1 << 17) as usize;
        let occupe = rng.sous(capacite as u64 + 1) as usize;
        let total = rng.sous(4096) as usize;
        let reserve = rng.sous(16_384) as usize;
        let libre = capacite - occupe;
        if tient(libre, total, reserve) {
            assert!(capacite - (occupe + 2 + total) >= reserve);
        } else {
            assert!(libre < 2 + total + reserve);
        }
    }
}
