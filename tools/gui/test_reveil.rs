//! Preuve hote de la politique event-driven du compositeur (Gate 1B).
//!
//! # Deux choses distinctes sont testees
//!
//! 1. LA POLITIQUE -- `src/gui/politique.rs`, inclus tel quel. Quand composer,
//!    quand dormir, jusqu'a quand. C'est de l'arithmetique sur des dates : elle
//!    n'a besoin ni de framebuffer, ni de clavier, ni d'ordonnanceur.
//!
//! 2. LE PROTOCOLE DE REVEIL -- modelise ici, parce que le vrai vit sur une
//!    `WaitQueue` noyau qui ne se compile pas sur l'hote. Le modele rejoue le
//!    MEME enchainement : billet pris avant lecture d'etat, generation
//!    incrementee par le producteur, refus de dormir sur un billet perime.
//!
//! La fenetre qui casse tous les compositeurs event-driven est celle-ci :
//!
//! ```text
//!     constate : rien a faire
//!         <-- EVENEMENT ARRIVE ICI
//!     s'endort  -> plus jamais reveille
//! ```
//!
//! Elle est rejouee A LA MAIN, sans fil ni hasard : ces tests ne clignotent pas.
//!
//! Lance par `tools/gui/test-reveil.sh`.

#[path = "../../src/gui/politique.rs"]
mod politique;

use politique::{
    doit_composer, doit_rafraichir_horloge, doit_recomposer_aveugle, duree_sommeil_ms,
    prochaine_echeance, Etat, PERIODE_HORLOGE_MS, PERIODE_RELEVE_MS, PERIODE_TRAME_MS,
    REACTIVITE_MUETTE_MS, REPOS_MUET_MS,
};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

// ===========================================================================
// 1. La politique
// ===========================================================================

/// Un bureau parfaitement immobile, sans horloge ni client muet, n'a AUCUNE
/// raison de se reveiller. C'est la propriete que Gate 1B rend atteignable.
#[test]
fn sans_horloge_ni_client_muet_le_sommeil_n_a_pas_de_fin() {
    let etat = Etat {
        maintenant_ms: 10_000,
        sale: false,
        client_muet_visible: false,
        horloge_visible: false,
        dernier_releve_ms: u64::MAX - PERIODE_RELEVE_MS, // releve tres lointain
        ..Default::default()
    };
    // Seul le releve de charge subsiste, et il est volontairement repousse ici
    // pour isoler la propriete : rien d'AFFICHE ne demande de reveil.
    let echeance = prochaine_echeance(&etat);
    assert!(
        echeance.is_none() || echeance.unwrap() > etat.maintenant_ms + 1_000_000,
        "rien ne change tout seul : aucune echeance proche, obtenu {echeance:?}"
    );
}

/// Aucun degat => aucune trame demandee. La regle de base de Gate 1B.
#[test]
fn aucun_degat_aucune_trame() {
    let etat = Etat {
        maintenant_ms: 1_000,
        sale: false,
        derniere_trame_ms: 0, // largement au-dela du creneau
        ..Default::default()
    };
    assert!(!doit_composer(&etat), "sans degat, rien a composer");
}

/// Un degat, mais le creneau de trame n'est pas atteint : on differe, et
/// l'echeance existe pour qu'on y revienne.
#[test]
fn un_degat_avant_le_creneau_est_differe_mais_planifie() {
    let etat = Etat {
        maintenant_ms: 1_005,
        sale: true,
        derniere_trame_ms: 1_000,
        horloge_visible: true,
        derniere_horloge_ms: 1_000,
        dernier_releve_ms: 1_000,
        ..Default::default()
    };
    assert!(!doit_composer(&etat), "5 ms apres la derniere trame : trop tot");
    assert_eq!(
        prochaine_echeance(&etat),
        Some(1_000 + PERIODE_TRAME_MS),
        "mais le reveil est planifie au creneau, pas repousse a l'horloge"
    );
}

/// Le creneau atteint, on compose.
#[test]
fn un_degat_au_creneau_compose() {
    let etat = Etat {
        maintenant_ms: 1_000 + PERIODE_TRAME_MS,
        sale: true,
        derniere_trame_ms: 1_000,
        ..Default::default()
    };
    assert!(doit_composer(&etat));
    assert_eq!(duree_sommeil_ms(&etat), Some(0), "echeance atteinte : ne pas dormir");
}

/// L'horloge est la seule animation permanente : elle impose un reveil par
/// seconde, et rien de plus.
#[test]
fn l_horloge_impose_un_reveil_par_seconde_et_rien_de_plus() {
    let etat = Etat {
        maintenant_ms: 5_000,
        sale: false,
        horloge_visible: true,
        derniere_horloge_ms: 5_000,
        dernier_releve_ms: 5_000,
        ..Default::default()
    };
    assert_eq!(prochaine_echeance(&etat), Some(5_000 + PERIODE_HORLOGE_MS));
    assert!(!doit_rafraichir_horloge(&etat), "elle vient d'etre rafraichie");

    let plus_tard = Etat { maintenant_ms: 5_000 + PERIODE_HORLOGE_MS, ..etat };
    assert!(doit_rafraichir_horloge(&plus_tard));
}

/// Un client muet impose la seule autre forme de polling qui subsiste -- et
/// elle disparait des qu'il n'est plus visible.
#[test]
fn un_client_muet_impose_une_echeance_et_lui_seul() {
    let base = Etat {
        maintenant_ms: 2_000,
        dernier_aveugle_ms: 2_000,
        derniere_entree_ms: 0, // aucune entree recente : cadence de veille
        dernier_releve_ms: 2_000,
        ..Default::default()
    };

    let muet = Etat { client_muet_visible: true, ..base };
    assert_eq!(prochaine_echeance(&muet), Some(2_000 + REPOS_MUET_MS));

    let sans_muet = Etat { client_muet_visible: false, ..base };
    assert_eq!(
        prochaine_echeance(&sans_muet),
        Some(2_000 + PERIODE_RELEVE_MS),
        "sans client muet, la recomposition aveugle disparait completement"
    );
}

/// La recomposition « a l'aveugle » ne se declenche QUE pour un client muet
/// visible, et seulement une fois sa periode ecoulee.
#[test]
fn la_recomposition_aveugle_ne_concerne_que_les_clients_muets() {
    let echu = Etat {
        maintenant_ms: 2_000 + REPOS_MUET_MS,
        client_muet_visible: true,
        dernier_aveugle_ms: 2_000,
        derniere_entree_ms: 0,
        ..Default::default()
    };
    assert!(doit_recomposer_aveugle(&echu), "periode ecoulee : on recopie");

    let trop_tot = Etat { maintenant_ms: 2_000 + REPOS_MUET_MS - 1, ..echu };
    assert!(!doit_recomposer_aveugle(&trop_tot));

    let bavard = Etat { client_muet_visible: false, ..echu };
    assert!(
        !doit_recomposer_aveugle(&bavard),
        "un client qui annonce ses trames n'a pas besoin d'etre devine"
    );
}

/// Juste apres une entree, un client muet est recompose a pleine cadence.
#[test]
fn apres_une_entree_le_client_muet_repasse_a_pleine_cadence() {
    let etat = Etat {
        maintenant_ms: 3_000,
        client_muet_visible: true,
        dernier_aveugle_ms: 3_000,
        derniere_entree_ms: 3_000 - REACTIVITE_MUETTE_MS / 2,
        dernier_releve_ms: 3_000,
        ..Default::default()
    };
    assert_eq!(etat.periode_aveugle(), PERIODE_TRAME_MS);
    assert_eq!(prochaine_echeance(&etat), Some(3_000 + PERIODE_TRAME_MS));

    let calme = Etat {
        derniere_entree_ms: 3_000 - REACTIVITE_MUETTE_MS - 1,
        ..etat
    };
    assert_eq!(calme.periode_aveugle(), REPOS_MUET_MS);
}

/// Consommer l'evenement ramene au repos : plus de degat, plus de trame, et
/// l'echeance retombe sur la seule horloge.
#[test]
fn consommer_l_evenement_ramene_au_repos() {
    let occupe = Etat {
        maintenant_ms: 4_000,
        sale: true,
        horloge_visible: true,
        derniere_trame_ms: 4_000 - PERIODE_TRAME_MS,
        derniere_horloge_ms: 4_000,
        dernier_releve_ms: 4_000,
        ..Default::default()
    };
    assert!(doit_composer(&occupe));

    // Apres composition : `sale` retombe, `derniere_trame` avance.
    let repos = Etat { sale: false, derniere_trame_ms: 4_000, ..occupe };
    assert!(!doit_composer(&repos));
    assert_eq!(prochaine_echeance(&repos), Some(4_000 + PERIODE_HORLOGE_MS));
}

/// Aucune echeance ne doit jamais produire un sommeil de duree nulle repete :
/// une echeance atteinte rend 0, ce que la boucle traite en rebouclant.
#[test]
fn une_echeance_atteinte_ne_produit_pas_de_sommeil_nul() {
    let etat = Etat {
        maintenant_ms: 9_000,
        horloge_visible: true,
        derniere_horloge_ms: 9_000 - PERIODE_HORLOGE_MS,
        dernier_releve_ms: 9_000,
        ..Default::default()
    };
    assert_eq!(duree_sommeil_ms(&etat), Some(0));
}

// ===========================================================================
// 2. Le protocole de reveil
// ===========================================================================

/// Le modele du reveil noyau : generation, inscription, et le double-controle
/// qui interdit le reveil perdu.
///
/// C'est le MEME motif que `WaitQueue` : quatre acces `SeqCst` dont l'ordre
/// total rend impossible que le dormeur ne voie pas la nouvelle generation ET
/// que le producteur ne voie pas l'inscription.
///
/// ```text
///     dormeur                        producteur
///     -------                        ----------
///     inscription += 1  (SeqCst)     generation += 1  (SeqCst)
///     relire generation (SeqCst)     lire inscriptions (SeqCst)
/// ```
///
/// Le producteur ne RETIRE jamais une inscription -- c'est le dormeur qui la
/// rend, comme le fait `Inscription::drop` dans le noyau. Un producteur qui
/// remettrait le compteur a zero le ferait passer sous zero des que le dormeur
/// se retirerait a son tour.
struct Porte {
    generation: AtomicU64,
    /// Inscriptions en cours. Posee et retiree par le dormeur, seulement lue
    /// par le producteur.
    dormeurs: AtomicU64,
    /// Nombre de fois ou un producteur a VU une inscription. C'est la preuve
    /// qu'un dormeur endormi sera reveille.
    vus: AtomicU64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fin {
    /// Le billet etait perime : on n'a pas dormi.
    DejaSignale,
    /// On s'est reellement endormi, inscription posee.
    Signale,
}

impl Porte {
    fn neuve() -> Self {
        Self {
            generation: AtomicU64::new(0),
            dormeurs: AtomicU64::new(0),
            vus: AtomicU64::new(0),
        }
    }

    fn billet(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Cote producteur : incrementer PUIS lire les inscriptions. L'ordre compte.
    fn signale(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.dormeurs.load(Ordering::SeqCst) != 0 {
            self.vus.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Cote dormeur : s'inscrire PUIS relire la generation. L'ordre compte.
    fn attends(&self, billet: u64) -> Fin {
        if self.generation.load(Ordering::SeqCst) != billet {
            return Fin::DejaSignale;
        }
        self.dormeurs.fetch_add(1, Ordering::SeqCst);
        if self.generation.load(Ordering::SeqCst) != billet {
            // Signale entre le controle et l'inscription : repartir au lieu de
            // dormir en attendant un reveil qui n'aura plus lieu.
            self.dormeurs.fetch_sub(1, Ordering::SeqCst);
            return Fin::DejaSignale;
        }
        Fin::Signale
    }

    /// Le dormeur rend son inscription au reveil.
    fn se_retire(&self) {
        self.dormeurs.fetch_sub(1, Ordering::SeqCst);
    }

    fn dormeurs_en_attente(&self) -> u64 {
        self.dormeurs.load(Ordering::SeqCst)
    }

    fn vus(&self) -> u64 {
        self.vus.load(Ordering::SeqCst)
    }
}

/// LA fenetre. Jouee a la main : l'evenement arrive APRES le constat « rien a
/// faire » et AVANT la tentative de sommeil.
#[test]
fn un_evenement_dans_la_fenetre_check_sleep_n_est_pas_perdu() {
    let porte = Porte::neuve();

    // 1. le consommateur prend son billet AVANT de regarder son etat
    let billet = porte.billet();
    // 2. il constate qu'il n'a rien a faire
    let rien_a_faire = true;
    assert!(rien_a_faire);
    // 3. L'EVENEMENT ARRIVE ICI, dans la fenetre
    porte.signale();
    // 4. il tente de dormir
    assert_eq!(
        porte.attends(billet),
        Fin::DejaSignale,
        "le billet est perime : dormir maintenant perdrait l'evenement"
    );
    assert_eq!(porte.dormeurs_en_attente(), 0, "personne ne dort");
}

/// La meme sequence avec le billet pris TROP TARD -- c'est-a-dire le bug que
/// le protocole existe pour empecher. Le test le documente explicitement.
#[test]
fn un_billet_pris_apres_le_constat_perdrait_l_evenement() {
    let porte = Porte::neuve();

    let rien_a_faire = true;
    assert!(rien_a_faire);
    porte.signale(); // l'evenement arrive ici
    let billet_tardif = porte.billet(); // ... et le billet est pris APRES

    assert_eq!(
        porte.attends(billet_tardif),
        Fin::Signale,
        "le billet tardif est valide : le consommateur s'endort sur un \
         evenement deja arrive -- c'est le reveil perdu"
    );
    assert_eq!(porte.dormeurs_en_attente(), 1, "un dormeur, et plus rien pour le reveiller");
}

/// Plusieurs invalidations avant le reveil ne s'annulent pas : une seule suffit
/// a interdire le sommeil, et aucune n'est requise pour cela.
#[test]
fn plusieurs_invalidations_ne_se_perdent_pas() {
    let porte = Porte::neuve();
    let billet = porte.billet();
    for _ in 0..64 {
        porte.signale();
    }
    assert_eq!(porte.attends(billet), Fin::DejaSignale);
    assert_eq!(porte.generation.load(Ordering::SeqCst), 64, "toutes comptees");
}

/// Un signal ANTERIEUR au billet ne doit pas empecher de dormir : sinon le
/// consommateur tournerait sans fin apres le premier evenement de sa vie.
#[test]
fn un_signal_deja_consomme_n_empeche_pas_de_dormir() {
    let porte = Porte::neuve();
    porte.signale();
    let billet = porte.billet(); // pris APRES, donc l'evenement est consomme
    assert_eq!(porte.attends(billet), Fin::Signale, "il n'y a plus rien en attente");
}

/// Contre-epreuve sous vrais fils, sur la propriete exacte du protocole :
///
///     si le dormeur a decide de dormir, alors le producteur a VU son
///     inscription -- donc il le reveillera.
///
/// Les deux issues (dormir, ne pas dormir) sont correctes ; ce qui est interdit
/// est que les deux se ratent. Assertion a sens unique : elle ne peut echouer
/// que si le protocole est faux, donc elle ne clignote pas.
///
/// Les echecs sont COMPTES puis asserted apres les `join` : une assertion dans
/// le fil laisserait l'autre bloque sur sa barriere, et le test se figerait au
/// lieu d'echouer.
#[test]
fn sous_fils_concurrents_un_dormeur_est_toujours_vu() {
    const TOURS: u64 = 10_000;
    let porte = Arc::new(Porte::neuve());
    let depart = Arc::new(Barrier::new(2));
    let fin_de_tour = Arc::new(Barrier::new(2));

    let (p1, d1, f1) = (porte.clone(), depart.clone(), fin_de_tour.clone());
    let producteur = std::thread::spawn(move || {
        for _ in 0..TOURS {
            d1.wait();
            p1.signale();
            f1.wait();
        }
    });

    let (p2, d2, f2) = (porte.clone(), depart.clone(), fin_de_tour.clone());
    let consommateur = std::thread::spawn(move || {
        let mut sommeils = 0u64;
        let mut oublies = 0u64;
        for _ in 0..TOURS {
            // Billet pris AVANT le constat, comme dans la boucle reelle.
            let billet = p2.billet();
            let vus_avant = p2.vus();
            d2.wait(); // le producteur signale exactement ici
            let dormi = p2.attends(billet) == Fin::Signale;
            f2.wait(); // le producteur a fini son tour
            if dormi {
                sommeils += 1;
                if p2.vus() == vus_avant {
                    // Endormi sans que personne ne l'ait vu : reveil perdu.
                    oublies += 1;
                }
                p2.se_retire();
            }
            assert_eq!(p2.dormeurs_en_attente(), 0);
        }
        (sommeils, oublies)
    });

    producteur.join().expect("producteur");
    let (sommeils, oublies) = consommateur.join().expect("consommateur");
    assert_eq!(oublies, 0, "un dormeur s'est endormi sans etre vu : reveil perdu");
    assert!(sommeils <= TOURS);
}

/// Rien a faire et rien qui change tout seul => pas de tour de boucle. La
/// propriete « aucun spin permanent quand vide ».
#[test]
fn aucun_spin_permanent_quand_il_n_y_a_rien() {
    let porte = Porte::neuve();
    let mut tours = 0u64;
    let mut etat = Etat {
        maintenant_ms: 0,
        horloge_visible: false,
        dernier_releve_ms: u64::MAX - PERIODE_RELEVE_MS,
        ..Default::default()
    };

    // Simule dix tours : sans evenement et sans echeance proche, chacun doit
    // se terminer par un sommeil, jamais par un rebouclage immediat.
    for _ in 0..10 {
        let billet = porte.billet();
        assert!(!doit_composer(&etat), "rien a composer");
        match prochaine_echeance(&etat) {
            Some(date) if date <= etat.maintenant_ms => {
                panic!("echeance immediate alors que rien ne change");
            }
            _ => {
                assert_eq!(porte.attends(billet), Fin::Signale, "on dort");
                tours += 1;
            }
        }
        etat.maintenant_ms += 1;
    }
    assert_eq!(tours, 10, "chaque tour se termine par un sommeil");
}

/// Une trame client reveille, meme sans aucune entree utilisateur : le
/// navigateur ne doit pas attendre un clic pour afficher sa page.
#[test]
fn une_trame_client_reveille_sans_entree_utilisateur() {
    let porte = Porte::neuve();
    let billet = porte.billet();

    let etat = Etat {
        maintenant_ms: 7_000,
        sale: false,
        derniere_entree_ms: 0, // aucune entree, jamais
        horloge_visible: true,
        derniere_horloge_ms: 7_000,
        dernier_releve_ms: 7_000,
        ..Default::default()
    };
    assert!(!doit_composer(&etat), "rien a composer pour l'instant");

    // Le client ecrit sur son canal : le noyau signale.
    porte.signale();

    assert_eq!(
        porte.attends(billet),
        Fin::DejaSignale,
        "la trame client sort le bureau du sommeil sans aucune entree"
    );
}

/// Le curseur produit un degat LOCAL : deux empreintes, l'ancienne et la
/// nouvelle. La politique doit alors composer, pas attendre l'horloge.
#[test]
fn un_mouvement_de_curseur_demande_une_composition() {
    let etat = Etat {
        maintenant_ms: 8_000,
        sale: true, // les deux empreintes de curseur viennent d'etre ajoutees
        derniere_trame_ms: 8_000 - PERIODE_TRAME_MS,
        horloge_visible: true,
        derniere_horloge_ms: 8_000,
        dernier_releve_ms: 8_000,
        ..Default::default()
    };
    assert!(doit_composer(&etat));
    assert_eq!(
        prochaine_echeance(&etat),
        Some(8_000),
        "le creneau de trame est deja atteint : composer maintenant"
    );
}
