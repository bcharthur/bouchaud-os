//! Une continuation synchrone n'est reprise QUE par la fin de sa racine.
//!
//! BOUCHAUD_CONTINUATION_SYNCHRONE_V1
//!
//! # L'incident
//!
//! TRIGKEY, T+60,519 s : `RUN_NOYAU_RETOUR nom=desktop fil_mort=0`, puis
//! quatorze `PROCESS_KILL raison=run_noyau_retour`. Le bureau etait VIVANT.
//! Sur le BSP, `KERNEL_CTX[0]` servait a la fois d'idle du coeur et de
//! continuation de `run_noyau` : la mort de n'importe quelle tache du coeur
//! zero, sans autre tache prete, la reprenait.
//!
//! # Ce que ce test rejoue
//!
//! Un modele de machine (coeurs, taches epinglees, une racine, une
//! continuation garee sur le coeur zero) soumis aux memes evenements sous
//! deux regles :
//!
//!   * l'ANCIENNE, ecrite ici : pas d'autre tache prete -> `KERNEL_CTX[cpu]`,
//!     qui sur le coeur zero EST la continuation ;
//!   * la NOUVELLE, `continuation::destination` / `reprenable`, le code meme
//!     que le noyau execute.
//!
//! Deux proprietes, verifiees a chaque pas :
//!
//!   surete   : la continuation n'est jamais reprise racine vivante ;
//!   vivacite : racine terminee et coeur proprietaire a l'idle (ou mort sur
//!              ce coeur) => la continuation est reprise dans le pas.
//!
//! L'ancienne regle doit violer les deux (temoin negatif) ; la nouvelle,
//! aucune. Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/scheduler/continuation.rs"]
mod continuation;

use continuation::{destination, reprenable, reveiller_proprietaire, Destination, Sortie};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Regle {
    Ancienne,
    Nouvelle,
}

#[derive(Clone, Debug)]
struct Tache {
    pid: u32,
    cpu: usize,
    vivante: bool,
    prete: bool,
}

#[derive(Default, Debug)]
struct Bilan {
    reprises: u32,
    reprises_racine_vivante: u32,
    /// Pas ou la continuation etait due, reprenable, et n'a PAS ete reprise.
    pertes: u32,
}

struct Machine {
    regle: Regle,
    taches: Vec<Tache>,
    racine: u32,
    garee: bool,
    cpu_continuation: usize,
    /// Coeurs a qui la nouvelle regle a demande de se reveiller.
    reveils: Vec<usize>,
    bilan: Bilan,
}

impl Machine {
    fn neuve(regle: Regle, cpus: usize) -> Self {
        let _ = cpus;
        Machine {
            regle,
            taches: Vec::new(),
            racine: 1,
            garee: true,
            cpu_continuation: 0,
            reveils: Vec::new(),
            bilan: Bilan::default(),
        }
    }

    fn ajoute(&mut self, pid: u32, cpu: usize, prete: bool) -> usize {
        self.taches.push(Tache { pid, cpu, vivante: true, prete });
        self.taches.len() - 1
    }

    fn racine_terminee(&self) -> bool {
        !self.taches.iter().any(|t| t.vivante && t.pid == self.racine)
    }

    fn autre_prete(&self, cpu: usize) -> bool {
        self.taches.iter().any(|t| t.vivante && t.prete && t.cpu == cpu)
    }

    fn reprend(&mut self) {
        self.bilan.reprises += 1;
        if !self.racine_terminee() {
            self.bilan.reprises_racine_vivante += 1;
        }
        self.garee = false;
    }

    /// La tache `i` meurt sur son coeur.
    fn meurt(&mut self, i: usize) {
        if !self.taches[i].vivante {
            return;
        }
        self.taches[i].vivante = false;
        let cpu = self.taches[i].cpu;
        let autre = self.autre_prete(cpu);
        match self.regle {
            Regle::Ancienne => {
                // `commute_sortie_definitive_si_possible`, puis
                // `switch_to_kernel` : KERNEL_CTX[cpu]. Sur le coeur zero,
                // c'est la continuation de `run_noyau`, quelle que soit la
                // tache qui meurt.
                if !autre && cpu == self.cpu_continuation && self.garee {
                    self.reprend();
                }
            }
            Regle::Nouvelle => {
                let s = Sortie {
                    cpu,
                    garee: self.garee,
                    cpu_continuation: self.cpu_continuation,
                    racine_terminee: self.racine_terminee(),
                    autre_prete: autre,
                };
                match destination(s) {
                    Destination::Continuation => self.reprend(),
                    Destination::AutreTache | Destination::Idle => {
                        if reveiller_proprietaire(s) {
                            self.reveils.push(self.cpu_continuation);
                        }
                    }
                }
            }
        }
        self.verifie_vivacite(cpu);
    }

    /// Un tour de boucle idle sur `cpu` (rien de pret). L'ancienne regle n'a
    /// pas de boucle idle sur le coeur zero : rien ne s'y passe.
    fn idle(&mut self, cpu: usize) {
        if self.regle == Regle::Nouvelle
            && reprenable(self.garee, self.racine_terminee(), cpu, self.cpu_continuation)
        {
            self.reprend();
        }
    }

    /// Apres une mort sur le coeur proprietaire sans autre tache prete, ou
    /// apres un passage idle, une continuation due doit avoir ete reprise.
    ///
    /// L'ancienne regle la laisse garee quand une autre tache est prete : la
    /// mort de la racine part vers elle, et plus rien ne la reprendra.
    fn verifie_vivacite(&mut self, cpu: usize) {
        if cpu == self.cpu_continuation && self.racine_terminee() && self.garee {
            self.bilan.pertes += 1;
        }
    }
}

/// L'incident, pas a pas : bureau vivant, une tache courte meurt sur le coeur
/// zero sans rien de pret.
fn incident(regle: Regle) -> Bilan {
    let mut m = Machine::neuve(regle, 4);
    let _bureau = m.ajoute(1, 0, false); // vivant, endormi
    let court = m.ajoute(42, 0, false);
    m.meurt(court);
    m.bilan
}

#[test]
fn ancien_protocole_reprend_la_continuation_d_une_racine_vivante() {
    let b = incident(Regle::Ancienne);
    assert_eq!(b.reprises, 1);
    assert_eq!(b.reprises_racine_vivante, 1, "le temoin negatif doit rougir");
}

#[test]
fn nouveau_protocole_mene_la_tache_etrangere_a_l_idle() {
    let b = incident(Regle::Nouvelle);
    assert_eq!(b.reprises, 0);
    assert_eq!(b.reprises_racine_vivante, 0);
}

/// Vingt morts courtes, racine vivante, avec et sans autre tache prete.
fn sorties_multiples(regle: Regle) -> Bilan {
    let mut m = Machine::neuve(regle, 2);
    m.ajoute(1, 0, false);
    for n in 0..20u32 {
        let prete = if n % 3 == 0 { Some(m.ajoute(500 + n, 0, true)) } else { None };
        let court = m.ajoute(100 + n, 0, false);
        m.meurt(court);
        if let Some(p) = prete {
            m.meurt(p);
        }
        m.idle(0);
    }
    m.bilan
}

#[test]
fn plusieurs_sorties_sur_le_meme_coeur_ne_reprennent_rien() {
    let ancien = sorties_multiples(Regle::Ancienne);
    assert!(ancien.reprises_racine_vivante >= 1);
    let nouveau = sorties_multiples(Regle::Nouvelle);
    assert_eq!(nouveau.reprises, 0);
    assert_eq!(nouveau.reprises_racine_vivante, 0);
}

/// La racine meurt alors qu'une autre tache est prete sur le coeur zero (le
/// constat SMP1 du lot B12 : `run_noyau` ne revenait plus).
fn racine_meurt_coeur_occupe(regle: Regle) -> (Bilan, bool) {
    let mut m = Machine::neuve(regle, 1);
    let racine = m.ajoute(1, 0, false);
    m.ajoute(77, 0, true); // travailleur perpetuel, toujours pret
    m.meurt(racine);
    (m.bilan, m.garee)
}

#[test]
fn la_mort_de_la_racine_reprend_meme_coeur_occupe() {
    let (ancien, encore_garee) = racine_meurt_coeur_occupe(Regle::Ancienne);
    assert_eq!(ancien.reprises, 0);
    assert!(encore_garee, "ancien protocole : continuation perdue");
    assert!(ancien.pertes >= 1);
    let (nouveau, garee) = racine_meurt_coeur_occupe(Regle::Nouvelle);
    assert_eq!(nouveau.reprises, 1);
    assert_eq!(nouveau.reprises_racine_vivante, 0);
    assert!(!garee);
}

#[test]
fn racine_vivante_jamais_reprise() {
    let mut m = Machine::neuve(Regle::Nouvelle, 4);
    m.ajoute(1, 0, false);
    for _ in 0..100 {
        m.idle(0);
    }
    assert_eq!(m.bilan.reprises, 0);
    assert!(m.garee);
}

/// SMP : la racine a deux fils ; le dernier meurt sur le coeur deux.
#[test]
fn dernier_fil_sur_un_autre_coeur_reveille_le_proprietaire() {
    let mut m = Machine::neuve(Regle::Nouvelle, 4);
    let a = m.ajoute(1, 0, false);
    let b = m.ajoute(1, 2, false);
    m.meurt(a);
    assert_eq!(m.bilan.reprises, 0, "un fil de la racine vit encore");
    m.meurt(b);
    assert_eq!(m.bilan.reprises, 0, "un AP ne reprend jamais la continuation");
    assert_eq!(m.reveils, vec![0]);
    m.idle(2);
    assert_eq!(m.bilan.reprises, 0);
    m.idle(0);
    assert_eq!(m.bilan.reprises, 1);
    assert_eq!(m.bilan.reprises_racine_vivante, 0);
}

/// Suites pseudo-aleatoires deterministes sur quatre coeurs.
fn hasard(regle: Regle, graine: u64) -> Bilan {
    let mut x = graine;
    let mut tire = move |n: u64| {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (x >> 33) % n
    };
    let mut m = Machine::neuve(regle, 4);
    m.ajoute(1, 0, false);
    let mut pid = 10u32;
    for _ in 0..400 {
        if !m.garee {
            break;
        }
        match tire(10) {
            0..=4 => {
                let cpu = tire(4) as usize;
                let prete = tire(2) == 0;
                pid += 1;
                m.ajoute(pid, cpu, prete);
            }
            5..=7 => {
                let vivantes: Vec<usize> = (0..m.taches.len())
                    .filter(|&i| m.taches[i].vivante && m.taches[i].pid != 1)
                    .collect();
                if !vivantes.is_empty() {
                    let i = vivantes[tire(vivantes.len() as u64) as usize];
                    m.meurt(i);
                }
            }
            8 => m.idle(tire(4) as usize),
            _ => {
                // La racine finit, rarement.
                if tire(20) == 0 {
                    let r = m.taches.iter().position(|t| t.pid == 1 && t.vivante);
                    if let Some(r) = r {
                        m.meurt(r);
                        m.idle(0);
                    }
                }
            }
        }
    }
    m.bilan
}

#[test]
fn suites_aleatoires_surete_et_vivacite() {
    let mut ancien = Bilan::default();
    let mut nouveau = Bilan::default();
    for graine in 0..2_000u64 {
        let a = hasard(Regle::Ancienne, graine);
        ancien.reprises += a.reprises;
        ancien.reprises_racine_vivante += a.reprises_racine_vivante;
        ancien.pertes += a.pertes;
        let n = hasard(Regle::Nouvelle, graine);
        nouveau.reprises += n.reprises;
        nouveau.reprises_racine_vivante += n.reprises_racine_vivante;
        nouveau.pertes += n.pertes;
    }
    println!(
        "CONTINUATION ancien: reprises={} racine_vivante={} pertes={} | nouveau: reprises={} racine_vivante={} pertes={}",
        ancien.reprises, ancien.reprises_racine_vivante, ancien.pertes,
        nouveau.reprises, nouveau.reprises_racine_vivante, nouveau.pertes,
    );
    assert!(ancien.reprises_racine_vivante > 0, "temoin negatif inerte");
    assert!(ancien.pertes > 0, "temoin de perte inerte");
    assert_eq!(nouveau.reprises_racine_vivante, 0);
    assert_eq!(nouveau.pertes, 0);
    assert!(nouveau.reprises > 0, "la vivacite n'a jamais ete exercee");
}
