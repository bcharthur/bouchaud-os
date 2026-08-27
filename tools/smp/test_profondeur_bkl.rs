//! Preuve hote du CONTRAT DE PROFONDEUR du gros verrou.
//!
//! # La propriete
//!
//! Toute primitive bloquante rend la main en laissant le gros verrou a la
//! profondeur exacte ou elle l'a trouve. `suspend_for_schedule` la met a zero,
//! `resume_after_schedule` la restaure.
//!
//! # Pourquoi ce contrat merite un test a lui seul
//!
//! Parce que le violer ne produit AUCUNE erreur au moment de la faute. La
//! profondeur perdue ne se manifeste que plus tard, au `Drop` d'un garde
//! quelconque, sous la forme :
//!
//!     smp_lock: release sans acquisition
//!
//! La tache qui panique n'est alors pas celle qui a fauté -- souvent meme pas
//! la meme fonction. C'est exactement ce qui rendait le panic `bkl.rs` du
//! bureau si difficile a attribuer : `depth=[0,0,0,0]`, `owner=0`, et aucune
//! indication de QUI avait consomme la profondeur.
//!
//! Les tests ci-dessous modelisent les cinq formes de rupture, et verifient que
//! la post-condition les nomme a la SOURCE au lieu de les laisser exploser chez
//! la victime.
//!
//! Lance par `tools/smp/test-profondeur-bkl.sh`.

const LIBRE: usize = 0;

/// Le verrou, reduit a ce qui porte le contrat.
#[derive(Debug)]
struct Verrou {
    owner: usize,
    depth: Vec<usize>,
}

/// Ce que `release_one` constate. Rendu au lieu de paniquer, pour que le test
/// puisse observer la faute au lieu de mourir avec elle.
#[derive(Debug, PartialEq, Eq)]
enum Relache {
    Ok,
    /// `debug_assert!(depth > 0)` : profondeur deja nulle.
    SansAcquisition,
    /// `debug_assert_eq!(owner, token(cpu))`.
    MauvaisProprietaire,
}

impl Verrou {
    fn neuf(cpus: usize) -> Self {
        Self { owner: LIBRE, depth: vec![0; cpus] }
    }

    fn token(cpu: usize) -> usize {
        cpu + 1
    }

    fn entre(&mut self, cpu: usize) {
        if self.owner == Self::token(cpu) {
            self.depth[cpu] += 1;
            return;
        }
        assert_eq!(self.owner, LIBRE, "modele : entree sur un verrou tenu ailleurs");
        self.owner = Self::token(cpu);
        self.depth[cpu] = 1;
    }

    fn relache(&mut self, cpu: usize) -> Relache {
        let depth = self.depth[cpu];
        if depth == 0 {
            return Relache::SansAcquisition;
        }
        if self.owner != Self::token(cpu) {
            return Relache::MauvaisProprietaire;
        }
        if depth > 1 {
            self.depth[cpu] = depth - 1;
        } else {
            self.depth[cpu] = 0;
            self.owner = LIBRE;
        }
        Relache::Ok
    }

    fn suspend(&mut self, cpu: usize) -> usize {
        let depth = self.depth[cpu];
        if depth == 0 {
            return 0;
        }
        self.depth[cpu] = 0;
        self.owner = LIBRE;
        depth
    }

    fn reprend(&mut self, cpu: usize, depth: usize) {
        if depth == 0 {
            return;
        }
        assert_eq!(self.owner, LIBRE, "modele : reprise sur un verrou tenu");
        self.owner = Self::token(cpu);
        self.depth[cpu] = depth;
    }

    fn profondeur(&self, cpu: usize) -> usize {
        self.depth[cpu]
    }
}

/// La post-condition du noyau : `verifie_profondeur_rendue`.
///
/// Rend le nom du site fautif, ou `None` si le contrat est tenu.
fn verifie(site: &'static str, attendue: usize, rendue: usize) -> Option<&'static str> {
    if rendue == attendue { None } else { Some(site) }
}

// ---------------------------------------------------------------- cas normaux

/// Une primitive bloquante correcte : suspension complete, reprise complete.
#[test]
fn une_suspension_equilibree_rend_la_profondeur_entiere() {
    let mut v = Verrou::neuf(4);
    v.entre(0); // garde externe du fil noyau
    v.entre(0); // garde d'appel systeme
    let attendue = v.profondeur(0);
    assert_eq!(attendue, 2);

    let d = v.suspend(0);
    assert_eq!(v.profondeur(0), 0, "la suspension libere TOUTE la profondeur");
    assert_eq!(v.owner, LIBRE, "et rend le verrou aux autres CPU");
    v.reprend(0, d);

    assert_eq!(verifie("primitive", attendue, v.profondeur(0)), None);
    assert_eq!(v.relache(0), Relache::Ok);
    assert_eq!(v.relache(0), Relache::Ok);
    assert_eq!(v.profondeur(0), 0);
}

/// Le cas du bureau : un fil noyau garde le verrou en permanence, et dort
/// dedans. Pendant tout son sommeil la profondeur est nulle et le verrou libre
/// -- c'est voulu, les autres CPU doivent pouvoir entrer en noyau.
#[test]
fn un_fil_noyau_dort_verrou_rendu_et_le_retrouve() {
    let mut v = Verrou::neuf(4);
    v.entre(0); // `kernel_task_trampoline` : jamais relache
    let attendue = v.profondeur(0);

    let externe = v.suspend(0);
    assert_eq!(v.profondeur(0), 0);
    assert_eq!(v.owner, LIBRE);

    // Un autre CPU travaille pendant ce temps.
    v.entre(2);
    assert_eq!(v.relache(2), Relache::Ok);

    // Le tour de boucle de `sleep_ticks` : entrer, lire l'etat, ressortir.
    v.entre(0);
    assert_eq!(v.relache(0), Relache::Ok);

    v.reprend(0, externe);
    assert_eq!(verifie("sleep_ticks", attendue, v.profondeur(0)), None);
    assert_eq!(v.profondeur(0), 1);
}

/// Migration : la continuation reprend sur un AUTRE CPU. La profondeur suit la
/// pile, pas le coeur.
#[test]
fn une_migration_preserve_la_profondeur() {
    let mut v = Verrou::neuf(4);
    v.entre(1);
    v.entre(1);
    let attendue = v.profondeur(1);

    let d = v.suspend(1);
    v.reprend(3, d); // reprise sur CPU 3

    assert_eq!(v.profondeur(3), attendue);
    assert_eq!(v.profondeur(1), 0);
    // Le Drop libere le CPU COURANT, pas celui de creation.
    assert_eq!(v.relache(3), Relache::Ok);
    assert_eq!(v.relache(3), Relache::Ok);
}

// ------------------------------------------------- les cinq formes de rupture

/// A) SUSPEND sans RESUME. La faute est invisible sur le coup.
#[test]
fn cas_a_suspension_sans_reprise_est_nommee_a_la_source() {
    let mut v = Verrou::neuf(4);
    v.entre(0);
    let attendue = v.profondeur(0);

    let _abandonnee = v.suspend(0); // la reprise n'a jamais lieu

    // La post-condition designe le SITE fautif, immediatement.
    assert_eq!(
        verifie("primitive_fautive", attendue, v.profondeur(0)),
        Some("primitive_fautive"),
    );

    // Sans elle, la faute n'apparait qu'ici -- chez la victime, qui n'a rien
    // fait de mal, et sans aucun moyen de remonter au coupable.
    assert_eq!(v.relache(0), Relache::SansAcquisition);
}

/// B) Reprise sur une continuation qui n'est pas la sienne : la profondeur
/// restauree est celle d'un AUTRE cadre.
#[test]
fn cas_b_reprise_sur_mauvaise_continuation() {
    let mut v = Verrou::neuf(4);
    v.entre(0);
    v.entre(0);
    v.entre(0);
    let attendue = v.profondeur(0);
    assert_eq!(attendue, 3);

    let _mien = v.suspend(0);
    v.reprend(0, 1); // profondeur d'une autre pile

    assert_eq!(verifie("switch", attendue, v.profondeur(0)), Some("switch"));

    // Deux relaches suffisent a vider le verrou, la troisieme fait le panic.
    assert_eq!(v.relache(0), Relache::Ok);
    assert_eq!(v.relache(0), Relache::SansAcquisition);
}

/// C) Reprise avec une profondeur trop GRANDE : les relaches en trop
/// s'attribuent des niveaux qui n'ont jamais ete pris.
#[test]
fn cas_c_reprise_avec_profondeur_excedentaire() {
    let mut v = Verrou::neuf(4);
    v.entre(0);
    let attendue = v.profondeur(0);

    let _d = v.suspend(0);
    v.reprend(0, 3);

    assert_eq!(verifie("resume", attendue, v.profondeur(0)), Some("resume"));

    // Le verrou reste tenu apres la seule relache legitime : c'est une fuite,
    // donc un blocage des autres CPU, et pas un panic. Pire qu'un panic.
    assert_eq!(v.relache(0), Relache::Ok);
    assert_eq!(v.profondeur(0), 2);
    assert_ne!(v.owner, LIBRE, "le verrou reste tenu par personne de reel");
}

/// D) Migration dont la reprise oublie de suivre la pile : l'etat BKL reste
/// attribue a l'ancien CPU.
#[test]
fn cas_d_migration_dont_la_reprise_reste_sur_l_ancien_cpu() {
    let mut v = Verrou::neuf(4);
    v.entre(1);
    let attendue = v.profondeur(1);

    let d = v.suspend(1);
    v.reprend(1, d); // la continuation, elle, s'execute sur le CPU 3

    // Vu du CPU 3 -- celui qui execute reellement -- la profondeur est nulle.
    assert_eq!(
        verifie("migration", attendue, v.profondeur(3)),
        Some("migration"),
    );
    assert_eq!(v.relache(3), Relache::SansAcquisition);
}

/// E) Un garde externe survit a une transition qui n'est plus symetrique :
/// la primitive interne suspend la profondeur TOTALE mais n'en restaure que
/// la sienne.
#[test]
fn cas_e_garde_externe_survit_a_une_transition_asymetrique() {
    let mut v = Verrou::neuf(4);
    v.entre(0); // garde externe (appel systeme, ou fil noyau)
    v.entre(0); // garde interne de la primitive
    let attendue = v.profondeur(0);

    let total = v.suspend(0);
    assert_eq!(total, 2, "la suspension emporte AUSSI le niveau externe");
    v.reprend(0, 1); // la primitive ne restaure que le sien

    assert_eq!(verifie("primitive", attendue, v.profondeur(0)), Some("primitive"));

    // Le garde interne se relache normalement...
    assert_eq!(v.relache(0), Relache::Ok);
    // ... et c'est le garde EXTERNE, innocent, qui panique.
    assert_eq!(v.relache(0), Relache::SansAcquisition);
}

// ------------------------------------------------------------------ propriete

/// La propriete qui justifie tout le dispositif : sans post-condition, le site
/// qui panique n'est jamais le site qui a fauté.
#[test]
fn sans_post_condition_la_victime_n_est_pas_le_coupable() {
    let mut v = Verrou::neuf(4);
    v.entre(0); // garde A -- innocent
    let _perdue = v.suspend(0); // faute commise ICI

    // Beaucoup de code s'execute entre les deux, sur ce CPU comme ailleurs.
    v.entre(2);
    assert_eq!(v.relache(2), Relache::Ok);

    // Et c'est le Drop de A qui explose, tres loin de la faute.
    assert_eq!(v.relache(0), Relache::SansAcquisition);
}

/// La post-condition ne doit pas crier sur un cas legitime : une primitive qui
/// n'a jamais eu le verrou n'a rien a rendre.
#[test]
fn une_primitive_sans_verrou_ne_declenche_rien() {
    let mut v = Verrou::neuf(4);
    let attendue = v.profondeur(0);
    assert_eq!(attendue, 0);

    let d = v.suspend(0);
    assert_eq!(d, 0, "suspendre un verrou non tenu ne rend rien");
    v.reprend(0, d);

    assert_eq!(verifie("primitive", attendue, v.profondeur(0)), None);
}
