//! Harnais de test hote pour l'ORDRE de publication d'une tache sortante.
//!
//! # La propriete testee
//!
//! Un changement de contexte fait deux choses distinctes :
//!
//!   1. il PUBLIE la tache sortante -- `on_cpu = -1`, puis mise en file --,
//!      ce qui la rend eligible pour n'importe quel autre CPU ;
//!   2. il SAUVEGARDE son sommet de pile dans `ctx.rsp` (`switch_context`).
//!
//! Entre les deux, le gros verrou est deja rendu. Si (1) precede (2), un autre
//! CPU peut reprendre la tache avec le `ctx.rsp` de la commutation PRECEDENTE
//! -- un sommet perime qui ne designe plus un cadre valide.
//!
//! Ce n'est pas une hypothese sur le materiel : c'est un ordre d'ecritures dans
//! du code Rust, et il se modelise exactement. Le modele ci-dessous rejoue donc
//! l'entrelacement A LA MAIN, sans fil ni hasard : le test ne peut pas
//! clignoter, et il echoue si et seulement si l'ordre redevient faux.
//!
//! Lance par `tools/smp/test-commutation.sh`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

/// Le noyau utilise zero : `amorce_pile` ecrit toujours un sommet non nul, et
/// `switch_context` n'ecrit jamais zero. La valeur est donc impossible
/// autrement, ce qui en fait une sentinelle et pas un etat de plus.
const CONTEXTE_EN_VOL: u64 = 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Etat {
    Pret,
    Bloque,
}

#[derive(Clone, Debug)]
struct Tache {
    etat: Etat,
    on_cpu: i8,
    ctx_rsp: u64,
    runq_cpu: u8,
}

/// Le modele : la table des taches et une file par CPU.
struct Ordonnanceur {
    taches: Vec<Tache>,
    files: Vec<Vec<usize>>,
}

/// Les deux ordres de publication mis face a face.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ordre {
    /// Ce que faisait `switch_to` : publier, PUIS sauvegarder.
    PublierPuisSauver,
    /// BOUCHAUD_P0_CTX_EN_VOL_V1 : invalider, publier, puis sauvegarder.
    InvaliderPublierSauver,
}

impl Ordonnanceur {
    fn neuf(nb_cpu: usize, taches: Vec<Tache>) -> Self {
        Self { taches, files: vec![Vec::new(); nb_cpu] }
    }

    /// Etape 1 du changement de contexte, gros verrou tenu.
    fn publie_sortante(&mut self, ordre: Ordre, cpu: usize, index: usize) {
        if ordre == Ordre::InvaliderPublierSauver {
            self.taches[index].ctx_rsp = CONTEXTE_EN_VOL;
        }
        self.taches[index].on_cpu = -1;
        self.taches[index].runq_cpu = cpu as u8;
        if self.taches[index].etat == Etat::Pret {
            self.files[cpu].push(index);
        }
    }

    /// Etape 2 : le `mov [rdi], rsp` de `switch_context`. Verrou DEJA rendu.
    fn sauvegarde_contexte(&mut self, index: usize, rsp: u64) {
        self.taches[index].ctx_rsp = rsp;
    }

    fn contexte_publie(&self, index: usize) -> bool {
        self.taches[index].ctx_rsp != CONTEXTE_EN_VOL
    }

    fn eligible_au_vol(&self, index: usize, voleur: usize) -> bool {
        let t = &self.taches[index];
        t.etat == Etat::Pret
            && t.on_cpu < 0
            && self.contexte_publie(index)
            && t.runq_cpu as usize != voleur
    }

    /// Le vol tel que `pick_next` le fait : on sort de la file du donneur, on
    /// verifie, et on REMET si le candidat n'est pas reprenable.
    fn vole(&mut self, voleur: usize, donneur: usize) -> Option<(usize, u64)> {
        let index = self.files[donneur].pop()?;
        if !self.eligible_au_vol(index, voleur) {
            self.files[donneur].push(index);
            return None;
        }
        self.taches[index].on_cpu = voleur as i8;
        self.taches[index].runq_cpu = voleur as u8;
        Some((index, self.taches[index].ctx_rsp))
    }

    /// Le tour de file local de `pick_next`, avec sa borne.
    fn choisit_local(&mut self, cpu: usize) -> Option<usize> {
        let mut tours = self.files[cpu].len().saturating_add(1);
        while !self.files[cpu].is_empty() {
            let index = self.files[cpu].remove(0);
            if !self.contexte_publie(index) {
                self.files[cpu].push(index);
                tours = tours.saturating_sub(1);
                if tours == 0 {
                    return None;
                }
                continue;
            }
            if self.taches[index].etat == Etat::Pret && self.taches[index].on_cpu < 0 {
                return Some(index);
            }
        }
        None
    }
}

fn tache_prete(rsp: u64, cpu: u8) -> Tache {
    Tache { etat: Etat::Pret, on_cpu: cpu as i8, ctx_rsp: rsp, runq_cpu: cpu }
}

const SOMMET_PERIME: u64 = 0xFFFF_8000_0011_0000;
const SOMMET_REEL: u64 = 0xFFFF_8000_0022_0000;

/// L'ancien ordre laisse voler un sommet PERIME. C'est la panne, jouee a la
/// main : aucun fil, aucun hasard, l'entrelacement est ecrit noir sur blanc.
#[test]
fn ancien_ordre_publie_un_sommet_perime() {
    let mut ord = Ordonnanceur::neuf(2, vec![tache_prete(SOMMET_PERIME, 0)]);

    // CPU 0 commute : il publie...
    ord.publie_sortante(Ordre::PublierPuisSauver, 0, 0);

    // ... et CPU 1 vole AVANT que `switch_context` n'ait ecrit le vrai sommet.
    let vol = ord.vole(1, 0);

    // CPU 0 finit son switch, trop tard.
    ord.sauvegarde_contexte(0, SOMMET_REEL);

    let (index, sommet) = vol.expect("l'ancien ordre laisse effectivement voler");
    assert_eq!(index, 0);
    assert_eq!(
        sommet, SOMMET_PERIME,
        "CPU 1 est reparti sur le sommet de la commutation PRECEDENTE"
    );
    assert_ne!(
        sommet, SOMMET_REEL,
        "le sommet vole n'est pas celui que switch_context allait ecrire"
    );
}

/// Le nouvel ordre refuse ce vol.
#[test]
fn nouvel_ordre_refuse_la_tache_en_vol() {
    let mut ord = Ordonnanceur::neuf(2, vec![tache_prete(SOMMET_PERIME, 0)]);

    ord.publie_sortante(Ordre::InvaliderPublierSauver, 0, 0);
    assert!(!ord.contexte_publie(0), "la sentinelle doit etre posee");

    assert!(
        ord.vole(1, 0).is_none(),
        "une tache dont le contexte est en vol ne doit pas etre reprise"
    );
}

/// Refuser n'est pas perdre : la tache reste en file et repart au tour suivant,
/// avec le BON sommet. Un correctif qui la ferait disparaitre serait pire que
/// le bug qu'il corrige.
#[test]
fn le_refus_ne_perd_pas_la_tache() {
    let mut ord = Ordonnanceur::neuf(2, vec![tache_prete(SOMMET_PERIME, 0)]);

    ord.publie_sortante(Ordre::InvaliderPublierSauver, 0, 0);
    assert!(ord.vole(1, 0).is_none());
    assert_eq!(ord.files[0], vec![0], "la tache doit etre remise dans la file");

    ord.sauvegarde_contexte(0, SOMMET_REEL);

    let (index, sommet) = ord.vole(1, 0).expect("reprenable une fois le contexte publie");
    assert_eq!(index, 0);
    assert_eq!(sommet, SOMMET_REEL, "et c'est le sommet REEL qui est repris");
}

/// Le tour de file local se termine meme si TOUTE la file est en vol, et il ne
/// vide pas la file. Sans borne, ce CPU tournerait indefiniment sous verrou.
#[test]
fn le_tour_de_file_local_est_borne() {
    let mut ord = Ordonnanceur::neuf(
        2,
        vec![tache_prete(SOMMET_PERIME, 0), tache_prete(SOMMET_PERIME, 0)],
    );
    ord.publie_sortante(Ordre::InvaliderPublierSauver, 0, 0);
    ord.publie_sortante(Ordre::InvaliderPublierSauver, 0, 1);

    assert!(ord.choisit_local(0).is_none(), "rien de reprenable pour l'instant");
    assert_eq!(ord.files[0].len(), 2, "les deux taches restent executables");

    ord.sauvegarde_contexte(0, SOMMET_REEL);
    ord.sauvegarde_contexte(1, SOMMET_REEL + 0x1000);
    assert!(ord.choisit_local(0).is_some(), "elles repartent au tour suivant");
}

/// Une tache qui se BLOQUE n'est pas mise en file par la commutation : c'est un
/// reveil concurrent qui l'y met. La fenetre est la meme, et la sentinelle doit
/// donc la couvrir aussi -- c'est le chemin `publish_ready`.
#[test]
fn un_reveil_concurrent_ne_contourne_pas_la_sentinelle() {
    let mut tache = tache_prete(SOMMET_PERIME, 0);
    tache.etat = Etat::Bloque;
    let mut ord = Ordonnanceur::neuf(2, vec![tache]);

    ord.publie_sortante(Ordre::InvaliderPublierSauver, 0, 0);
    assert!(ord.files[0].is_empty(), "une tache bloquee n'est pas mise en file");

    // Un autre CPU la reveille pendant que le switch est encore en vol.
    ord.taches[0].etat = Etat::Pret;
    ord.files[0].push(0);

    assert!(ord.vole(1, 0).is_none(), "la sentinelle tient aussi sur ce chemin");

    ord.sauvegarde_contexte(0, SOMMET_REEL);
    assert_eq!(ord.vole(1, 0).map(|(_, rsp)| rsp), Some(SOMMET_REEL));
}

/// Contre-epreuve sous vrais fils : sous le NOUVEL ordre, aucun voleur ne doit
/// JAMAIS observer un sommet perime.
///
/// Cette assertion est a sens unique -- elle ne peut echouer que si le bug est
/// present -- donc elle ne clignote pas. C'est volontaire : la demonstration de
/// la panne est faite plus haut, a la main ; ici on ne cherche qu'a soumettre
/// l'ordre memoire a du vrai parallelisme.
#[test]
fn sous_fils_concurrents_aucun_sommet_perime() {
    const TOURS: u64 = 20_000;

    // `ctx_rsp` partage, comme dans le noyau : ecrit par le commutateur,
    // lu par le voleur.
    let ctx = Arc::new(AtomicU64::new(SOMMET_PERIME));
    // Publication de la tache dans la file, modelisee par un drapeau.
    let en_file = Arc::new(AtomicU64::new(0));
    let barriere = Arc::new(Barrier::new(2));

    let (c1, f1, b1) = (ctx.clone(), en_file.clone(), barriere.clone());
    let commutateur = std::thread::spawn(move || {
        for tour in 1..=TOURS {
            b1.wait();
            // Ordre du noyau : sentinelle, PUIS publication, PUIS sauvegarde.
            c1.store(CONTEXTE_EN_VOL, Ordering::Relaxed);
            f1.store(tour, Ordering::Release);
            std::hint::spin_loop();
            c1.store(SOMMET_REEL + tour, Ordering::Release);
        }
    });

    let (c2, f2, b2) = (ctx.clone(), en_file.clone(), barriere.clone());
    let voleur = std::thread::spawn(move || {
        let mut vols = 0u64;
        for tour in 1..=TOURS {
            b2.wait();
            // On ne "vole" que ce qui est publie ET dont le contexte l'est.
            while f2.load(Ordering::Acquire) != tour {
                std::hint::spin_loop();
            }
            let vu = c2.load(Ordering::Acquire);
            if vu != CONTEXTE_EN_VOL {
                assert_eq!(
                    vu,
                    SOMMET_REEL + tour,
                    "sommet perime observe au tour {tour}"
                );
                vols += 1;
            }
        }
        vols
    });

    commutateur.join().expect("fil commutateur");
    let vols = voleur.join().expect("fil voleur");
    // On n'exige pas un nombre de vols : ce qui compte est qu'AUCUN d'eux
    // n'ait rapporte un sommet perime.
    assert!(vols <= TOURS);
}
