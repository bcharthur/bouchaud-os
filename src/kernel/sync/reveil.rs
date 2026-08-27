//! Reveil evenementiel : dormir jusqu'a ce qu'il y ait REELLEMENT du travail.
//!
//! # Le probleme
//!
//! Un producteur (IRQ clavier, paquet souris, ecriture d'un client) veut dire
//! « il y a du travail ». Un consommateur (le compositeur) veut dormir tant
//! qu'il n'y en a pas. Entre les deux se cache la course qui casse tous les
//! compositeurs event-driven :
//!
//! ```text
//!     consommateur                    producteur
//!     ------------                    ----------
//!     constate : rien a faire
//!                                     depose du travail
//!                                     reveille les dormeurs -> personne
//!     s'endort
//!     ... plus jamais reveille
//! ```
//!
//! # Le contrat
//!
//! Le consommateur prend un [`Billet`] **avant** d'examiner son etat. Le billet
//! echantillonne la generation. S'endormir avec un billet perime ne dort pas :
//! [`Reveil::attends`] le constate et rend la main immediatement.
//!
//! ```text
//!     consommateur                    producteur
//!     ------------                    ----------
//!     billet = generation             generation += 1
//!     constate : rien a faire         reveille les dormeurs
//!     attends(billet) -> la generation a bouge, on ne dort pas
//! ```
//!
//! C'est exactement le protocole de [`WaitQueue`], dont l'absence de reveil
//! perdu est deja demontree par l'ordre total de ses quatre acces `SeqCst`. Ce
//! module n'y ajoute que la comptabilite par SOURCE -- sans elle, « le
//! compositeur s'est reveille 4000 fois » ne dit pas s'il faut regarder du cote
//! de la souris, d'un client bavard ou d'une echeance mal choisie.
//!
//! # Ce que ce module n'est pas
//!
//! Ce n'est pas un `sleep` plus long. Un `sleep(20ms)` a la place d'un
//! `sleep(4ms)` reste du polling : il se reveille pour constater qu'il n'y a
//! rien. Ici, sans evenement et sans echeance, le consommateur ne se reveille
//! pas du tout.

use core::sync::atomic::{AtomicU64, Ordering};

use super::wait_queue::{WaitQueue, WaitTicket};

/// Qui a demande le reveil.
///
/// Ces valeurs sont des INDICES de tableau : leur ordre est celui de
/// [`Reveil::compteurs`] et de la ligne de journal. Ne pas les reordonner sans
/// mettre l'un et l'autre a jour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Source {
    Clavier = 0,
    Souris = 1,
    Client = 2,
    Fenetre = 3,
    Explicite = 4,
}

pub const NOMBRE_SOURCES: usize = 5;

/// Noms des sources, dans l'ordre de l'enumeration.
pub const NOMS_SOURCES: [&str; NOMBRE_SOURCES] =
    ["clavier", "souris", "client", "fenetre", "explicite"];

/// Pourquoi [`Reveil::attends`] a rendu la main.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fin {
    /// Un producteur avait deja signale entre le billet et l'appel : on n'a pas
    /// dormi du tout. C'est le cas qui prouve l'absence de reveil perdu.
    DejaSignale,
    /// Reveille par un producteur pendant le sommeil.
    Signale,
    /// L'echeance est arrivee sans qu'aucun producteur ne signale.
    Echeance,
}

/// Etat echantillonne avant d'examiner son travail.
///
/// A prendre AVANT toute lecture d'etat, jamais apres : c'est tout l'interet.
#[derive(Clone, Copy)]
pub struct Billet {
    ticket: WaitTicket,
    generation: u64,
}

pub struct Reveil {
    file: WaitQueue,
    /// Generation globale : incrementee par chaque signal, quelle que soit la
    /// source. C'est elle qui porte la correction ; les compteurs par source ne
    /// servent qu'au diagnostic.
    generation: AtomicU64,
    compteurs: [AtomicU64; NOMBRE_SOURCES],
    /// Sommeils reellement entames (billet encore valide au moment de dormir).
    sommeils: AtomicU64,
    /// Sommeils evites parce que le billet etait deja perime. Un nombre eleve
    /// n'est pas une panne : c'est le protocole qui fait son travail.
    sommeils_evites: AtomicU64,
    /// Reveils par un producteur.
    reveils_signal: AtomicU64,
    /// Reveils par echeance -- le seul polling qui reste, et il est nomme.
    reveils_echeance: AtomicU64,
}

impl Reveil {
    pub const fn new() -> Self {
        Self {
            file: WaitQueue::new(),
            generation: AtomicU64::new(0),
            compteurs: [const { AtomicU64::new(0) }; NOMBRE_SOURCES],
            sommeils: AtomicU64::new(0),
            sommeils_evites: AtomicU64::new(0),
            reveils_signal: AtomicU64::new(0),
            reveils_echeance: AtomicU64::new(0),
        }
    }

    /// Signale du travail. Appelable depuis un handler d'interruption.
    ///
    /// Quand personne ne dort -- le cas courant, le consommateur etant en train
    /// de travailler -- `wake_all` sort sur une seule lecture atomique sans
    /// toucher au gros verrou. Le cout d'un signal est alors deux `fetch_add`.
    #[inline]
    pub fn signale(&self, source: Source) {
        self.compteurs[source as usize].fetch_add(1, Ordering::Relaxed);
        // La generation AVANT le reveil : un dormeur qui relit apres avoir pose
        // son inscription doit voir la nouvelle valeur. `wake_all` incremente
        // sa propre generation et lit les inscriptions en `SeqCst`, ce qui
        // ferme la fenetre ; celle-ci n'est la que pour `attends`.
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.file.wake_all();
    }

    /// Echantillonne l'etat avant d'examiner son travail.
    #[inline]
    pub fn billet(&self) -> Billet {
        Billet {
            ticket: self.file.ticket(),
            generation: self.generation.load(Ordering::SeqCst),
        }
    }

    /// Dort jusqu'a un signal ou jusqu'a `echeance_ns`.
    ///
    /// Rend la main immediatement si un signal est arrive depuis `billet` :
    /// c'est le point ou le reveil perdu est impossible.
    ///
    /// `echeance_ns` est une date absolue sur l'horloge monotone. Passer une
    /// echeance deja echue equivaut a ne pas dormir.
    pub fn attends(&self, billet: Billet, echeance_ns: u64) -> Fin {
        if self.generation.load(Ordering::SeqCst) != billet.generation {
            self.sommeils_evites.fetch_add(1, Ordering::Relaxed);
            return Fin::DejaSignale;
        }
        self.sommeils.fetch_add(1, Ordering::Relaxed);
        if self.file.wait_until(billet.ticket, echeance_ns) {
            self.reveils_signal.fetch_add(1, Ordering::Relaxed);
            Fin::Signale
        } else {
            self.reveils_echeance.fetch_add(1, Ordering::Relaxed);
            Fin::Echeance
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn invalidations(&self, source: Source) -> u64 {
        self.compteurs[source as usize].load(Ordering::Relaxed)
    }

    pub fn invalidations_totales(&self) -> u64 {
        self.compteurs.iter().map(|c| c.load(Ordering::Relaxed)).sum()
    }

    /// `(sommeils, sommeils_evites, reveils_signal, reveils_echeance)`.
    pub fn statistiques(&self) -> (u64, u64, u64, u64) {
        (
            self.sommeils.load(Ordering::Relaxed),
            self.sommeils_evites.load(Ordering::Relaxed),
            self.reveils_signal.load(Ordering::Relaxed),
            self.reveils_echeance.load(Ordering::Relaxed),
        )
    }
}

/// Le reveil de l'interface graphique.
///
/// Il vit dans le noyau et non dans `gui/` pour que les producteurs restent
/// propres : un pilote PS/2 ou la couche descripteurs n'ont aucune raison de
/// connaitre le compositeur. Tout le monde parle au noyau ; le noyau ne parle
/// a personne.
pub static INTERFACE: Reveil = Reveil::new();

/// Signale du travail a l'interface graphique.
///
/// Point d'entree unique des producteurs. Ne rien faire quand l'interface n'est
/// pas active serait une optimisation trompeuse : le compteur doit refleter ce
/// qui est arrive, pas ce qui a servi.
#[inline]
pub fn signale_interface(source: Source) {
    INTERFACE.signale(source);
}
