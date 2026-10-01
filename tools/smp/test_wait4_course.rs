//! Preuve hote de BOUCHAUD_WAIT4_PUBLIE_PUIS_RELIT_V1.
//!
//! Le parent (`sys_wait4`) et le fils (`exit` -> `notify_parent_of_exit_for`)
//! se rencontrent sur deux grandeurs : `zombie` (pose par le fils sous le verrou
//! `lifecycle`) et `waiting_for_child` (pose par le parent, consomme par le fils
//! par CAS). Ce modele reproduit les deux protocoles avec de vrais fils
//! d'execution et le meme ordonnancement memoire que le noyau :
//!
//!   * ANCIEN : le parent cherche le zombie, PUIS publie son attente. Un fils
//!     qui meurt entre les deux trouve `waiting_for_child = false` et ne
//!     reveille personne. Le test elargit la fenetre (une pause injectee) et
//!     EXIGE d'observer des reveils perdus : sinon le banc ne prouve rien ;
//!   * NOUVEAU : le parent publie, barriere, PUIS relit les zombies et annule
//!     son parking s'il en trouve. La meme pause, placee a chaque point du
//!     protocole, ne doit produire AUCUN reveil perdu.
//!
//! Le noyau a perdu ce reveil sur l'image B11 (`wait4-course-probe` : parent
//! `Blocked`, `cle_attente=0`, `echeance=0`, `zombies=4`).

use std::sync::atomic::{fence, AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PRET: u8 = 0;
const BLOQUE: u8 = 1;

struct Etat {
    zombie: Mutex<bool>,
    attend_fils: AtomicBool,
    etat: AtomicU8,
}

impl Etat {
    fn neuf() -> Self {
        Self { zombie: Mutex::new(false), attend_fils: AtomicBool::new(false), etat: AtomicU8::new(PRET) }
    }

    fn a_un_zombie(&self) -> bool {
        *self.zombie.lock().unwrap()
    }

    /// `exit` du fils : zombie sous verrou, puis CAS sur l'attente du parent.
    fn fils_meurt(&self) {
        *self.zombie.lock().unwrap() = true;
        if self.attend_fils.compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            let _ = self.etat.compare_exchange(BLOQUE, PRET, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    /// Le parent dort jusqu'a etre remis pret ; `false` si personne ne vient
    /// dans `patience_ms`.
    fn dort(&self, patience_ms: u64) -> bool {
        let fin = Instant::now() + Duration::from_millis(patience_ms);
        while self.etat.load(Ordering::Acquire) == BLOQUE {
            if Instant::now() > fin {
                return false;
            }
            std::hint::spin_loop();
        }
        true
    }
}

fn pause() {
    for _ in 0..2_000 {
        std::hint::spin_loop();
    }
}

/// Rend `true` si le parent a bien recolte son fils.
fn wait4_ancien(e: &Etat, pause_fenetre: bool) -> bool {
    loop {
        if e.a_un_zombie() {
            return true;
        }
        if pause_fenetre {
            pause(); // la fenetre : zombie absent, attente pas encore publiee
        }
        e.attend_fils.store(true, Ordering::Release);
        e.etat.store(BLOQUE, Ordering::SeqCst);
        // Patience courte : une fausse perte ne ferait que grossir un compte
        // dont on exige seulement qu'il soit non nul.
        if !e.dort(20) {
            return false; // reveil perdu : le parent dormirait pour toujours
        }
        e.attend_fils.store(false, Ordering::Release);
    }
}

fn wait4_nouveau(e: &Etat, pause_point: u8) -> bool {
    loop {
        if e.a_un_zombie() {
            return true;
        }
        if pause_point == 1 {
            pause();
        }
        e.attend_fils.store(true, Ordering::Release);
        e.etat.store(BLOQUE, Ordering::SeqCst);
        if pause_point == 2 {
            pause();
        }
        fence(Ordering::SeqCst);
        if e.a_un_zombie() {
            let _ = e.attend_fils.compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire);
            e.etat.store(PRET, Ordering::SeqCst);
            continue;
        }
        if pause_point == 3 {
            pause();
        }
        // Patience longue : une fausse perte rendrait le test instable.
        if !e.dort(200) {
            return false;
        }
        e.attend_fils.store(false, Ordering::Release);
    }
}

/// Fait courir parent et fils `tours` fois ; rend le nombre de reveils perdus.
fn course(tours: usize, parent: impl Fn(&Etat) -> bool + Send + Sync + 'static) -> usize {
    let parent = Arc::new(parent);
    let mut perdus = 0;
    for tour in 0..tours {
        let e = Arc::new(Etat::neuf());
        let depart = Arc::new(Barrier::new(2));
        let (e2, d2) = (e.clone(), depart.clone());
        let fils = thread::spawn(move || {
            d2.wait();
            for _ in 0..(tour % 7) * 300 {
                std::hint::spin_loop();
            }
            e2.fils_meurt();
        });
        depart.wait();
        if !parent(&e) {
            perdus += 1;
        }
        fils.join().unwrap();
    }
    perdus
}

#[test]
fn l_ancien_ordre_perd_des_reveils() {
    // Sans ce temoin, l'absence de perte avec le nouvel ordre ne prouverait
    // rien : le banc pourrait simplement ne jamais toucher la fenetre.
    let perdus = course(200, |e| wait4_ancien(e, true));
    eprintln!("ancien ordre : {} reveil(s) perdu(s) sur 200", perdus);
    assert!(perdus > 0, "le banc n'atteint pas la fenetre : aucun reveil perdu avec l'ancien ordre");
}

#[test]
fn le_nouvel_ordre_ne_perd_aucun_reveil() {
    for point in 1..=3u8 {
        let perdus = course(400, move |e| wait4_nouveau(e, point));
        assert_eq!(perdus, 0, "reveil perdu avec pause au point {}", point);
    }
}
