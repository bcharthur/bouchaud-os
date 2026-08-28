//! La comptabilite du gros verrou peut-elle encore annoncer 183 % ?
//!
//! # Ce qui a rendu un releve entier inutilisable
//!
//! ```text
//! [BKL-SYSCALL] window_ns=5000000000 poll=[hold_delta_ns=9166000000 hold_pct=183 ...]
//! [BKL-MAX-HOLD] ns=29562372510 origine=resume_after_schedule
//! ```
//!
//! 183 % du temps ecoule passe a detenir un verrou EXCLUSIF. Le chiffre n'est
//! pas « imprecis », il est IMPOSSIBLE : a tout instant il y a au plus un
//! proprietaire, donc la somme des tenues est majoree par le temps qui passe.
//! Et un chiffre impossible ne se corrige pas -- il retire toute valeur aux
//! autres chiffres du meme releve, y compris a la pointe de 29 secondes qui,
//! elle, designait un vrai figement.
//!
//! # La propriete que ce test etablit
//!
//! Pour toute suite d'acquisitions et de liberations d'un verrou EXCLUSIF,
//! horodatee par une horloge monotone :
//!
//!     tenue_ns  <=  temps ecoule
//!
//! Ce n'est pas une esperance mais une consequence : la sonde de liberation
//! s'executant avant que le verrou ne redevienne libre, les intervalles
//! factures sont deux a deux disjoints. `hold_pct <= 100` devient alors une
//! propriete de la structure, et non un resultat a verifier apres coup.
//!
//! Le test la soumet a de vrais fils concurrents, puis rejoue la sequence
//! exacte qui faisait deborder l'ANCIEN schema -- une continuation qui migre,
//! verrou en main -- pour montrer qu'il depassait bien 100 % la ou celui-ci
//! reste juste.
//!
//! Lance par `tools/dev/validate-fast.ps1`.

#[path = "../../src/kernel/sync/bkl_compte.rs"]
mod bkl_compte;

use bkl_compte::{Comptes, AUCUN, MAX_CPUS};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// L'horloge du noyau : monotone GLOBALEMENT, y compris entre CPU dont les TSC
/// derivent. `monotonic_ns` obtient cela par un `fetch_max` ; on le reproduit,
/// parce que la majoration ci-dessus repose entierement dessus. Une horloge qui
/// recule ferait mesurer un intervalle plus long que le temps ecoule.
struct Horloge {
    origine: Instant,
    dernier: AtomicU64,
}

impl Horloge {
    fn neuve() -> Self {
        Self { origine: Instant::now(), dernier: AtomicU64::new(0) }
    }

    fn maintenant(&self) -> u64 {
        let brut = self.origine.elapsed().as_nanos() as u64;
        brut.max(self.dernier.fetch_max(brut, Ordering::AcqRel))
    }
}

// ---------------------------------------------------------------------------
// 1. L'invariant, sous concurrence reelle
// ---------------------------------------------------------------------------

#[test]
fn la_somme_des_tenues_ne_depasse_jamais_le_temps_ecoule() {
    let comptes = Arc::new(Comptes::neuf());
    let horloge = Arc::new(Horloge::neuve());
    // Le verrou lui-meme. C'est son EXCLUSIVITE qui rend les intervalles
    // disjoints ; sans elle la propriete serait fausse, et c'est pour cela que
    // le test le modelise plutot que de se contenter d'appeler les sondes.
    let verrou = Arc::new(Mutex::new(()));

    let debut = horloge.maintenant();

    let mut fils = Vec::new();
    for cpu in 0..4usize {
        let comptes = Arc::clone(&comptes);
        let horloge = Arc::clone(&horloge);
        let verrou = Arc::clone(&verrou);
        fils.push(std::thread::spawn(move || {
            for tour in 0..5_000u64 {
                let garde = verrou.lock().unwrap();
                comptes.ouvre(horloge.maintenant(), (tour % 17) as usize, cpu, 1);
                // Une section critique de longueur variable : c'est le
                // recouvrement de deux intervalles qui ferait deborder la
                // somme, et il faut lui laisser toutes ses chances.
                std::hint::black_box(tour.wrapping_mul(2654435761));
                // La sonde ferme AVANT que le verrou ne soit rendu, exactement
                // comme `probe_note_release` s'execute avant `OWNER <- FREE`.
                let tenue = comptes.ferme(horloge.maintenant(), cpu);
                drop(garde);

                comptes.note_attente(17, 1, cpu, (tour % 17) as usize);
                let tenue = tenue.expect("toute acquisition a sa liberation");
                assert_eq!(tenue.cpu_acquisition, cpu);
                assert_eq!(tenue.cpu_liberation, cpu);
                assert_eq!(tenue.seau, (tour % 17) as usize);
            }
        }));
    }
    for f in fils {
        f.join().unwrap();
    }

    let ecoule = horloge.maintenant() - debut;
    let tenue = comptes.tenue_ns();
    assert!(
        tenue <= ecoule,
        "somme des tenues {tenue} ns > temps ecoule {ecoule} ns : c'est le \
         `hold_pct` superieur a 100 % qui revient",
    );

    let (sans_debut, sur_tenue, rebours) = comptes.anomalies();
    assert_eq!(
        (sans_debut, sur_tenue, rebours), (0, 0, 0),
        "une suite bien formee ne doit declencher aucune anomalie",
    );
    assert_eq!(comptes.liberations_migrees(), 0);
}

// ---------------------------------------------------------------------------
// 2. Le temoin : l'ANCIEN schema depassait, celui-ci non
// ---------------------------------------------------------------------------

/// L'ancienne comptabilite, reproduite a l'identique.
///
/// Un horodatage PAR CPU, et une addition placee HORS du test qui verifiait
/// qu'un intervalle etait bien ouvert :
///
/// ```text
/// let acquired = ACQUIRED_AT_NS[cpu].swap(0);
/// let tenue = now.saturating_sub(acquired);
/// TOTAL_HOLD_NS.fetch_add(tenue);      // <- hors du `if acquired != 0`
/// if acquired != 0 { ... }
/// ```
#[derive(Default)]
struct AncienSchema {
    par_cpu: [u64; MAX_CPUS],
    total: u64,
    max: u64,
}

impl AncienSchema {
    fn ouvre(&mut self, maintenant: u64, cpu: usize) {
        self.par_cpu[cpu] = maintenant;
    }

    fn ferme(&mut self, maintenant: u64, cpu: usize) {
        let acquis = core::mem::replace(&mut self.par_cpu[cpu], 0);
        let tenue = maintenant.saturating_sub(acquis);
        self.total += tenue;
        if acquis != 0 && tenue > self.max {
            self.max = tenue;
        }
    }
}

/// La sequence exacte que produit une continuation qui MIGRE verrou en main.
///
/// Le noyau la fabrique tout seul : une pile suspendue sur un coeur reprend sur
/// un autre, et `KernelGuard::drop` libere le coeur COURANT. L'acquisition est
/// alors notee sur un CPU et la liberation sur un autre.
///
///   t=1000  acquisition sur le CPU 0
///   t=1100  liberation  sur le CPU 1   <- 100 ns reellement tenues
///   t=9000  liberation  sur le CPU 0   <- une autre tenue, plus tard
///
/// L'ancien schema facture 1100 ns pour la premiere -- `9000 - 0`, c'est-a-dire
/// tout le temps depuis le demarrage -- puis 8000 ns pour la seconde, en
/// consommant la case laissee en place par la premiere. Total : 9100 ns sur une
/// fenetre de 9000. Cent ns reellement tenues, lues comme 101 %.
#[test]
fn une_continuation_qui_migre_faisait_deborder_l_ancien_schema() {
    let fenetre = 9_000u64;

    let mut ancien = AncienSchema::default();
    ancien.ouvre(1_000, 0);
    ancien.ferme(1_100, 1);
    ancien.ferme(9_000, 0);

    assert!(
        ancien.total > fenetre,
        "le temoin doit reproduire le debordement : {} ns sur une fenetre de \
         {} ns", ancien.total, fenetre,
    );
    assert_eq!(ancien.total, 9_100);
    // Et la pointe : 8 microsecondes publiees pour une tenue qui n'a jamais eu
    // lieu. A l'echelle du runtime, c'est la ligne `ns=29562372510`.
    assert_eq!(ancien.max, 8_000);

    let neuf = Comptes::neuf();
    neuf.ouvre(1_000, 3, 0, 1);
    let tenue = neuf.ferme(1_100, 1).expect("l'intervalle etait ouvert");
    assert_eq!(tenue.ns, 100, "seule la duree REELLEMENT tenue est facturee");
    assert_eq!(tenue.cpu_acquisition, 0);
    assert_eq!(tenue.cpu_liberation, 1);
    assert_eq!(tenue.seau, 3, "le seau voyage avec l'intervalle, pas avec le CPU");
    assert_eq!(
        neuf.liberations_migrees(), 1,
        "la migration doit etre VISIBLE, et non deduite d'un chiffre absurde",
    );

    // La liberation orpheline ne facture rien : elle se compte.
    assert!(neuf.ferme(9_000, 0).is_none());
    assert_eq!(neuf.tenue_ns(), 100);
    assert!(
        neuf.tenue_ns() <= fenetre,
        "meme sur la sequence qui cassait l'ancien schema, la somme reste \
         majoree par la fenetre",
    );
    assert_eq!(neuf.anomalies().0, 1, "l'orpheline est comptee, pas absorbee");
}

// ---------------------------------------------------------------------------
// 3. Aucune anomalie n'est absorbee en silence
// ---------------------------------------------------------------------------

#[test]
fn une_liberation_sans_debut_ne_facture_rien() {
    let c = Comptes::neuf();
    assert!(c.ferme(5_000_000_000, 0).is_none());
    assert_eq!(
        c.tenue_ns(), 0,
        "c'est ICI que l'ancien code ajoutait `maintenant - 0`, soit tout le \
         temps depuis le demarrage",
    );
    assert_eq!(c.anomalies(), (1, 0, 0));
}

#[test]
fn une_acquisition_par_dessus_une_tenue_ouverte_ne_compte_pas_deux_fois() {
    let c = Comptes::neuf();
    c.ouvre(1_000, 0, 0, 1);
    c.ouvre(2_000, 0, 1, 1); // le modele s'est decroche : on l'annonce
    assert_eq!(c.anomalies(), (0, 1, 0));

    let tenue = c.ferme(2_500, 1).unwrap();
    assert_eq!(
        tenue.ns, 500,
        "seul le dernier intervalle est facture ; additionner les deux \
         compterait deux fois le meme temps de muraille",
    );
    assert_eq!(c.tenue_ns(), 500);
}

#[test]
fn une_horloge_a_rebours_est_signalee_et_ne_facture_rien() {
    let c = Comptes::neuf();
    c.ouvre(2_000, 0, 0, 1);
    assert!(c.ferme(1_000, 0).is_none());
    assert_eq!(c.tenue_ns(), 0);
    assert_eq!(c.anomalies(), (0, 0, 1));
    // Et l'intervalle est bien referme : la valeur fautive ne reste pas en
    // place a empoisonner la liberation suivante.
    assert_eq!(c.proprietaire(), AUCUN);
    assert!(c.ferme(3_000, 0).is_none());
    assert_eq!(c.anomalies(), (1, 0, 1));
}

#[test]
fn un_horodatage_nul_ne_se_confond_pas_avec_l_absence_de_tenue() {
    // `0` sert de sentinelle « aucune tenue en cours ». Une acquisition a
    // l'instant zero -- le tout premier `enter` du demarrage -- ne doit pas
    // s'effacer elle-meme.
    let c = Comptes::neuf();
    c.ouvre(0, 0, 0, 1);
    assert_ne!(c.proprietaire(), AUCUN);
    let tenue = c.ferme(1_000, 0).expect("l'intervalle etait bien ouvert");
    assert_eq!(tenue.ns, 999, "a une nanoseconde pres, celle de la sentinelle");
    assert_eq!(c.anomalies(), (0, 0, 0));
}

// ---------------------------------------------------------------------------
// 4. Les grandeurs sont separees, et disent qui attend sur qui
// ---------------------------------------------------------------------------

#[test]
fn la_reprise_est_isolee_de_l_attente_globale() {
    let c = Comptes::neuf();
    // Un `enter` ordinaire : de l'attente, pas de la reprise.
    c.note_attente(300, 1, 0, 7);
    // Une reprise apres commutation : les deux.
    c.note_attente(9_000_000_000, 3, 2, 23);
    c.note_reprise(9_000_000_000);
    c.note_attente(50, 3, 1, 23);
    c.note_reprise(50);

    assert_eq!(c.attente_ns(), 9_000_000_350);
    assert_eq!(c.reprise_ns(), 9_000_000_050);
    assert_eq!(
        c.reprise_max_ns(), 9_000_000_000,
        "c'est le MAXIMUM qui distingue un noyau qui travaille d'un noyau qui \
         gele : neuf secondes dans une seule reprise ne se voient pas dans un \
         cumul",
    );
    assert!(c.reprise_ns() <= c.attente_ns());
    assert_eq!(c.tenue_ns(), 0, "attendre n'est pas detenir");

    // Les spins comptent des TOURS, jamais du temps : un spin sous TCG ne dure
    // pas ce qu'il dure sur du materiel, et le convertir en nanosecondes
    // fabriquerait une quatrieme duree qui ne voudrait rien dire.
    c.note_spin();
    c.note_spin();
    assert_eq!(c.spins(), 2);
}

#[test]
fn la_plus_longue_attente_garde_son_contexte_avec_elle() {
    // « Aucune acquisition ne doit prendre plus de 50 ms » est un critere sur
    // le MAXIMUM. Un cumul ne peut pas y repondre : mille attentes d'une
    // microseconde et une attente de deux secondes le laissent identique.
    let c = Comptes::neuf();
    c.note_attente(1_000, 1, 0, 7);
    c.note_attente(2_000_000_000, 3, 2, 23);
    c.note_attente(5_000, 1, 1, 7);

    let (ns, origine, cpu, seau) = c.attente_max();
    assert_eq!(ns, 2_000_000_000);
    assert_eq!(
        (origine, cpu, seau), (3, 2, 23),
        "duree et contexte doivent designer LA MEME attente : une duree juste          avec le mauvais coupable fait chercher au mauvais endroit",
    );
    assert_eq!(c.attente_ns(), 2_000_006_000, "le cumul reste, il repond a une                                                 autre question");
}

#[test]
fn le_contexte_d_une_attente_survit_aux_valeurs_extremes() {
    // Les trois champs partagent un mot de 64 bits. Un empaquetage errone ne
    // se verrait qu'a la lecture d'un journal, sur un chiffre plausible.
    let c = Comptes::neuf();
    c.note_attente(1, 3, MAX_CPUS - 1, 511);
    assert_eq!(c.attente_max(), (1, 3, MAX_CPUS - 1, 511));

    let d = Comptes::neuf();
    d.note_attente(u64::MAX, 0, 0, 0);
    assert_eq!(d.attente_max(), (u64::MAX, 0, 0, 0));
}

#[test]
fn la_ventilation_dit_sur_qui_on_attend_et_qui_recoit_les_reveils() {
    let c = Comptes::neuf();
    for _ in 0..7 { c.note_park(2); }
    c.note_park(0);
    // Verrou deja libre au moment du parking : une course benigne, mais qui a
    // sa propre case plutot que de polluer celle d'un CPU.
    c.note_park(AUCUN);

    assert_eq!(c.parks(), 9);
    assert_eq!(c.parks_sur(2), 7, "le CPU 2 est celui qu'on attend");
    assert_eq!(c.parks_sur(0), 1);
    assert_eq!(c.parks_sur(MAX_CPUS), 1, "la case « deja libre »");

    c.note_wake(1);
    c.note_wake(1);
    c.note_wake(3);
    assert_eq!(c.wake_ipis(), 3);
    assert_eq!(c.wakes_vers(1), 2);
    assert_eq!(c.wakes_vers(3), 1);
}

#[test]
fn les_reveils_improductifs_mesurent_le_troupeau() {
    let c = Comptes::neuf();
    // Quatre CPU gares, un seul gagne : trois reveils n'ont rien produit.
    c.note_reveils_improductifs(3);
    c.note_reveils_improductifs(0);
    assert_eq!(c.reveils_sans_acquisition(), 3);
}

#[test]
fn un_index_de_cpu_hors_bornes_ne_deborde_aucun_tableau() {
    // La provenance vient desormais de l'INTERVALLE, qui peut porter `AUCUN`.
    // Un tableau indexe sans precaution paniquerait dans le noyau, au milieu
    // d'une liberation de verrou -- le pire endroit possible.
    let c = Comptes::neuf();
    c.note_park(usize::MAX);
    c.note_wake(usize::MAX);
    assert_eq!(c.parks_sur(usize::MAX), 1);
    let _ = c.wakes_vers(usize::MAX);
}
