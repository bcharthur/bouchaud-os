//! Le protocole de parking peut-il perdre un reveil ?
//!
//! # Ce qui est en jeu
//!
//! Une file d'attente du noyau enregistrait son dormeur, relisait sa
//! generation, puis le declarait bloque. L'ordre n'etait correct que grace au
//! gros verrou, qui empechait tout reveilleur de tourner entre les deux
//! derniers pas. Le retirer ouvre une fenetre reelle, et le symptome est le
//! pire possible : une tache qui ne repart JAMAIS, sans produire un seul
//! message. Cela se voit uniquement comme une interface qui se fige.
//!
//! # La propriete
//!
//! Pour tout entrelacement de dormeurs et de reveilleurs :
//!
//!     aucun dormeur ne reste gare apres le dernier signal
//!
//! Le test la met a l'epreuve avec de vrais fils, sur le MEME code que le
//! noyau execute -- pas sur une reecriture, qui divergerait en silence.
//!
//! Lance par `tools/dev/validate-fast.ps1` et la barriere courte.

#[path = "../../src/kernel/sync/rendezvous.rs"]
mod rendezvous;

use rendezvous::{Dormeur, Rendezvous};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

const PRET: u8 = 0;
const GARE: u8 = 1;

/// L'etat d'un dormeur, avec la meme discipline d'ordre que `EtatAtomique`
/// dans le noyau : publication SEQUENTIELLE, transition par compare_exchange.
struct EtatTest {
    etat: AtomicU8,
    reveils: AtomicU64,
    annulations: AtomicU64,
}

impl EtatTest {
    fn neuf() -> Self {
        Self {
            etat: AtomicU8::new(PRET),
            reveils: AtomicU64::new(0),
            annulations: AtomicU64::new(0),
        }
    }
    fn est_gare(&self) -> bool {
        self.etat.load(Ordering::SeqCst) == GARE
    }
}

impl Dormeur for EtatTest {
    fn publie_parking(&self) {
        self.etat.store(GARE, Ordering::SeqCst);
    }
    fn tente_reveil(&self) -> bool {
        let gagne = self
            .etat
            .compare_exchange(GARE, PRET, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if gagne {
            self.reveils.fetch_add(1, Ordering::Relaxed);
        }
        gagne
    }
    fn annule_parking(&self) {
        self.etat.store(PRET, Ordering::SeqCst);
        self.annulations.fetch_add(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// 1. La fenetre exacte, forcee au lieu d'etre esperee
// ---------------------------------------------------------------------------

/// Un dormeur qui laisse le reveilleur passer AU MOMENT PRECIS ou le protocole
/// s'apprete a publier son parking.
///
/// # Pourquoi un entrelacement force
///
/// La premiere version de ce test lancait simplement deux fils et repetait
/// deux mille fois. Elle passait aussi bien avec le BON ordre qu'avec le
/// MAUVAIS -- la fenetre ne fait que quelques instructions, et on ne tombe
/// jamais dedans par hasard. Un test qui ne distingue pas les deux ne prouve
/// rien, quel que soit le nombre de tours.
///
/// Ici le dormeur ARRETE le protocole juste avant sa publication, laisse le
/// reveilleur faire tout son travail, puis reprend. C'est l'entrelacement le
/// plus defavorable, et il est atteint a chaque execution.
struct DormeurSynchronise {
    etat: AtomicU8,
    reveils: AtomicU64,
    annulations: AtomicU64,
    /// Passe a vrai quand le reveilleur a le droit de courir.
    liberer: Arc<AtomicU8>,
    /// Passe a vrai quand il a fini.
    termine: Arc<AtomicU8>,
}

impl Dormeur for DormeurSynchronise {
    fn publie_parking(&self) {
        // Le reveilleur court MAINTENANT, avant que l'etat ne passe a « gare ».
        self.liberer.store(1, Ordering::SeqCst);
        while self.termine.load(Ordering::SeqCst) == 0 {
            std::hint::spin_loop();
        }
        self.etat.store(GARE, Ordering::SeqCst);
    }
    fn tente_reveil(&self) -> bool {
        let gagne = self
            .etat
            .compare_exchange(GARE, PRET, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if gagne {
            self.reveils.fetch_add(1, Ordering::Relaxed);
        }
        gagne
    }
    fn annule_parking(&self) {
        self.etat.store(PRET, Ordering::SeqCst);
        self.annulations.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn le_reveil_ne_se_perd_pas_dans_la_fenetre_de_publication() {
    // L'entrelacement exact que le gros verrou rendait impossible :
    //
    //   dormeur      s'inscrit, prend son ticket
    //   reveilleur   incremente la generation, tente le reveil : ECHOUE,
    //                le dormeur n'est pas encore gare
    //   dormeur      publie « gare »
    //
    // Avec l'ancien ordre -- relire la generation AVANT de publier -- le
    // dormeur ne voit rien de neuf, se gare, et ne repart jamais. Avec le bon
    // ordre, la relecture vient apres et voit la generation changee.
    let point = Arc::new(Rendezvous::neuf());
    let liberer = Arc::new(AtomicU8::new(0));
    let termine = Arc::new(AtomicU8::new(0));
    let dormeur = Arc::new(DormeurSynchronise {
        etat: AtomicU8::new(PRET),
        reveils: AtomicU64::new(0),
        annulations: AtomicU64::new(0),
        liberer: Arc::clone(&liberer),
        termine: Arc::clone(&termine),
    });

    let ticket = point.ticket();
    point.inscrit();

    let signaleur = {
        let point = Arc::clone(&point);
        let dormeur = Arc::clone(&dormeur);
        let termine = Arc::clone(&termine);
        std::thread::spawn(move || {
            while liberer.load(Ordering::SeqCst) == 0 {
                std::hint::spin_loop();
            }
            point.signale(usize::MAX, core::iter::once(&*dormeur));
            termine.store(1, Ordering::SeqCst);
        })
    };

    let doit_dormir = point.doit_dormir(ticket, &*dormeur);
    signaleur.join().unwrap();
    point.desinscrit();

    assert!(
        !doit_dormir,
        "le protocole veut dormir alors que le signal est deja passe : c'est \
         le reveil perdu, et il se voit comme une tache qui ne repart jamais",
    );
    assert_eq!(
        dormeur.etat.load(Ordering::SeqCst), PRET,
        "le dormeur reste gare apres le signal",
    );
    assert_eq!(
        dormeur.annulations.load(Ordering::Relaxed), 1,
        "la publication devait etre annulee par la relecture de generation",
    );
}

// ---------------------------------------------------------------------------
// 2. La propriete centrale, sous concurrence reelle
// ---------------------------------------------------------------------------

#[test]
fn aucun_dormeur_ne_reste_gare_apres_le_dernier_signal() {
    // Un dormeur et un reveilleur qui se croisent des milliers de fois. C'est
    // la fenetre exacte que le gros verrou fermait : entre la publication du
    // parking et la relecture de la generation.
    for tour in 0..2_000u64 {
        let point = Arc::new(Rendezvous::neuf());
        let etat = Arc::new(EtatTest::neuf());

        let ticket = point.ticket();
        point.inscrit();

        let signaleur = {
            let point = Arc::clone(&point);
            let etat = Arc::clone(&etat);
            std::thread::spawn(move || {
                // Un decalage variable pour balayer l'entrelacement plutot que
                // de retomber toujours sur le meme.
                for _ in 0..(tour % 7) {
                    std::hint::spin_loop();
                }
                point.signale(1, core::iter::once(&*etat));
            })
        };

        let doit_dormir = point.doit_dormir(ticket, &*etat);
        signaleur.join().unwrap();
        point.desinscrit();

        assert!(
            !etat.est_gare(),
            "tour {tour} : le dormeur reste gare apres le signal -- c'est le \
             reveil perdu, et il se voit comme une tache qui ne repart jamais",
        );
        // AU MOINS un des deux chemins a agi. Les deux peuvent agir, et c'est
        // correct : le reveilleur gagne la transition, puis le dormeur voit la
        // generation changee et annule sa publication. L'annulation est alors
        // une ecriture redondante de l'etat deja rendu -- sans effet.
        //
        // Ce qui serait fautif est qu'AUCUN des deux n'agisse : le dormeur
        // resterait gare sans que personne ne l'ait reveille.
        assert!(
            etat.reveils.load(Ordering::Relaxed)
                + etat.annulations.load(Ordering::Relaxed)
                >= 1,
            "tour {tour} : ni reveil ni annulation (doit_dormir={doit_dormir})",
        );
    }
}

#[test]
fn plusieurs_dormeurs_plusieurs_reveilleurs() {
    const DORMEURS: usize = 8;
    for tour in 0..300u64 {
        let point = Arc::new(Rendezvous::neuf());
        let etats: Arc<Vec<EtatTest>> =
            Arc::new((0..DORMEURS).map(|_| EtatTest::neuf()).collect());

        let ticket = point.ticket();
        for _ in 0..DORMEURS {
            point.inscrit();
        }

        let mut fils = Vec::new();
        for _ in 0..3 {
            let point = Arc::clone(&point);
            let etats = Arc::clone(&etats);
            fils.push(std::thread::spawn(move || {
                for _ in 0..(tour % 5) {
                    std::hint::spin_loop();
                }
                point.signale(usize::MAX, etats.iter());
            }));
        }
        for index in 0..DORMEURS {
            point.doit_dormir(ticket, &etats[index]);
        }
        for f in fils {
            f.join().unwrap();
        }
        for _ in 0..DORMEURS {
            point.desinscrit();
        }

        for (index, etat) in etats.iter().enumerate() {
            assert!(
                !etat.est_gare(),
                "tour {tour} : dormeur {index} reste gare",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Exactement un gagnant par reveil
// ---------------------------------------------------------------------------

#[test]
fn deux_reveilleurs_ne_reveillent_pas_deux_fois_la_meme_tache() {
    // Sans compare_exchange, les deux verraient la tache garee et la mettraient
    // tous les deux en file d'execution : elle serait ordonnancee deux fois.
    for _ in 0..2_000 {
        let point = Arc::new(Rendezvous::neuf());
        let etat = Arc::new(EtatTest::neuf());
        point.inscrit();
        etat.publie_parking();

        let mut fils = Vec::new();
        for _ in 0..4 {
            let point = Arc::clone(&point);
            let etat = Arc::clone(&etat);
            fils.push(std::thread::spawn(move || {
                point.signale(usize::MAX, core::iter::once(&*etat))
            }));
        }
        let total: usize = fils.into_iter().map(|f| f.join().unwrap()).sum();
        point.desinscrit();

        assert_eq!(total, 1, "exactement un reveilleur doit gagner");
        assert_eq!(etat.reveils.load(Ordering::Relaxed), 1);
    }
}

// ---------------------------------------------------------------------------
// 3. Les cas simples restent justes
// ---------------------------------------------------------------------------

#[test]
fn un_ticket_perime_ne_dort_pas() {
    let point = Rendezvous::neuf();
    let etat = EtatTest::neuf();
    let ticket = point.ticket();
    point.inscrit();
    // Le signal passe AVANT que le dormeur ne se gare.
    point.signale(usize::MAX, core::iter::once(&etat));

    assert!(
        !point.doit_dormir(ticket, &etat),
        "un ticket perime doit refuser le parking, pas dormir sur un reveil \
         deja passe",
    );
    assert!(!etat.est_gare());
    assert_eq!(etat.annulations.load(Ordering::Relaxed), 1);
}

#[test]
fn sans_dormeur_le_signal_ne_reveille_personne() {
    let point = Rendezvous::neuf();
    let etat = EtatTest::neuf();
    assert_eq!(point.signale(usize::MAX, core::iter::once(&etat)), 0);
    assert_eq!(point.dormeurs(), 0);
    // Et la generation a tout de meme avance. C'est essentiel : un signal qui
    // sortirait sans l'incrementer parce qu'il ne voit aucun dormeur laisserait
    // valide le ticket d'un dormeur sur le point de s'inscrire -- qui
    // s'endormirait alors sur un reveil deja passe.
    assert!(
        point.ticket() > 1,
        "la generation doit avancer AVANT la lecture du compte de dormeurs",
    );
}

#[test]
fn la_limite_de_reveil_est_respectee() {
    let point = Rendezvous::neuf();
    let etats: Vec<EtatTest> = (0..5).map(|_| EtatTest::neuf()).collect();
    for etat in &etats {
        point.inscrit();
        etat.publie_parking();
    }
    assert_eq!(point.signale(2, etats.iter()), 2, "wake_one ne doit pas tout reveiller");
    assert_eq!(etats.iter().filter(|e| e.est_gare()).count(), 3);
}

#[test]
fn l_inscription_se_defait_exactement() {
    let point = Rendezvous::neuf();
    assert_eq!(point.dormeurs(), 0);
    point.inscrit();
    point.inscrit();
    assert_eq!(point.dormeurs(), 2);
    point.desinscrit();
    point.desinscrit();
    assert_eq!(point.dormeurs(), 0);
}
