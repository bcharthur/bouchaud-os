//! Une tache prete peut-elle rester six secondes sans coeur ?
//!
//! # Le releve qu'il faut reproduire
//!
//! TRIGKEY, seize coeurs, Ladybird en charge :
//!
//! ```text
//! hid_wake_to_run_max_us = 6782927
//! hid_run_to_lock_max_us = 2
//! hid_poll_body_max_us   = 162
//! ```
//!
//! Le fil `usb-hid` n'est ni lent ni bloque : il n'est pas ELU. Les trois
//! quarts de ce fichier ne testent donc pas une fonction, ils REJOUENT le
//! chemin de reveil -- publication, mise en file, IPI, quantum, vol -- avec
//! les deux politiques, et comparent.
//!
//! La simulation encode les CINQ exclusions lues dans le noyau, et rien
//! d'autre :
//!
//!   1. l'IPI de publication n'est envoye que si le coeur cible DORT ;
//!   2. l'IPI recu ne fait rien si la tache interrompue est une tache NOYAU ;
//!   3. le balayage de quantum saute les coeurs qui executent une tache
//!      NOYAU ;
//!   4. aucun fil noyau n'atteint de point sur ;
//!   5. la pression volable ne compte que la bande NORMALE.
//!
//! Avec la politique historique, `reproduit_le_defaut_physique` mesure une
//! attente qui depasse la seconde et ne converge pas : c'est le releve. Avec
//! la politique ciblee, la meme charge tient sous deux millisecondes.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/scheduler/reveil.rs"]
mod reveil;

use reveil::{
    choisit_coeur, decide, pression_de_secours, Classe, Coeur, Decision, Reveille,
    BUDGET_ACTIVATION_NS, RESIDENCE_MINIMALE_NS,
};

// ===========================================================================
// La politique, en isolation
// ===========================================================================

fn sensible() -> Reveille {
    Reveille { classe: Classe::Interactive, sensible: true, consomme_ns: 160_000 }
}

fn ordinaire() -> Reveille {
    Reveille { classe: Classe::Interactive, sensible: false, consomme_ns: 0 }
}

fn coeur_libre() -> Coeur {
    Coeur { en_ligne: true, autorise: true, inactif: true, ..Coeur::default() }
}

fn coeur_occupe(classe: Classe, occupant_sensible: bool, residence_ns: u64) -> Coeur {
    Coeur {
        en_ligne: true,
        autorise: true,
        inactif: false,
        occupant_sensible,
        occupant_classe: classe,
        residence_ns,
        ..Coeur::default()
    }
}

#[test]
fn un_coeur_au_repos_se_reveille_immediatement() {
    assert_eq!(decide(&coeur_libre(), &sensible()), Decision::ReveilImmediat);
    assert_eq!(decide(&coeur_libre(), &ordinaire()), Decision::ReveilImmediat);
}

#[test]
fn un_occupant_de_classe_inferieure_est_coupe() {
    let cible = coeur_occupe(Classe::Normale, false, 50_000_000);
    assert_eq!(decide(&cible, &sensible()), Decision::PreemptionCiblee);
}

#[test]
fn un_occupant_de_meme_classe_est_coupe_aussi() {
    // L'occupant est interactif, mais il n'est pas sensible a la latence :
    // ceder 162 us ne lui coute rien de mesurable.
    let cible = coeur_occupe(Classe::Interactive, false, 50_000_000);
    assert_eq!(decide(&cible, &sensible()), Decision::PreemptionCiblee);
}

#[test]
fn un_pair_sensible_garde_sa_tranche_minimale() {
    let jeune = coeur_occupe(Classe::Interactive, true, RESIDENCE_MINIMALE_NS - 1);
    assert_eq!(decide(&jeune, &sensible()), Decision::DemandeDifferee);

    let installe = coeur_occupe(Classe::Interactive, true, RESIDENCE_MINIMALE_NS);
    assert_eq!(decide(&installe, &sensible()), Decision::PreemptionCiblee);
}

#[test]
fn une_tache_ordinaire_ne_coupe_personne() {
    let cible = coeur_occupe(Classe::Normale, false, 50_000_000);
    assert_eq!(decide(&cible, &ordinaire()), Decision::DemandeDifferee);
}

#[test]
fn un_coeur_hors_ligne_ne_recoit_rien() {
    let mut cible = coeur_libre();
    cible.en_ligne = false;
    assert_eq!(decide(&cible, &sensible()), Decision::MiseEnFile);
}

#[test]
fn le_budget_desarme_le_privilege() {
    let mut derive = sensible();
    derive.consomme_ns = BUDGET_ACTIVATION_NS;
    assert!(!derive.privilegiee());
    // Elle redevient une interactive ordinaire : elle ne coupe plus personne.
    let cible = coeur_occupe(Classe::Normale, false, 50_000_000);
    assert_eq!(decide(&cible, &derive), Decision::DemandeDifferee);
    // Et elle ne demande plus de placement particulier.
    assert_eq!(choisit_coeur(3, &derive, &[coeur_libre(), coeur_libre()]), None);
}

#[test]
fn le_placement_prefere_le_coeur_precedent_quand_il_dort() {
    let coeurs = [coeur_libre(), coeur_libre(), coeur_libre()];
    assert_eq!(choisit_coeur(2, &sensible(), &coeurs), Some(2));
    assert_eq!(choisit_coeur(0, &sensible(), &coeurs), Some(0));
}

#[test]
fn le_placement_quitte_un_coeur_occupe_pour_un_coeur_au_repos() {
    let coeurs = [
        coeur_occupe(Classe::Normale, false, 10_000_000),
        coeur_libre(),
        coeur_occupe(Classe::Interactive, false, 10_000_000),
    ];
    // Le coeur precedent (0) travaille : la residence de cache ne vaut pas
    // une attente.
    assert_eq!(choisit_coeur(0, &sensible(), &coeurs), Some(1));
}

#[test]
fn le_placement_evite_un_pair_sensible_quand_tout_travaille() {
    let coeurs = [
        coeur_occupe(Classe::Interactive, true, 10_000_000),
        coeur_occupe(Classe::Normale, false, 10_000_000),
    ];
    assert_eq!(choisit_coeur(0, &sensible(), &coeurs), Some(1));
}

#[test]
fn le_placement_prefere_la_file_la_plus_courte() {
    let mut charge = coeur_occupe(Classe::Normale, false, 10_000_000);
    charge.attente_interactive = 3;
    let mut leger = coeur_occupe(Classe::Normale, false, 10_000_000);
    leger.attente_normale = 1;
    assert_eq!(choisit_coeur(0, &sensible(), &[charge, leger]), Some(1));
}

#[test]
fn le_placement_respecte_l_affinite_et_la_mise_en_ligne() {
    let mut interdit = coeur_libre();
    interdit.autorise = false;
    let mut eteint = coeur_libre();
    eteint.en_ligne = false;
    let coeurs = [interdit, eteint, coeur_occupe(Classe::Normale, false, 0)];
    assert_eq!(choisit_coeur(0, &sensible(), &coeurs), Some(2));

    let aucun = [interdit, eteint];
    assert_eq!(choisit_coeur(0, &sensible(), &aucun), None);
}

#[test]
fn la_pression_de_secours_compte_les_deux_bandes() {
    // C'est la cinquieme exclusion : une interactive seule en attente rendait
    // une pression NULLE, donc aucun voleur.
    assert_eq!(pression_de_secours(1, 0), 1);
    assert_eq!(pression_de_secours(0, 1), 1);
    assert_eq!(pression_de_secours(2, 3), 5);
}

// ===========================================================================
// La simulation du chemin de reveil
// ===========================================================================

const PAS_US: u64 = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Politique {
    /// Les cinq exclusions, telles qu'elles sont dans le noyau avant ce lot.
    Historique,
    /// Placement au reveil + preemption ciblee + secours par vol.
    Ciblee,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Etat {
    Pret,
    Court(usize),
    Dort(u64),
}

#[derive(Clone)]
struct Tache {
    classe: Classe,
    sensible: bool,
    /// Une tache NOYAU : c'est le predicat sur lequel les cinq exclusions se
    /// referment.
    noyau: bool,
    etat: Etat,
    pret_depuis_us: u64,
    /// Duree d'une rafale de travail. `None` : la tache ne s'arrete jamais.
    rafale_us: Option<u64>,
    periode_us: u64,
    reste_us: u64,
    consomme_activation_us: u64,
    consomme_precedent_us: u64,
    dernier_coeur: usize,
    travail_total_us: u64,
    reveils: u64,
}

impl Tache {
    fn charge_noyau(coeur: usize) -> Self {
        Self {
            classe: Classe::Interactive,
            sensible: false,
            noyau: true,
            etat: Etat::Pret,
            pret_depuis_us: 0,
            rafale_us: None,
            periode_us: 0,
            reste_us: u64::MAX,
            consomme_activation_us: 0,
            consomme_precedent_us: 0,
            dernier_coeur: coeur,
            travail_total_us: 0,
            reveils: 0,
        }
    }

    fn periodique(coeur: usize, periode_us: u64, rafale_us: u64, sensible: bool) -> Self {
        Self {
            classe: Classe::Interactive,
            sensible,
            noyau: true,
            etat: Etat::Dort(0),
            pret_depuis_us: 0,
            rafale_us: Some(rafale_us),
            periode_us,
            reste_us: rafale_us,
            consomme_activation_us: 0,
            consomme_precedent_us: 0,
            dernier_coeur: coeur,
            travail_total_us: 0,
            reveils: 0,
        }
    }
}

#[derive(Clone, Default)]
struct CoeurSim {
    occupant: Option<usize>,
    /// Les deux bandes, dans l'ordre de service.
    interactive: Vec<usize>,
    normale: Vec<usize>,
    depuis_us: u64,
    need_resched: bool,
    ipi: bool,
    /// La demande CIBLEE, seule autorisee a couper un fil noyau.
    ciblee: bool,
}

impl CoeurSim {
    fn inactif(&self) -> bool {
        self.occupant.is_none()
    }

    fn enfile(&mut self, tache: usize, classe: Classe) {
        let bande = match classe {
            Classe::Interactive => &mut self.interactive,
            Classe::Normale => &mut self.normale,
        };
        if !bande.contains(&tache) {
            bande.push(tache);
        }
    }

    fn defile(&mut self) -> Option<usize> {
        if !self.interactive.is_empty() {
            return Some(self.interactive.remove(0));
        }
        if !self.normale.is_empty() {
            return Some(self.normale.remove(0));
        }
        None
    }

    /// `FileCpu::vole` : la bande NORMALE d'abord, l'interactive ensuite.
    fn vole(&mut self) -> Option<usize> {
        if let Some(t) = self.normale.pop() {
            return Some(t);
        }
        self.interactive.pop()
    }
}

struct Sim {
    politique: Politique,
    coeurs: Vec<CoeurSim>,
    taches: Vec<Tache>,
    maintenant_us: u64,
    /// Le critere d'acceptation : `hid_wake_to_run_max_us`.
    wake_to_run_max_us: u64,
    commutations: u64,
    preemptions_noyau: u64,
    vols: u64,
    demarree: bool,
}

impl Sim {
    fn neuve(politique: Politique, nb_coeurs: usize, taches: Vec<Tache>) -> Self {
        Self {
            politique,
            coeurs: vec![CoeurSim::default(); nb_coeurs],
            taches,
            maintenant_us: 0,
            wake_to_run_max_us: 0,
            commutations: 0,
            preemptions_noyau: 0,
            vols: 0,
            demarree: false,
        }
    }

    /// L'amorcage : les taches deja pretes sont publiees, puis une passe
    /// d'election les installe. Sans cela la premiere milliseconde mesurerait
    /// un demarrage a froid et non le regime etabli.
    fn demarre(&mut self) {
        if self.demarree {
            return;
        }
        self.demarree = true;
        for t in 0..self.taches.len() {
            if self.taches[t].etat == Etat::Pret {
                self.taches[t].pret_depuis_us = 0;
                self.publie(t);
            }
        }
        for c in 0..self.coeurs.len() {
            self.elit(c);
        }
    }

    fn instantane(&self, indice: usize, precedent_du_reveille: usize) -> Coeur {
        let _ = precedent_du_reveille;
        let coeur = &self.coeurs[indice];
        let (occupant_sensible, occupant_classe) = match coeur.occupant {
            Some(t) => (self.taches[t].sensible, self.taches[t].classe),
            None => (false, Classe::Normale),
        };
        Coeur {
            en_ligne: true,
            autorise: true,
            inactif: coeur.inactif(),
            occupant_sensible,
            occupant_classe,
            residence_ns: self.maintenant_us.saturating_sub(coeur.depuis_us) * 1_000,
            attente_interactive: coeur.interactive.len(),
            attente_normale: coeur.normale.len(),
        }
    }

    fn publie(&mut self, t: usize) {
        let precedent = self.taches[t].dernier_coeur;
        let reveille = Reveille {
            classe: self.taches[t].classe,
            sensible: self.taches[t].sensible,
            consomme_ns: self.taches[t].consomme_precedent_us * 1_000,
        };

        let cible = match self.politique {
            Politique::Historique => precedent,
            Politique::Ciblee => {
                let coeurs: Vec<Coeur> =
                    (0..self.coeurs.len()).map(|i| self.instantane(i, precedent)).collect();
                choisit_coeur(precedent, &reveille, &coeurs).unwrap_or(precedent)
            }
        };

        self.taches[t].dernier_coeur = cible;
        let classe = self.taches[t].classe;
        self.coeurs[cible].enfile(t, classe);

        // La barriere du noyau : l'etat du coeur est relu APRES la mise en
        // file, jamais avant. C'est ce qui ferme le reveil perdu.
        match self.politique {
            Politique::Historique => {
                self.coeurs[cible].need_resched = true;
                if self.coeurs[cible].inactif() {
                    self.coeurs[cible].ipi = true;
                }
            }
            Politique::Ciblee => {
                let etat = self.instantane(cible, precedent);
                match decide(&etat, &reveille) {
                    Decision::ReveilImmediat => {
                        self.coeurs[cible].need_resched = true;
                        self.coeurs[cible].ipi = true;
                    }
                    Decision::PreemptionCiblee => {
                        self.coeurs[cible].need_resched = true;
                        self.coeurs[cible].ipi = true;
                        self.coeurs[cible].ciblee = true;
                    }
                    Decision::DemandeDifferee => {
                        self.coeurs[cible].need_resched = true;
                    }
                    Decision::MiseEnFile => {}
                }
            }
        }
    }

    /// Le coeur peut-il commuter maintenant ?
    fn commutation_permise(&self, c: usize) -> bool {
        let coeur = &self.coeurs[c];
        let Some(occupant) = coeur.occupant else {
            return true;
        };
        if !coeur.need_resched {
            return false;
        }
        if !self.taches[occupant].noyau {
            // Tache utilisateur : l'IPI de quantum la coupe, ou le point sur
            // au retour d'appel systeme.
            return true;
        }
        // Tache NOYAU. C'est ici que le noyau d'avant ce lot ne faisait RIEN.
        match self.politique {
            Politique::Historique => false,
            Politique::Ciblee => coeur.ciblee,
        }
    }

    fn elit(&mut self, c: usize) {
        if !self.commutation_permise(c) {
            return;
        }
        let Some(suivant) = self.coeurs[c].defile().or_else(|| self.vole_pour(c)) else {
            // Rien a elire : la demande tombe, comme dans `pick_next`.
            if self.coeurs[c].occupant.is_none() {
                self.coeurs[c].need_resched = false;
                self.coeurs[c].ciblee = false;
                self.coeurs[c].ipi = false;
            }
            return;
        };
        if let Some(sortant) = self.coeurs[c].occupant {
            if sortant == suivant {
                return;
            }
            if self.taches[sortant].noyau {
                self.preemptions_noyau += 1;
            }
            self.taches[sortant].etat = Etat::Pret;
            self.taches[sortant].pret_depuis_us = self.maintenant_us;
            let classe = self.taches[sortant].classe;
            self.coeurs[c].enfile(sortant, classe);
        }
        let attente = self.maintenant_us.saturating_sub(self.taches[suivant].pret_depuis_us);
        if self.taches[suivant].sensible && self.wake_to_run_max_us < attente {
            self.wake_to_run_max_us = attente;
        }
        self.taches[suivant].etat = Etat::Court(c);
        self.taches[suivant].consomme_activation_us = 0;
        self.coeurs[c].occupant = Some(suivant);
        self.coeurs[c].depuis_us = self.maintenant_us;
        self.coeurs[c].need_resched = false;
        self.coeurs[c].ciblee = false;
        self.coeurs[c].ipi = false;
        self.commutations += 1;
    }

    fn vole_pour(&mut self, voleur: usize) -> Option<usize> {
        if self.coeurs[voleur].occupant.is_some() {
            return None;
        }
        let mut meilleur: Option<(usize, usize)> = None;
        for c in 0..self.coeurs.len() {
            if c == voleur {
                continue;
            }
            let pression = match self.politique {
                // Cinquieme exclusion : la bande NORMALE seule.
                Politique::Historique => self.coeurs[c].normale.len(),
                Politique::Ciblee => pression_de_secours(
                    self.coeurs[c].interactive.len(),
                    self.coeurs[c].normale.len(),
                ),
            };
            if pression >= 1 && meilleur.map(|(_, p)| pression > p).unwrap_or(true) {
                meilleur = Some((c, pression));
            }
        }
        let (donneur, _) = meilleur?;
        let vole = self.coeurs[donneur].vole()?;
        self.taches[vole].dernier_coeur = voleur;
        self.vols += 1;
        Some(vole)
    }

    fn pas(&mut self) {
        // 0. L'attente EN COURS des taches sensibles.
        //
        // Le noyau ferme sa mesure a l'election. Ici il faut la suivre en
        // continu : avec la politique historique l'election n'arrive JAMAIS,
        // et une mesure fermee a l'election resterait a zero -- elle
        // raconterait l'inverse du defaut.
        for t in 0..self.taches.len() {
            if self.taches[t].sensible && self.taches[t].etat == Etat::Pret {
                let attente = self.maintenant_us.saturating_sub(self.taches[t].pret_depuis_us);
                if self.wake_to_run_max_us < attente {
                    self.wake_to_run_max_us = attente;
                }
            }
        }

        // 1. Les reveils dus.
        for t in 0..self.taches.len() {
            if let Etat::Dort(jusqua) = self.taches[t].etat {
                if jusqua <= self.maintenant_us {
                    self.taches[t].etat = Etat::Pret;
                    self.taches[t].pret_depuis_us = self.maintenant_us;
                    self.taches[t].reveils += 1;
                    self.taches[t].reste_us = self.taches[t].rafale_us.unwrap_or(u64::MAX);
                    self.publie(t);
                }
            }
        }

        // 2. Le balayage de quantum. Historique : il saute les coeurs qui
        //    executent une tache NOYAU (`running_user_cpu_mask`).
        if self.maintenant_us % 4_000 == 0 {
            for c in 0..self.coeurs.len() {
                let noyau = self.coeurs[c].occupant.map(|t| self.taches[t].noyau).unwrap_or(false);
                let a_du_travail =
                    !self.coeurs[c].interactive.is_empty() || !self.coeurs[c].normale.is_empty();
                if a_du_travail && !noyau {
                    self.coeurs[c].need_resched = true;
                }
            }
        }

        // 3. Election : coeurs au repos d'abord, puis les demandes.
        for c in 0..self.coeurs.len() {
            self.elit(c);
        }

        // 4. Le travail avance.
        for c in 0..self.coeurs.len() {
            let Some(t) = self.coeurs[c].occupant else { continue };
            self.taches[t].travail_total_us += PAS_US;
            self.taches[t].consomme_activation_us += PAS_US;
            if self.taches[t].reste_us != u64::MAX {
                self.taches[t].reste_us = self.taches[t].reste_us.saturating_sub(PAS_US);
                if self.taches[t].reste_us == 0 {
                    let periode = self.taches[t].periode_us;
                    self.taches[t].consomme_precedent_us = self.taches[t].consomme_activation_us;
                    self.taches[t].etat = Etat::Dort(self.maintenant_us + periode);
                    self.coeurs[c].occupant = None;
                    self.coeurs[c].depuis_us = self.maintenant_us;
                }
            }
        }

        self.maintenant_us += PAS_US;
    }

    fn joue(&mut self, duree_us: u64) {
        self.demarre();
        while self.maintenant_us < duree_us {
            self.pas();
        }
    }
}

/// La charge du releve physique : seize coeurs, seize fils NOYAU interactifs
/// qui ne rendent jamais la main, et un fil periodique sensible.
fn charge_physique(politique: Politique, sensible: bool) -> Sim {
    let mut taches: Vec<Tache> = (0..16).map(Tache::charge_noyau).collect();
    // Le fil HID : reveil toutes les millisecondes, 160 us de travail, place
    // au depart sur le coeur 7 -- un coeur deja occupe.
    taches.push(Tache::periodique(7, 1_000, 160, sensible));
    Sim::neuve(politique, 16, taches)
}

#[test]
fn reproduit_le_defaut_physique() {
    let mut sim = charge_physique(Politique::Historique, true);
    sim.joue(200_000);

    let hid = sim.taches.len() - 1;
    // Le releve : la tache est prete, elle n'est pas elue.
    assert!(
        sim.wake_to_run_max_us >= 50_000,
        "le defaut n'est pas reproduit : wake_to_run_max={} us",
        sim.wake_to_run_max_us
    );
    // Et elle ne tourne pas du tout : zero rafale servie en 200 ms.
    assert_eq!(
        sim.taches[hid].travail_total_us, 0,
        "la politique historique aurait servi le fil HID"
    );
    assert_eq!(sim.preemptions_noyau, 0, "un fil noyau aurait ete preempte");
    assert_eq!(sim.vols, 0, "la pression volable aurait trouve un donneur");
}

#[test]
fn la_politique_ciblee_borne_l_attente() {
    let mut sim = charge_physique(Politique::Ciblee, true);
    sim.joue(200_000);

    let hid = sim.taches.len() - 1;
    assert!(
        sim.wake_to_run_max_us < 2_000,
        "wake_to_run_max={} us : le critere produit est 30 ms, la cible 10 ms",
        sim.wake_to_run_max_us
    );
    // Elle a bien tourne : environ une rafale par milliseconde.
    // La periode court depuis la FIN de la rafale : un cycle vaut donc
    // 1000 + 160 us, soit environ 172 rafales en 200 ms.
    assert!(
        sim.taches[hid].reveils >= 165,
        "reveils={} pour 200 ms a 1 kHz",
        sim.taches[hid].reveils
    );
    assert!(
        sim.taches[hid].travail_total_us >= 165 * 160,
        "travail servi={} us",
        sim.taches[hid].travail_total_us
    );
    assert!(sim.preemptions_noyau > 0, "aucun fil noyau n'a cede");
}

#[test]
fn aucune_famine_des_taches_normales() {
    let mut sim = charge_physique(Politique::Ciblee, true);
    sim.joue(200_000);

    // Seize coeurs pendant 200 ms, c'est 3 200 000 us de processeur. Le fil
    // sensible en prend 160 par milliseconde, soit 32 000 -- un pour cent.
    let charge: u64 = sim.taches[..16].iter().map(|t| t.travail_total_us).sum();
    let capacite = 16 * 200_000;
    assert!(
        charge * 100 >= capacite * 97,
        "la charge n'a recu que {} us sur {} : le privilege affame les autres",
        charge,
        capacite
    );
    // Et AUCUN des seize ne se fait oublier.
    for (indice, tache) in sim.taches[..16].iter().enumerate() {
        assert!(
            tache.travail_total_us >= 150_000,
            "la tache de charge {} n'a recu que {} us",
            indice,
            tache.travail_total_us
        );
    }
}

#[test]
fn le_privilege_n_est_pas_une_priorite_permanente() {
    // Le meme fil, mais SANS la propriete `latency_sensitive` : il retombe
    // exactement dans le comportement historique. C'est la preuve que le gain
    // vient de la propriete declaree et non d'un effet de bord du placement.
    let mut sim = charge_physique(Politique::Ciblee, false);
    sim.joue(200_000);
    let hid = sim.taches.len() - 1;
    assert_eq!(sim.taches[hid].travail_total_us, 0);
    assert_eq!(sim.preemptions_noyau, 0);
}

#[test]
fn une_tache_sensible_qui_derive_perd_son_privilege() {
    // Rafale de 10 ms : au-dela du budget d'activation. Elle est servie la
    // premiere fois -- son activation precedente etait vide -- puis se
    // desarme.
    let mut taches: Vec<Tache> = (0..4).map(Tache::charge_noyau).collect();
    taches.push(Tache::periodique(1, 1_000, 10_000, true));
    let mut sim = Sim::neuve(Politique::Ciblee, 4, taches);
    sim.joue(200_000);

    let derive = sim.taches.len() - 1;
    assert!(sim.taches[derive].consomme_precedent_us * 1_000 >= BUDGET_ACTIVATION_NS);
    // Elle a tourne au moins une fois, et n'a pas monopolise les quatre coeurs.
    assert!(sim.taches[derive].travail_total_us >= 10_000);
    let charge: u64 = sim.taches[..4].iter().map(|t| t.travail_total_us).sum();
    assert!(
        charge * 100 >= 4 * 200_000 * 90,
        "la tache derivante a pris {} us aux quatre coeurs",
        charge
    );
}

#[test]
fn le_placement_seul_suffit_quand_des_coeurs_dorment() {
    // Deux fils de charge sur seize coeurs : c'est la machine du releve avec
    // Ladybird a 109 % de processeur. Aucune preemption ne doit etre
    // necessaire.
    let mut taches: Vec<Tache> = (0..2).map(Tache::charge_noyau).collect();
    taches.push(Tache::periodique(0, 1_000, 160, true));
    let mut sim = Sim::neuve(Politique::Ciblee, 16, taches);
    sim.joue(100_000);

    assert!(
        sim.wake_to_run_max_us <= PAS_US,
        "wake_to_run_max={} us alors que quatorze coeurs dorment",
        sim.wake_to_run_max_us
    );
    assert_eq!(
        sim.preemptions_noyau, 0,
        "le placement aurait du suffire : {} preemptions",
        sim.preemptions_noyau
    );
}

#[test]
fn le_reveil_distant_ne_se_perd_pas() {
    // La course : le coeur cible passe au repos pendant qu'on publie. Le
    // noyau relit `is_idle` APRES la mise en file et APRES la barriere ; la
    // simulation fait de meme. Aucune tache ne doit rester prete sans coeur a
    // la fin du jeu.
    for depart in 0..8usize {
        let mut taches: Vec<Tache> = (0..3).map(Tache::charge_noyau).collect();
        taches.push(Tache::periodique(depart, 1_000, 160, true));
        let mut sim = Sim::neuve(Politique::Ciblee, 8, taches);
        sim.joue(50_000);
        let hid = sim.taches.len() - 1;
        assert!(
            sim.taches[hid].reveils >= 38,
            "depart={} : seulement {} reveils",
            depart,
            sim.taches[hid].reveils
        );
        assert!(sim.wake_to_run_max_us < 2_000, "depart={}", depart);
    }
}

#[test]
fn plusieurs_reveils_rapproches_ne_se_marchent_pas_dessus() {
    // Quatre fils sensibles a 1 kHz sur quatre coeurs deja pris. La borne de
    // residence minimale empeche le ping-pong : chacun doit etre servi, et le
    // nombre de commutations doit rester proportionne.
    let mut taches: Vec<Tache> = (0..4).map(Tache::charge_noyau).collect();
    for i in 0..4 {
        taches.push(Tache::periodique(i, 1_000, 200, true));
    }
    let mut sim = Sim::neuve(Politique::Ciblee, 4, taches);
    sim.joue(100_000);

    for i in 4..8 {
        assert!(
            sim.taches[i].reveils >= 75,
            "le fil sensible {} n'a ete reveille que {} fois",
            i,
            sim.taches[i].reveils
        );
    }
    assert!(sim.wake_to_run_max_us < 3_000, "max={} us", sim.wake_to_run_max_us);
    // Un ping-pong ferait exploser ce chiffre : quatre reveils par
    // milliseconde, cent millisecondes, c'est 400 rafales et donc de l'ordre
    // de 800 commutations.
    assert!(
        sim.commutations < 4_000,
        "{} commutations : la tranche minimale ne tient pas",
        sim.commutations
    );
}

#[test]
fn le_vol_recupere_une_interactive_oubliee() {
    // Le cas de la cinquieme exclusion, isole : une interactive en attente
    // derriere un occupant, et un coeur au repos a cote.
    // Les deux taches declarent le MEME coeur precedent : la seconde reste
    // donc en file derriere la premiere, exactement comme `usb-hid` derriere
    // son occupant.
    let mut historique = Sim::neuve(
        Politique::Historique,
        2,
        vec![Tache::charge_noyau(0), Tache::charge_noyau(0)],
    );
    historique.joue(10_000);
    assert_eq!(historique.vols, 0, "la bande normale seule n'offre rien a voler");
    assert_eq!(historique.taches[1].travail_total_us, 0);

    let mut ciblee = Sim::neuve(
        Politique::Ciblee,
        2,
        vec![Tache::charge_noyau(0), Tache::charge_noyau(0)],
    );
    ciblee.joue(10_000);
    assert_eq!(ciblee.vols, 1);
    assert!(ciblee.taches[1].travail_total_us > 0);
}
