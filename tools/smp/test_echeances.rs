//! Le raccourci d'echeances peut-il faire sauter un reveil ?
//!
//! # Ce qui est en jeu
//!
//! `wake_sleepers()` balayait toute la table des taches a chaque `schedule()`,
//! c'est-a-dire des milliers de fois par seconde et par CPU, sous le gros
//! verrou. Il consulte desormais une borne : tant que l'heure ne l'a pas
//! atteinte, aucune tache ne peut etre due.
//!
//! Le gain est reel et le risque aussi : une borne trop TARD fait dormir une
//! tache au-dela de son echeance -- un `pthread_cond_timedwait` qui ne rend
//! jamais la main, un `poll` qui rate son delai. Rien dans le journal ne le
//! dirait ; on verrait seulement une machine qui se fige par moments.
//!
//! # La propriete
//!
//! Pour toute suite d'armements, de retraits et de balayages :
//!
//!     doit_balayer(t) == false  =>  aucune echeance <= t
//!
//! Autrement dit : sauter un balayage ne peut jamais sauter un reveil. Le test
//! rejoue des sequences pseudo-aleatoires deterministes contre un modele de
//! reference -- l'ensemble exact des echeances -- et verifie l'implication a
//! chaque instant.
//!
//! Lance par `tools/smp/test-echeances.sh`.

extern crate alloc;

#[path = "../../src/kernel/scheduler/echeances.rs"]
mod echeances;

use alloc::collections::BTreeMap;
use echeances::{Echeances, JAMAIS};

/// Le modele : qui attend, et jusqu'a quand. C'est la verite que la borne
/// approche.
#[derive(Default)]
struct Modele {
    attentes: BTreeMap<usize, u64>,
}

impl Modele {
    fn arme(&mut self, tache: usize, echeance: u64) {
        if echeance != 0 {
            self.attentes.insert(tache, echeance);
        }
    }

    fn retire(&mut self, tache: usize) {
        self.attentes.remove(&tache);
    }

    /// Le vrai minimum, celui que `recalcule_echeance` calcule dans le noyau.
    fn minimum(&self) -> u64 {
        self.attentes.values().copied().min().unwrap_or(JAMAIS)
    }

    /// Y a-t-il une echeance due a `maintenant` ?
    fn due(&self, maintenant: u64) -> bool {
        self.attentes.values().any(|&e| maintenant >= e)
    }

    /// Le balayage du noyau : reveille les dues et recale la borne.
    fn balaie(&mut self, maintenant: u64) {
        self.attentes.retain(|_, e| maintenant < *e);
    }
}

/// Une suite deterministe, pour que l'echec soit reproductible.
struct Suite(u64);

impl Suite {
    fn suivant(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

// ─── LA propriete ──────────────────────────────────────────────────────────

/// Sauter un balayage ne doit jamais sauter un reveil.
#[test]
fn un_balayage_saute_ne_saute_jamais_un_reveil() {
    for graine in [1u64, 7, 42, 1337, 999_983] {
        let mut suite = Suite(graine);
        let borne = Echeances::neuve();
        let mut modele = Modele::default();
        let mut maintenant = 0u64;

        for tour in 0..4000 {
            match suite.suivant() % 5 {
                // Armer une echeance.
                0 | 1 => {
                    let tache = (suite.suivant() % 8) as usize;
                    let echeance = maintenant + 1 + suite.suivant() % 500;
                    modele.arme(tache, echeance);
                    borne.arme(echeance);
                }
                // Retirer une echeance -- un `FUTEX_WAKE`, une fermeture. Le
                // noyau ne touche PAS la borne ici, et c'est le cas subtil :
                // un retrait ne peut que reculer le vrai minimum.
                2 => {
                    let tache = (suite.suivant() % 8) as usize;
                    modele.retire(tache);
                }
                // Avancer le temps.
                3 => {
                    maintenant += 1 + suite.suivant() % 200;
                }
                // Balayer, comme `wake_sleepers`.
                _ => {
                    if borne.commence_balayage(maintenant) {
                        modele.balaie(maintenant);
                        borne.recale(modele.minimum());
                    }
                }
            }

            // L'IMPLICATION, verifiee a chaque tour.
            if !borne.doit_balayer(maintenant) {
                assert!(
                    !modele.due(maintenant),
                    "graine {graine}, tour {tour} : balayage saute a t={maintenant} \
                     alors qu'une echeance etait due (borne={}, minimum reel={})",
                    borne.borne(),
                    modele.minimum()
                );
            }
        }
    }
}

/// Et le raccourci doit REELLEMENT raccourcir : une borne toujours atteinte
/// laisserait la propriete vraie sans rien economiser.
#[test]
fn le_raccourci_evite_la_plupart_des_balayages() {
    let borne = Echeances::neuve();
    // Le cas reel : un fil bloque une seconde, le tick reveille le CPU toutes
    // les millisecondes. Mille reveils, un seul balayage utile.
    borne.arme(1_000_000_000);
    let mut balayages = 0usize;
    for tick in 0..1000u64 {
        if borne.doit_balayer(tick * 1_000_000) {
            balayages += 1;
        }
    }
    assert_eq!(balayages, 0, "aucun balayage avant l'echeance");
    assert!(borne.doit_balayer(1_000_000_000), "et un a l'echeance");
}

// ─── Les cas de bord ───────────────────────────────────────────────────────

/// Une borne neuve ne fait rien balayer : au demarrage, personne n'attend.
#[test]
fn une_borne_neuve_ne_declenche_rien() {
    let borne = Echeances::neuve();
    assert!(!borne.doit_balayer(0));
    assert!(!borne.doit_balayer(u64::MAX - 1));
    assert_eq!(borne.borne(), JAMAIS);
}

/// Zero veut dire « pas d'echeance » -- c'est la valeur que `wake_deadline_ns`
/// porte au repos. L'armer ramenerait la borne a zero et ferait balayer a
/// chaque tour : exactement ce qu'on cherche a eviter.
#[test]
fn une_echeance_nulle_n_arme_rien() {
    let borne = Echeances::neuve();
    borne.arme(0);
    assert_eq!(borne.borne(), JAMAIS, "zero ne doit pas armer");
    assert!(!borne.doit_balayer(1_000_000));
}

/// La borne ne recule jamais toute seule : deux armements gardent le plus tot.
#[test]
fn la_borne_garde_la_plus_proche() {
    let borne = Echeances::neuve();
    borne.arme(500);
    borne.arme(900);
    assert_eq!(borne.borne(), 500);
    borne.arme(100);
    assert_eq!(borne.borne(), 100);
    // Et seul un balayage revendique peut la repousser.
    assert!(borne.commence_balayage(100));
    borne.recale(900);
    assert_eq!(borne.borne(), 900);
}

/// Une echeance armee pendant le balayage ne doit jamais etre ecrasee par le
/// recalage calcule avant elle.
#[test]
fn un_armement_concurrent_survit_au_recalage() {
    let borne = Echeances::neuve();
    borne.arme(100);
    assert!(borne.commence_balayage(100));

    // Publication concurrente pendant que le scanner calcule son minimum.
    borne.arme(150);
    borne.recale(900);
    assert_eq!(borne.borne(), 150);
}

#[test]
fn un_seul_cpu_revendique_un_balayage_du() {
    use std::sync::{Arc, Barrier};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    let borne = Arc::new(Echeances::neuve());
    borne.arme(100);
    let depart = Arc::new(Barrier::new(4));
    let gagnants = Arc::new(AtomicUsize::new(0));
    let mut fils = Vec::new();
    for _ in 0..4 {
        let borne = Arc::clone(&borne);
        let depart = Arc::clone(&depart);
        let gagnants = Arc::clone(&gagnants);
        fils.push(thread::spawn(move || {
            depart.wait();
            if borne.commence_balayage(100) {
                gagnants.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for fil in fils {
        fil.join().unwrap();
    }
    assert_eq!(gagnants.load(Ordering::Relaxed), 1);
}

/// L'echeance exacte compte comme due : `>=`, pas `>`. Un `poll` de zero
/// milliseconde doit rendre la main tout de suite.
#[test]
fn l_echeance_exacte_est_due() {
    let borne = Echeances::neuve();
    borne.arme(1_000);
    assert!(!borne.doit_balayer(999));
    assert!(borne.doit_balayer(1_000));
}

/// Le cas qui a motive tout : un fil bloque sans echeance ne doit pas empecher
/// les autres d'etre servis, ni forcer un balayage permanent.
#[test]
fn un_dormeur_sans_echeance_ne_force_aucun_balayage() {
    let borne = Echeances::neuve();
    let mut modele = Modele::default();
    // Deux fils sur un futex sans delai : `wake_deadline_ns == 0`.
    modele.arme(0, 0);
    modele.arme(1, 0);
    borne.arme(0);
    borne.arme(0);
    for tick in 0..10_000u64 {
        assert!(!borne.doit_balayer(tick * 1_000_000),
            "un futex sans delai ne doit rien faire balayer");
    }
    assert!(modele.attentes.is_empty(), "zero n'entre pas dans le modele");
}
