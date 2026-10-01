//! Le cycle de vie d'une tache : un zombie ne ressuscite jamais, une
//! condamnation n'est jamais perdue.
//!
//! BOUCHAUD_CYCLE_DE_VIE_V1
//!
//! # Ce que ce test explore
//!
//! TOUS les entrelacements (ordre sequentiel, comme `SeqCst` dans le noyau)
//! de trois acteurs :
//!
//!   * la TACHE victime, qui entre dans une attente puis retourne en espace
//!     utilisateur -- quatre scenarios : `nanosleep` (interruptible, echeance),
//!     `futex` sans reveil (interruptible, personne ne viendra), verrou
//!     dormant (non interruptible, l'evenement viendra), calcul en espace
//!     utilisateur (aucune attente) ;
//!   * le TUEUR (un frere en `exit_group`) ;
//!   * le REVEILLEUR de l'attente, quand le scenario en a un.
//!
//! sous deux protocoles :
//!
//!   * l'ANCIEN, ecrit ici : le tueur ecrit `Zombie` par un simple store
//!     (`marque_zombie`), la tache ecrit `Bloque` par un simple store
//!     (`range`), la frontiere ne retire que l'etat `Zombie` ;
//!   * le NOUVEAU : `cycle_vie::action_tueur`, `meurt_au_parking`,
//!     `applique` -- le code meme que le noyau execute.
//!
//! Proprietes verifiees a chaque pas et a chaque fin :
//!
//!   P1 Zombie est absorbant : aucune ecriture ne le quitte ;
//!   P2 une tache tuee ne s'execute plus en espace utilisateur apres sa
//!      frontiere suivante (pas de resurrection) ;
//!   P3 une tache tuee n'est jamais parquee pour toujours (pas de mort perdue) ;
//!   P4 jamais deux entrees de file pour la meme tache.
//!
//! L'ancien protocole doit violer P1 a P3 (temoin negatif) ; le nouveau,
//! aucune. Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/scheduler/cycle_vie.rs"]
mod cycle_vie;

use cycle_vie::{action_tueur, applique, meurt_au_parking, ActionTueur, Etat, Transition, VueTueur};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Protocole {
    Ancien,
    Nouveau,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Scenario {
    /// nanosleep : interruptible, l'echeance reveillera.
    Sommeil,
    /// futex sans reveilleur : interruptible, rien ne viendra.
    FutexOublie,
    /// verrou dormant : non interruptible, le detenteur le rendra.
    Verrou,
    /// calcul en espace utilisateur, aucune attente.
    Calcul,
}

impl Scenario {
    fn interruptible(self) -> bool {
        matches!(self, Scenario::Sommeil | Scenario::FutexOublie)
    }
    fn a_un_reveilleur(self) -> bool {
        matches!(self, Scenario::Sommeil | Scenario::Verrou)
    }
}

// Compteurs de programme.
const T_DEBUT: u8 = 0;
const T_PUBLIE: u8 = 1;
const T_RELIT: u8 = 2;
const T_QUITTE: u8 = 3;
const T_PARQUEE: u8 = 4;
const T_REPREND: u8 = 5;
const T_FRONTIERE: u8 = 6;
const T_UTILISATEUR: u8 = 7;
const T_MORTE: u8 = 8;

const K_CONDAMNE: u8 = 0;
const K_DECIDE: u8 = 1;
const K_FINI: u8 = 2;

const W_ARME: u8 = 0;
const W_FINI: u8 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Monde {
    etat: Etat,
    condamnee: bool,
    sur_coeur: bool,
    interruptible: bool,
    enfile: u8,
    evenement: bool,
    t: u8,
    k: u8,
    w: u8,
    /// Passages en espace utilisateur APRES la fin du tueur.
    survies: u8,
}

#[derive(Default, Debug)]
struct Bilan {
    etats: usize,
    p1_zombie_quitte: usize,
    p2_resurrection: usize,
    p3_mort_perdue: usize,
    p4_double_file: usize,
}

impl Bilan {
    fn propre(&self) -> bool {
        self.p1_zombie_quitte == 0
            && self.p2_resurrection == 0
            && self.p3_mort_perdue == 0
            && self.p4_double_file == 0
    }
}

/// Une ecriture d'etat. Rend `false` si elle quitte `Zombie` (P1).
fn ecrit(m: &mut Monde, vers: Etat) -> bool {
    let ok = !(m.etat == Etat::Zombie && vers != Etat::Zombie);
    m.etat = vers;
    ok
}

/// Publication par un reveilleur : une entree si la tache n'occupe pas de
/// coeur (sinon la passation la republiera).
fn publie(m: &mut Monde) {
    if !m.sur_coeur && m.etat == Etat::Pret {
        m.enfile += 1;
    }
}

/// Les successeurs d'un monde, avec les violations de P1 rencontrees.
fn successeurs(m: Monde, p: Protocole, s: Scenario, p1: &mut usize) -> Vec<Monde> {
    let mut out = Vec::new();

    // --- La tache ---------------------------------------------------------
    let mut t = m;
    let mut bouge = true;
    match m.t {
        T_DEBUT => {
            if s == Scenario::Calcul {
                t.t = T_FRONTIERE;
            } else {
                t.interruptible = s.interruptible();
                t.t = T_PUBLIE;
            }
        }
        T_PUBLIE => match p {
            Protocole::Ancien => {
                // `range(Bloque)` : ecrase ce qui s'y trouve.
                if !ecrit(&mut t, Etat::Bloque) {
                    *p1 += 1;
                }
                t.t = T_QUITTE;
            }
            Protocole::Nouveau => match applique(m.etat, Transition::Endort) {
                Some(e) => {
                    t.etat = e;
                    t.t = T_RELIT;
                }
                // `endort` refuse : la tache est deja morte (impossible dans
                // ce modele : seul un tueur l'ecrirait, et seulement parquee).
                None => {
                    t.t = T_MORTE;
                }
            },
        },
        T_RELIT => {
            if meurt_au_parking(m.condamnee, m.interruptible) {
                if !ecrit(&mut t, Etat::Zombie) {
                    *p1 += 1;
                }
                t.sur_coeur = false;
                t.t = T_MORTE;
            } else if m.evenement && m.etat == Etat::Bloque {
                // La relecture de la generation : le reveil est deja passe,
                // la tache annule son parking.
                t.etat = applique(Etat::Bloque, Transition::Reveille).unwrap();
                t.t = T_REPREND;
            } else {
                t.t = T_QUITTE;
            }
        }
        T_QUITTE => {
            // `while etat == Bloque { schedule() }`
            if m.etat == Etat::Bloque {
                t.sur_coeur = false;
                t.t = T_PARQUEE;
            } else if m.etat == Etat::Zombie {
                // Tuee pendant qu'elle quittait son coeur : la passation ne
                // republie pas un zombie, la tache ne reviendra jamais.
                t.sur_coeur = false;
                t.t = T_MORTE;
            } else {
                t.t = T_REPREND;
            }
        }
        T_PARQUEE => {
            if m.etat == Etat::Pret && m.enfile > 0 {
                t.enfile -= 1;
                t.sur_coeur = true;
                t.t = T_REPREND;
            } else if m.etat == Etat::Zombie {
                t.t = T_MORTE;
            } else {
                bouge = false;
            }
        }
        T_REPREND => {
            // L'appelant de l'attente reboucle tant que sa condition n'est
            // pas remplie -- et une attente interruptible relit d'abord la
            // condamnation, comme elle relit un signal.
            let interrompue = p == Protocole::Nouveau && m.interruptible && m.condamnee;
            if m.evenement || interrompue {
                t.interruptible = false;
                t.t = T_FRONTIERE;
            } else {
                t.t = T_PUBLIE;
            }
        }
        T_FRONTIERE => {
            let retiree = match p {
                // `retire_current_if_zombie` : l'etat seul.
                Protocole::Ancien => m.etat == Etat::Zombie,
                Protocole::Nouveau => m.condamnee,
            };
            if retiree {
                if !ecrit(&mut t, Etat::Zombie) {
                    *p1 += 1;
                }
                t.sur_coeur = false;
                t.t = T_MORTE;
            } else {
                t.t = T_UTILISATEUR;
                if m.k == K_FINI {
                    t.survies = t.survies.saturating_add(1);
                }
            }
        }
        T_UTILISATEUR => {
            // Le prochain appel systeme, ou la prochaine preemption depuis
            // l'espace utilisateur : une frontiere. On s'arrete a deux
            // survies, qui suffisent a prouver la resurrection.
            if m.survies >= 2 {
                bouge = false;
            } else {
                t.t = T_FRONTIERE;
            }
        }
        _ => bouge = false,
    }
    if bouge {
        out.push(t);
    }

    // --- Le tueur ---------------------------------------------------------
    let mut k = m;
    match (p, m.k) {
        (Protocole::Ancien, K_CONDAMNE) => {
            // `marque_zombie` : store simple.
            k.etat = Etat::Zombie;
            k.k = K_FINI;
            out.push(k);
        }
        (Protocole::Nouveau, K_CONDAMNE) => {
            k.condamnee = true;
            k.k = K_DECIDE;
            out.push(k);
        }
        (Protocole::Nouveau, K_DECIDE) => {
            let vue = VueTueur { etat: m.etat, sur_coeur: m.sur_coeur, interruptible: m.interruptible };
            match action_tueur(vue) {
                ActionTueur::Rien | ActionTueur::Frontiere | ActionTueur::AttendSonEvenement => {
                    k.k = K_FINI;
                }
                ActionTueur::TueSurPlace => {
                    // CAS Bloque -> Zombie ; l'etat lu est l'etat courant
                    // (pas atomique separe), le CAS reussit donc ici.
                    k.etat = applique(m.etat, Transition::TueParquee).expect("tue_parquee depuis Bloque");
                    k.k = K_FINI;
                }
                ActionTueur::Reveille => {
                    k.etat = applique(m.etat, Transition::Reveille).expect("reveille depuis Bloque");
                    publie(&mut k);
                    k.k = K_FINI;
                }
            }
            out.push(k);
        }
        _ => {}
    }

    // --- Le reveilleur ----------------------------------------------------
    if s.a_un_reveilleur() && m.w == W_ARME {
        let mut w = m;
        w.evenement = true;
        // CAS Bloque -> Pret : seulement s'il gagne.
        if m.etat == Etat::Bloque {
            w.etat = Etat::Pret;
            publie(&mut w);
        }
        w.w = W_FINI;
        out.push(w);
    }
    out
}

fn explore(p: Protocole, s: Scenario) -> Bilan {
    let depart = Monde {
        etat: Etat::Pret,
        condamnee: false,
        sur_coeur: true,
        interruptible: false,
        enfile: 0,
        evenement: false,
        t: T_DEBUT,
        k: K_CONDAMNE,
        w: if s.a_un_reveilleur() { W_ARME } else { W_FINI },
        survies: 0,
    };
    let mut vus = HashSet::new();
    let mut pile = vec![depart];
    let mut bilan = Bilan::default();
    while let Some(m) = pile.pop() {
        if !vus.insert(m) {
            continue;
        }
        bilan.etats += 1;
        if m.enfile > 1 {
            bilan.p4_double_file += 1;
        }
        if m.survies >= 2 {
            bilan.p2_resurrection += 1;
        }
        let mut p1 = 0;
        let suivants = successeurs(m, p, s, &mut p1);
        bilan.p1_zombie_quitte += p1;
        if suivants.is_empty() {
            // Fin : plus personne ne bouge.
            if m.t == T_PARQUEE && m.etat != Etat::Zombie {
                bilan.p3_mort_perdue += 1;
            }
            if m.t == T_UTILISATEUR && m.k == K_FINI && m.survies >= 2 {
                // deja compte en P2
            }
        }
        pile.extend(suivants);
    }
    bilan
}

const SCENARIOS: [Scenario; 4] =
    [Scenario::Sommeil, Scenario::FutexOublie, Scenario::Verrou, Scenario::Calcul];

#[test]
fn table_des_transitions() {
    // Zombie est absorbant.
    for t in [Transition::Endort, Transition::Reveille, Transition::Meurt, Transition::TueParquee] {
        let vers = applique(Etat::Zombie, t);
        assert!(vers.is_none() || vers == Some(Etat::Zombie), "{t:?} quitte Zombie");
    }
    assert_eq!(applique(Etat::Pret, Transition::Endort), Some(Etat::Bloque));
    assert_eq!(applique(Etat::Bloque, Transition::Reveille), Some(Etat::Pret));
    assert_eq!(applique(Etat::Pret, Transition::TueParquee), None, "un tueur ne tue pas une tache prete");
    assert_eq!(applique(Etat::Bloque, Transition::Endort), None);
}

#[test]
fn decisions_du_tueur() {
    let v = |etat, sur_coeur, interruptible| VueTueur { etat, sur_coeur, interruptible };
    assert_eq!(action_tueur(v(Etat::Zombie, false, true)), ActionTueur::Rien);
    assert_eq!(action_tueur(v(Etat::Pret, true, false)), ActionTueur::Frontiere);
    assert_eq!(action_tueur(v(Etat::Pret, false, false)), ActionTueur::Frontiere);
    assert_eq!(action_tueur(v(Etat::Bloque, true, true)), ActionTueur::Reveille);
    assert_eq!(action_tueur(v(Etat::Bloque, true, false)), ActionTueur::Reveille);
    assert_eq!(action_tueur(v(Etat::Bloque, false, true)), ActionTueur::TueSurPlace);
    assert_eq!(action_tueur(v(Etat::Bloque, false, false)), ActionTueur::AttendSonEvenement);
}

#[test]
fn l_ancien_protocole_ressuscite_et_perd() {
    let sommeil = explore(Protocole::Ancien, Scenario::Sommeil);
    assert!(sommeil.p1_zombie_quitte > 0, "ancien, nanosleep : Bloque ecrit par-dessus Zombie {sommeil:?}");
    assert!(sommeil.p2_resurrection > 0, "ancien, nanosleep : retour en espace utilisateur apres la mort {sommeil:?}");
    let futex = explore(Protocole::Ancien, Scenario::FutexOublie);
    assert!(futex.p3_mort_perdue > 0, "ancien, futex : tache tuee parquee pour toujours {futex:?}");
    let verrou = explore(Protocole::Ancien, Scenario::Verrou);
    assert!(verrou.p2_resurrection > 0, "ancien, verrou : resurrection {verrou:?}");
}

#[test]
fn le_nouveau_protocole_ne_viole_rien() {
    for s in SCENARIOS {
        let b = explore(Protocole::Nouveau, s);
        assert!(b.etats > 10, "{s:?} : exploration trop courte {b:?}");
        assert!(b.propre(), "nouveau protocole, {s:?} : {b:?}");
    }
}

#[test]
fn la_tache_tuee_meurt_dans_chaque_scenario() {
    // Vivacite : tout entrelacement termine avec la tache MORTE (le tueur
    // passe toujours), jamais vivante en espace utilisateur ni parquee.
    for s in SCENARIOS {
        let depart_vivants = explore(Protocole::Nouveau, s);
        assert_eq!(depart_vivants.p3_mort_perdue, 0, "{s:?}");
        assert_eq!(depart_vivants.p2_resurrection, 0, "{s:?}");
    }
}

#[test]
fn l_ancien_et_le_nouveau_explorent_le_meme_ordre_de_grandeur() {
    // Garde-fou du modele : un protocole qui ne bougerait pas « ne violerait
    // rien ». Le nouveau doit explorer autant d'etats que l'ancien, a un
    // facteur pres.
    for s in SCENARIOS {
        let a = explore(Protocole::Ancien, s).etats;
        let n = explore(Protocole::Nouveau, s).etats;
        assert!(n * 4 >= a && a * 8 >= n, "{s:?} : ancien {a} etats, nouveau {n}");
    }
}
