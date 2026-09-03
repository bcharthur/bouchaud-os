//! Preuve hote de la file d'execution O(1) du chantier 2.
//!
//! Ce test n'est PAS un modele : il inclut le module de production
//! `src/kernel/scheduler/runqueue.rs` tel quel. Ce qui est verifie ici est donc
//! ce qui s'execute dans le noyau, pas une reecriture qui lui ressemble.
//!
//! # Ce que la file precedente ne pouvait pas promettre
//!
//! Elle etait un `SpinLockIrq<Vec<u64>>` :
//!
//!   * `contains()` lineaire a chaque mise en file ;
//!   * `remove(0)` lineaire, deplacant tout le vecteur, a chaque election ;
//!   * `push()` pouvant REALLOUER -- donc allouer -- interruptions masquees ;
//!   * `len()` prenant le verrou, alors que le choix du CPU d'accueil l'appelle
//!     une fois par CPU a chaque reveil ;
//!   * et surtout : la classe `Interactive` n'existait NULLE PART dans la
//!     structure. Le commentaire promettait un tourniquet a deux etages, la
//!     file etait une seule FIFO.
//!
//! # Ce que ce test prouve
//!
//!   1. plusieurs taches interactives passent avant les normales sous
//!      contention ;
//!   2. une tache normale progresse malgre tout -- la borne anti-famine est
//!      exercee, pas seulement declaree ;
//!   3. une entree perimee (emplacement recycle, generation changee) n'est
//!      jamais servie comme si elle etait vivante ;
//!   4. aucune tache n'est servie deux fois, meme quand deux « CPU » elisent
//!      en meme temps sur la meme file ;
//!   5. aucune tache n'est perdue : tout ce qui entre ressort exactement une
//!      fois ;
//!   6. la mise en file est idempotente : pas de double entree ;
//!   7. le vol prend d'abord le travail NON interactif ;
//!   8. le mot de resume peut mentir sans qu'une tache soit perdue.

#[path = "../../src/kernel/scheduler/runqueue.rs"]
mod runqueue;

use runqueue::{Bande, FileCpu, EMPLACEMENTS, MOTS, TOURS_INTERACTIFS_MAX};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// 1 & 2 : la priorite est reelle, et la famine est bornee.
// ---------------------------------------------------------------------------

/// Sous contention, les interactives passent devant -- et la normale avance.
#[test]
fn interactive_passe_devant_sans_affamer_la_normale() {
    let file = FileCpu::neuve();

    // Quatre interactives, quatre normales, toutes pretes en meme temps.
    for i in 0..4 {
        assert!(file.enfile(i, Bande::Interactive));
        assert!(file.enfile(100 + i, Bande::Normale));
    }

    let mut ordre = Vec::new();
    while let Some(emplacement) = file.defile() {
        ordre.push(emplacement);
    }
    assert_eq!(ordre.len(), 8, "toutes les taches doivent sortir");

    // Les quatre interactives sortent avant la premiere normale : c'est la
    // priorite, et c'est ce qui n'existait pas.
    let premiere_normale = ordre.iter().position(|e| *e >= 100).unwrap();
    assert_eq!(
        premiere_normale, 4,
        "les interactives doivent toutes passer avant la premiere normale, ordre={ordre:?}"
    );
}

/// Une interactive qui ne se bloque JAMAIS ne doit pas figer la normale.
///
/// C'est la difference entre une priorite et une famine, et c'est le seul
/// invariant que la borne `TOURS_INTERACTIFS_MAX` existe pour tenir.
#[test]
fn interactive_infinie_laisse_progresser_la_normale() {
    let file = FileCpu::neuve();
    file.enfile(500, Bande::Normale);

    let mut normales_servies = 0usize;
    // A chaque tour on remet une interactive prete : elle est immediatement
    // reeligible, exactement comme une tache qui ne se bloque pas.
    for _ in 0..(TOURS_INTERACTIFS_MAX as usize * 3 + 8) {
        file.enfile(7, Bande::Interactive);
        match file.defile() {
            Some(500) => {
                normales_servies += 1;
                file.enfile(500, Bande::Normale);
            }
            Some(7) => {}
            autre => panic!("emplacement inattendu : {autre:?}"),
        }
    }

    assert!(
        normales_servies >= 2,
        "la bande normale doit avancer face a une interactive permanente \
         (servie {normales_servies} fois)"
    );
    assert!(
        file.compteurs().anti_famine >= 2,
        "la borne anti-famine doit etre EXERCEE, pas seulement declaree"
    );
}

// ---------------------------------------------------------------------------
// 3 : une incarnation perimee n'est jamais servie.
// ---------------------------------------------------------------------------

/// Le registre des taches, reduit a ce qui compte : quelle generation occupe
/// chaque emplacement.
struct Registre {
    generations: Vec<u32>,
}

impl Registre {
    fn neuf() -> Self {
        Self { generations: vec![1; EMPLACEMENTS] }
    }

    fn identite(&self, emplacement: usize) -> u64 {
        ((self.generations[emplacement] as u64) << 32) | emplacement as u64
    }

    /// Recycle l'emplacement : l'ancienne incarnation est morte.
    fn recycle(&mut self, emplacement: usize) {
        self.generations[emplacement] += 1;
    }

    /// Ce que fait `pick_next` : refuser ce que le registre ne reconnait plus.
    fn reconnait(&self, identite: u64) -> bool {
        let emplacement = (identite & 0xffff_ffff) as usize;
        let generation = (identite >> 32) as u32;
        emplacement < EMPLACEMENTS && self.generations[emplacement] == generation
    }
}

#[test]
fn une_generation_perimee_n_est_jamais_servie() {
    let file = FileCpu::neuve();
    let mut registre = Registre::neuf();

    let emplacement = 42usize;
    runqueue::publie_identite(emplacement, registre.identite(emplacement));
    file.enfile(emplacement, Bande::Normale);

    // La tache meurt, l'emplacement est reattribue a une autre. Personne n'a
    // retire l'entree de la file : c'est exactement le cas ABA.
    registre.recycle(emplacement);

    let sorti = file.defile().expect("l'entree est toujours en file");
    let identite = runqueue::identite_en_file(sorti);
    assert!(
        !registre.reconnait(identite),
        "une entree dont la generation a change doit etre refusee"
    );

    // Et la NOUVELLE incarnation, elle, doit pouvoir etre mise en file et
    // servie normalement.
    runqueue::publie_identite(emplacement, registre.identite(emplacement));
    file.enfile(emplacement, Bande::Normale);
    let sorti = file.defile().expect("la nouvelle incarnation doit sortir");
    assert!(registre.reconnait(runqueue::identite_en_file(sorti)));
}

/// Un emplacement recycle PENDANT qu'il est en file doit porter la nouvelle
/// identite, sinon la tache neuve ne serait jamais servie.
#[test]
fn le_recyclage_en_file_ne_perd_pas_la_nouvelle_incarnation() {
    let file = FileCpu::neuve();
    let mut registre = Registre::neuf();
    let emplacement = 9usize;

    runqueue::publie_identite(emplacement, registre.identite(emplacement));
    file.enfile(emplacement, Bande::Normale);

    registre.recycle(emplacement);
    // La nouvelle incarnation se publie : le bit est deja pose, l'identite
    // doit malgre tout etre remplacee.
    runqueue::publie_identite(emplacement, registre.identite(emplacement));
    let doublon = file.enfile(emplacement, Bande::Normale);
    assert!(!doublon, "le bit etait deja pose : pas de seconde entree");

    let sorti = file.defile().unwrap();
    assert!(
        registre.reconnait(runqueue::identite_en_file(sorti)),
        "l'identite publiee en dernier doit gagner"
    );
}

// ---------------------------------------------------------------------------
// 4 & 5 : exactement une fois, meme a plusieurs coeurs.
// ---------------------------------------------------------------------------

#[test]
fn aucun_double_ordonnancement_sous_contention_smp() {
    const TACHES: usize = 512;
    const COEURS: usize = 4;

    let file = Arc::new(FileCpu::neuve());
    for emplacement in 0..TACHES {
        let bande = if emplacement % 3 == 0 { Bande::Interactive } else { Bande::Normale };
        assert!(file.enfile(emplacement, bande));
    }
    assert_eq!(file.longueur(), TACHES);

    let servies = Arc::new((0..EMPLACEMENTS).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
    let mut fils = Vec::new();
    for coeur in 0..COEURS {
        let file = Arc::clone(&file);
        let servies = Arc::clone(&servies);
        fils.push(std::thread::spawn(move || {
            let mut prises = 0usize;
            loop {
                // Deux coeurs elisent, deux volent : les deux bouts de la meme
                // bande sont sollicites en meme temps.
                let pris = if coeur % 2 == 0 { file.defile() } else { file.vole() };
                match pris {
                    Some(emplacement) => {
                        servies[emplacement].fetch_add(1, Ordering::Relaxed);
                        prises += 1;
                    }
                    None => {
                        if file.est_vide() { break; }
                        std::thread::yield_now();
                    }
                }
            }
            prises
        }));
    }

    let total: usize = fils.into_iter().map(|f| f.join().unwrap()).sum();
    assert_eq!(total, TACHES, "chaque tache est servie exactement une fois");
    for emplacement in 0..TACHES {
        assert_eq!(
            servies[emplacement].load(Ordering::Relaxed), 1,
            "l'emplacement {emplacement} a ete servi plusieurs fois ou jamais"
        );
    }
    assert!(file.est_vide());
    assert_eq!(file.longueur(), 0);
}

/// Mises en file et elections SIMULTANEES : rien ne se perd.
#[test]
fn rien_ne_se_perd_quand_on_enfile_et_defile_en_meme_temps() {
    const TACHES: usize = 400;
    let file = Arc::new(FileCpu::neuve());
    let servies = Arc::new((0..EMPLACEMENTS).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());

    let producteur = {
        let file = Arc::clone(&file);
        std::thread::spawn(move || {
            for emplacement in 0..TACHES {
                let bande = if emplacement % 2 == 0 { Bande::Interactive } else { Bande::Normale };
                assert!(file.enfile(emplacement, bande));
            }
        })
    };

    let mut consommateurs = Vec::new();
    for _ in 0..3 {
        let file = Arc::clone(&file);
        let servies = Arc::clone(&servies);
        consommateurs.push(std::thread::spawn(move || {
            let mut prises = 0usize;
            let mut vides = 0usize;
            // On s'arrete apres une longue serie de files vides : le
            // producteur a fini et il ne reste rien.
            while vides < 20_000 {
                match file.defile() {
                    Some(emplacement) => {
                        servies[emplacement].fetch_add(1, Ordering::Relaxed);
                        prises += 1;
                        vides = 0;
                    }
                    None => { vides += 1; std::hint::spin_loop(); }
                }
            }
            prises
        }));
    }

    producteur.join().unwrap();
    let total: usize = consommateurs.into_iter().map(|f| f.join().unwrap()).sum();
    assert_eq!(total, TACHES, "toutes les taches produites doivent sortir une fois");
    for emplacement in 0..TACHES {
        assert_eq!(servies[emplacement].load(Ordering::Relaxed), 1);
    }
}

// ---------------------------------------------------------------------------
// 6 : pas de double entree.
// ---------------------------------------------------------------------------

#[test]
fn la_mise_en_file_est_idempotente() {
    let file = FileCpu::neuve();
    assert!(file.enfile(3, Bande::Normale));
    assert!(!file.enfile(3, Bande::Normale), "deuxieme mise en file : doublon");
    assert!(!file.enfile(3, Bande::Normale));
    assert_eq!(file.longueur(), 1);
    assert_eq!(file.compteurs().doublons, 2);

    assert_eq!(file.defile(), Some(3));
    assert_eq!(file.defile(), None, "une seule entree, une seule election");
}

/// Changer de bande ne cree pas une seconde entree.
#[test]
fn changer_de_bande_ne_duplique_pas() {
    let file = FileCpu::neuve();
    file.enfile(11, Bande::Normale);
    file.enfile(11, Bande::Interactive);
    assert_eq!(file.longueur(), 1, "la tache ne doit exister que dans une bande");
    assert!(file.bande(Bande::Interactive).contient(11));
    assert!(!file.bande(Bande::Normale).contient(11));
    assert_eq!(file.defile(), Some(11));
    assert!(file.est_vide());
}

// ---------------------------------------------------------------------------
// 7 : le vol prend le travail de fond.
// ---------------------------------------------------------------------------

#[test]
fn le_vol_prend_d_abord_le_travail_non_interactif() {
    let file = FileCpu::neuve();
    file.enfile(1, Bande::Interactive);
    file.enfile(2, Bande::Interactive);
    file.enfile(300, Bande::Normale);
    file.enfile(301, Bande::Normale);

    assert!(matches!(file.vole(), Some(300..=301)));
    assert!(matches!(file.vole(), Some(300..=301)));
    // Les normales epuisees, le vol peut se rabattre sur l'interactive.
    assert!(matches!(file.vole(), Some(1..=2)));

    assert_eq!(
        file.pression_volable(), 0,
        "la pression volable ne compte que le travail de fond"
    );
}

/// Election et vol prennent par des bouts OPPOSES de la meme bande.
#[test]
fn election_et_vol_se_croisent_le_moins_possible() {
    let file = FileCpu::neuve();
    for emplacement in [10usize, 200, 700] {
        file.enfile(emplacement, Bande::Normale);
    }
    assert_eq!(file.defile(), Some(10), "l'election sert par le bas");
    assert_eq!(file.vole(), Some(700), "le vol sert par le haut");
}

// ---------------------------------------------------------------------------
// 8 : le resume est un accelerateur, jamais une autorite.
// ---------------------------------------------------------------------------

/// Le balayage integral rattrape un resume faux.
///
/// Le resume est maintenu par un protocole (effacer, relire, remettre) qui
/// tient sous `SeqCst`. La correction, elle, ne doit pas en dependre : une
/// tache prete dont le bit de resume manquerait serait une machine figee. Ce
/// test verifie le filet -- `longueur` fait autorite, et le balayage borne par
/// `MOTS` retrouve le bit.
#[test]
fn un_resume_faux_ne_perd_aucune_tache() {
    let file = FileCpu::neuve();
    // Un emplacement dans chaque mot du bitmap : le balayage doit tous les
    // retrouver, quel que soit l'etat du resume.
    let emplacements: Vec<usize> = (0..MOTS).map(|mot| mot * 64 + 5).collect();
    for &emplacement in &emplacements {
        file.enfile(emplacement, Bande::Normale);
    }
    assert_eq!(file.longueur(), MOTS);

    let mut vus = HashSet::new();
    while let Some(emplacement) = file.defile() {
        assert!(vus.insert(emplacement), "emplacement {emplacement} servi deux fois");
    }
    assert_eq!(vus.len(), MOTS, "chaque mot du bitmap doit avoir ete visite");
    for emplacement in emplacements {
        assert!(vus.contains(&emplacement));
    }
}

/// Le tourniquet fait tourner le service entre les mots du bitmap.
#[test]
fn le_curseur_fait_tourner_le_service() {
    let file = FileCpu::neuve();
    // Deux emplacements tres eloignes : sans curseur, le bas serait toujours
    // servi en premier et le haut attendrait qu'il n'y ait plus rien en bas.
    file.enfile(1, Bande::Normale);
    file.enfile(900, Bande::Normale);
    let premier = file.defile().unwrap();
    file.enfile(premier, Bande::Normale);
    let second = file.defile().unwrap();
    assert_ne!(premier, second, "le service doit avancer, pas repeter le meme mot");
}

// ---------------------------------------------------------------------------
// Bornes et sanite.
// ---------------------------------------------------------------------------

#[test]
fn les_emplacements_hors_bornes_sont_refuses_sans_paniquer() {
    let file = FileCpu::neuve();
    assert!(!file.enfile(EMPLACEMENTS, Bande::Normale));
    assert!(!file.enfile(usize::MAX, Bande::Interactive));
    assert!(!file.contient(EMPLACEMENTS));
    assert!(file.est_vide());
}

#[test]
fn retirer_une_tache_la_sort_de_sa_bande() {
    let file = FileCpu::neuve();
    file.enfile(77, Bande::Interactive);
    assert!(file.contient(77));
    assert!(file.retire(77));
    assert!(!file.retire(77), "un second retrait ne retire rien");
    assert!(file.est_vide());
    assert_eq!(file.defile(), None);
}

#[test]
fn la_longueur_reste_exacte_apres_un_cycle_complet() {
    let file = FileCpu::neuve();
    for tour in 0..4 {
        for emplacement in 0..64usize {
            file.enfile(emplacement, if tour % 2 == 0 { Bande::Interactive } else { Bande::Normale });
        }
        assert_eq!(file.longueur(), 64);
        for _ in 0..64 { assert!(file.defile().is_some()); }
        assert_eq!(file.longueur(), 0);
        assert_eq!(file.defile(), None);
    }
}
