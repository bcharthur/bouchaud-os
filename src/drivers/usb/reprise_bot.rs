//! L'etat du transport Bulk-Only, et ce qu'on a le droit d'en attendre.
//!
//! # Le defaut que ceci corrige
//!
//! Le transport BOT n'avait pas d'etat. Chaque commande partait comme si la
//! precedente s'etait bien terminee, y compris quand elle avait expire. Or
//! une commande qui expire laisse derriere elle :
//!
//!   * un TD encore poste sur l'anneau du point de terminaison, que le
//!     peripherique peut achever PLUS TARD -- et dont l'evenement sera alors
//!     pris pour la reponse de la commande SUIVANTE ;
//!   * un peripherique qui attend toujours sa phase de donnees ou son CSW,
//!     donc un desaccord de phase que rien ne resorbe ;
//!   * un tampon DMA dans lequel le peripherique peut encore ecrire.
//!
//! C'est la raison pour laquelle raccourcir le budget d'attente, seul, aurait
//! ete pire que ne rien faire : abandonner plus tot, c'est abandonner plus
//! souvent, et chaque abandon empoisonnait le suivant. Raccourcir le budget
//! N'EST SUR qu'accompagne d'une reprise qui purge ce que l'abandon a laisse.
//!
//! # La regle
//!
//! `Pret` est le seul etat qui autorise une entree-sortie. Un incident fait
//! passer en `Reprise`, ou AUCUNE entree-sortie nouvelle n'est acceptee tant
//! que la procedure de reprise n'a pas rendu le materiel coherent. Elle
//! reussit : retour a `Pret`. Elle echoue trop souvent : `HorsService`, qui
//! refuse IMMEDIATEMENT -- sans attente, sans verrou tenu, et donc sans rien
//! couter au clavier.
//!
//! `HorsService` est un etat de service, pas une mort : un support retire et
//! rebranche repasse par l'enumeration, qui remet `Pret`.
//!
//! Module PUR : ni `use crate::`, ni `unsafe`. L'horloge est un argument.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

/// Tentatives de reprise avant de declarer le transport hors service.
///
/// Trois, parce que la procedure de reprise elle-meme emet des transferts de
/// controle : un peripherique qui ne repond plus du tout les fait echouer
/// aussi, et insister indefiniment tiendrait le verrou du pilote pour rien.
pub const REPRISES_AVANT_HORS_SERVICE: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Etat {
    /// Le materiel est coherent. C'est le seul etat qui laisse passer une
    /// entree-sortie.
    Pret = 0,
    /// Un incident est survenu. Rien ne passe avant la reprise.
    Reprise = 1,
    /// La reprise a echoue trop souvent. On refuse tout de suite.
    HorsService = 2,
}

impl Etat {
    pub fn code(self) -> u8 {
        self as u8
    }
    pub fn depuis_code(code: u8) -> Self {
        match code {
            1 => Self::Reprise,
            2 => Self::HorsService,
            _ => Self::Pret,
        }
    }
    pub fn nom(self) -> &'static str {
        match self {
            Self::Pret => "pret",
            Self::Reprise => "reprise",
            Self::HorsService => "hors-service",
        }
    }
}

/// Ou en etait la commande quand l'incident est survenu.
///
/// La phase est ce qui distingue « la cle n'a pas pris ma commande » de
/// « elle a pris ma commande, rendu mes donnees, et perdu son statut ». Les
/// deux se reprennent, mais elles ne se diagnostiquent pas pareil, et sans ce
/// champ les trois se lisaient identiques dans l'archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Repos = 0,
    Commande = 1,
    Donnees = 2,
    Statut = 3,
}

impl Phase {
    pub fn code(self) -> u8 {
        self as u8
    }
    pub fn depuis_code(code: u8) -> Self {
        match code {
            1 => Self::Commande,
            2 => Self::Donnees,
            3 => Self::Statut,
            _ => Self::Repos,
        }
    }
    pub fn nom(self) -> &'static str {
        match self {
            Self::Repos => "repos",
            Self::Commande => "commande",
            Self::Donnees => "donnees",
            Self::Statut => "statut",
        }
    }
}

/// Ce qui casse un transport BOT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Incident {
    /// Le budget d'attente a expire : un TD reste poste.
    Echeance = 1,
    /// Le point de terminaison est arrete (STALL).
    PointArrete = 2,
    /// Le CSW est illisible ou tronque.
    StatutInvalide = 3,
    /// Le peripherique et l'hote ne sont plus d'accord sur la phase.
    PhaseIncoherente = 4,
}

impl Incident {
    pub fn code(self) -> u8 {
        self as u8
    }
    pub fn nom(self) -> &'static str {
        match self {
            Self::Echeance => "echeance",
            Self::PointArrete => "point-arrete",
            Self::StatutInvalide => "statut-invalide",
            Self::PhaseIncoherente => "phase-incoherente",
        }
    }
    /// Un TD est-il reste poste sur l'anneau ?
    ///
    /// Une echeance, oui : le controleur n'a rien rendu. Un STALL ou un CSW
    /// refuse, non : l'evenement est arrive, le TD est consomme. La reprise
    /// doit reposer le pointeur de file dans le premier cas SEULEMENT, et
    /// confondre les deux reinitialiserait un anneau sain.
    pub fn laisse_un_td(self) -> bool {
        matches!(self, Self::Echeance)
    }
}

/// Ce qu'un releve doit montrer du transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Releve {
    pub etat: Etat,
    pub derniere_phase: Phase,
    pub dernier_incident: u8,
    pub dernier_slot: u8,
    pub dernier_dci: u8,
    pub echeances: u64,
    pub reprises: u64,
    pub reprises_reussies: u64,
    pub reprises_echouees: u64,
    pub tentatives_en_cours: u8,
    pub refus: u64,
    pub dernier_incident_ns: u64,
}

impl Releve {
    /// Le transport laisse-t-il passer une entree-sortie ?
    pub fn autorise(&self) -> bool {
        self.etat == Etat::Pret
    }
}

pub struct Transport {
    etat: AtomicU8,
    phase: AtomicU8,
    dernier_incident: AtomicU8,
    dernier_slot: AtomicU8,
    dernier_dci: AtomicU8,
    tentatives: AtomicU8,
    echeances: AtomicU64,
    reprises: AtomicU64,
    reussies: AtomicU64,
    echouees: AtomicU64,
    refus: AtomicU64,
    dernier_incident_ns: AtomicU64,
}

impl Transport {
    pub const fn neuf() -> Self {
        Self {
            etat: AtomicU8::new(0),
            phase: AtomicU8::new(0),
            dernier_incident: AtomicU8::new(0),
            dernier_slot: AtomicU8::new(0),
            dernier_dci: AtomicU8::new(0),
            tentatives: AtomicU8::new(0),
            echeances: AtomicU64::new(0),
            reprises: AtomicU64::new(0),
            reussies: AtomicU64::new(0),
            echouees: AtomicU64::new(0),
            refus: AtomicU64::new(0),
            dernier_incident_ns: AtomicU64::new(0),
        }
    }

    pub fn etat(&self) -> Etat {
        Etat::depuis_code(self.etat.load(Ordering::Acquire))
    }

    /// A-t-on le droit d'emettre ? Compte le refus quand la reponse est non.
    ///
    /// Le refus est COMPTE parce que c'est la seule trace qu'un appelant a ete
    /// econduit : sans lui, un support hors service ressemble a un support
    /// qu'on n'a jamais sollicite.
    pub fn autorise_es(&self) -> bool {
        if self.etat() == Etat::Pret {
            return true;
        }
        self.refus.fetch_add(1, Ordering::Relaxed);
        false
    }

    /// Note la phase en cours, pour que l'incident sache d'ou il vient.
    pub fn entre_en_phase(&self, phase: Phase) {
        self.phase.store(phase.code(), Ordering::Release);
    }

    pub fn sort_de_phase(&self) {
        self.phase.store(Phase::Repos.code(), Ordering::Release);
    }

    /// Un incident : le transport passe en reprise, quoi qu'il arrive.
    ///
    /// Rend l'etat resultant. Il n'y a PAS de cas ou un incident laisse
    /// `Pret` : continuer a emettre sur un transport qui vient d'expirer est
    /// exactement ce qui transformait une commande perdue en pilote perdu.
    pub fn incident(&self, quoi: Incident, slot: u8, dci: u8, maintenant_ns: u64) -> Etat {
        if quoi == Incident::Echeance {
            self.echeances.fetch_add(1, Ordering::Relaxed);
        }
        self.dernier_incident.store(quoi.code(), Ordering::Relaxed);
        self.dernier_slot.store(slot, Ordering::Relaxed);
        self.dernier_dci.store(dci, Ordering::Relaxed);
        self.dernier_incident_ns
            .store(maintenant_ns, Ordering::Relaxed);
        if self.etat() == Etat::HorsService {
            return Etat::HorsService;
        }
        self.etat.store(Etat::Reprise.code(), Ordering::Release);
        Etat::Reprise
    }

    /// Une tentative de reprise commence. Rend `false` si elle ne doit PAS
    /// avoir lieu -- transport deja sain, ou deja hors service.
    pub fn commence_reprise(&self) -> bool {
        match self.etat() {
            Etat::Reprise => {
                self.reprises.fetch_add(1, Ordering::Relaxed);
                self.tentatives.fetch_add(1, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }

    /// La reprise a rendu le materiel coherent.
    pub fn reprise_reussie(&self) {
        self.reussies.fetch_add(1, Ordering::Relaxed);
        self.tentatives.store(0, Ordering::Relaxed);
        self.phase.store(Phase::Repos.code(), Ordering::Release);
        self.etat.store(Etat::Pret.code(), Ordering::Release);
    }

    /// La reprise a echoue. Au-dela du seuil, le transport est hors service.
    pub fn reprise_echouee(&self) -> Etat {
        self.echouees.fetch_add(1, Ordering::Relaxed);
        let tentatives = self.tentatives.load(Ordering::Relaxed);
        if tentatives >= REPRISES_AVANT_HORS_SERVICE {
            self.etat.store(Etat::HorsService.code(), Ordering::Release);
            return Etat::HorsService;
        }
        self.etat.store(Etat::Reprise.code(), Ordering::Release);
        Etat::Reprise
    }

    /// Le support vient d'etre (re)configure : on repart de zero.
    pub fn remet_a_neuf(&self) {
        self.tentatives.store(0, Ordering::Relaxed);
        self.phase.store(Phase::Repos.code(), Ordering::Release);
        self.etat.store(Etat::Pret.code(), Ordering::Release);
    }

    pub fn releve(&self) -> Releve {
        Releve {
            etat: self.etat(),
            derniere_phase: Phase::depuis_code(self.phase.load(Ordering::Acquire)),
            dernier_incident: self.dernier_incident.load(Ordering::Relaxed),
            dernier_slot: self.dernier_slot.load(Ordering::Relaxed),
            dernier_dci: self.dernier_dci.load(Ordering::Relaxed),
            echeances: self.echeances.load(Ordering::Relaxed),
            reprises: self.reprises.load(Ordering::Relaxed),
            reprises_reussies: self.reussies.load(Ordering::Relaxed),
            reprises_echouees: self.echouees.load(Ordering::Relaxed),
            tentatives_en_cours: self.tentatives.load(Ordering::Relaxed),
            refus: self.refus.load(Ordering::Relaxed),
            dernier_incident_ns: self.dernier_incident_ns.load(Ordering::Relaxed),
        }
    }
}
