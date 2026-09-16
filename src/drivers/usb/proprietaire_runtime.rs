//! Qui tient le pilote xHCI, depuis quand, et combien de temps au pire.
//!
//! # Le defaut que ceci corrige
//!
//! `RUNTIME_BUSY` etait un `AtomicBool` pris et rendu a la main, sur six
//! chemins differents. Trois consequences, toutes constatees :
//!
//!   * un `return` intermediaire entre la prise et le `store(false)` laissait
//!     le controleur tenu pour toujours -- et la machine paraissait alors
//!     avoir perdu son clavier, son stockage et son enregistreur d'un coup ;
//!   * quand la scrutation HID renoncait, rien ne disait CONTRE QUI. « Le
//!     verrou etait pris » n'est pas un diagnostic ; « le systeme de fichiers
//!     l'a tenu 480 ms » en est un ;
//!   * aucune trace ne portait la duree de tenue, donc aucun releve ne
//!     pouvait distinguer une contention normale -- quelques millisecondes --
//!     d'une tenue qui mange la fenetre de scrutation du clavier.
//!
//! # Ce que ce module est, et ce qu'il n'est pas
//!
//! Il n'est PAS le verrou : le verrou reste l'echange compare sur
//! `RUNTIME_BUSY`, dans le pilote, ou il peut voir le materiel. Il en est la
//! COMPTABILITE -- qui, depuis quand, combien de fois, au pire combien de
//! temps. C'est la partie qui se raisonne sans materiel, donc celle qu'une
//! machine hote peut mettre a l'epreuve.
//!
//! Module PUR : ni `use crate::`, ni `unsafe`. L'horloge est un argument.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

/// Qui peut tenir le pilote xHCI.
///
/// L'enumeration est FERMEE, et c'est le point : un septieme consommateur qui
/// apparaitrait devrait se nommer ici, donc se declarer aux releves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Proprietaire {
    Aucun = 0,
    /// Scrutation HID : clavier et souris, mille fois par seconde.
    Hid = 1,
    /// Pont EP0 pour les points HID muets.
    ReplieHid = 2,
    /// Systeme de fichiers : lecture et ecriture de volume.
    SystemeDeFichiers = 3,
    /// Vidage de l'enregistreur de vol vers le support.
    VidageBlackbox = 4,
    /// Enumeration et branchement a chaud.
    Enumeration = 5,
    /// Releve de diagnostic : `lsusb`, parcours des points.
    Diagnostic = 6,
}

impl Proprietaire {
    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn depuis_code(code: u8) -> Self {
        match code {
            1 => Self::Hid,
            2 => Self::ReplieHid,
            3 => Self::SystemeDeFichiers,
            4 => Self::VidageBlackbox,
            5 => Self::Enumeration,
            6 => Self::Diagnostic,
            _ => Self::Aucun,
        }
    }

    pub fn nom(self) -> &'static str {
        match self {
            Self::Aucun => "aucun",
            Self::Hid => "hid",
            Self::ReplieHid => "repli-hid",
            Self::SystemeDeFichiers => "systeme-de-fichiers",
            Self::VidageBlackbox => "vidage-blackbox",
            Self::Enumeration => "enumeration",
            Self::Diagnostic => "diagnostic",
        }
    }
}

/// Ce qu'un releve doit montrer du verrou du pilote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Etat {
    pub proprietaire: Proprietaire,
    /// Depuis combien de temps il tient, zero s'il n'y a personne.
    pub tenue_courante_ns: u64,
    /// La pire tenue de la session, et par qui.
    pub tenue_max_ns: u64,
    pub tenue_max_proprietaire: Proprietaire,
    pub prises: u64,
    /// Prises refusees faute d'avoir trouve le verrou libre.
    pub contentions: u64,
    /// Attentes bornees qui ont expire sans obtenir le verrou.
    pub expirations: u64,
    pub derniere_liberation_ns: u64,
}

impl Etat {
    /// Le verrou est-il revenu a `Aucun` ? C'est le critere d'acceptation C.
    pub fn libre(&self) -> bool {
        self.proprietaire == Proprietaire::Aucun
    }
}

pub struct Registre {
    proprietaire: AtomicU8,
    depuis_ns: AtomicU64,
    tenue_max_ns: AtomicU64,
    tenue_max_proprietaire: AtomicU8,
    prises: AtomicU64,
    contentions: AtomicU64,
    expirations: AtomicU64,
    derniere_liberation_ns: AtomicU64,
}

impl Registre {
    pub const fn neuf() -> Self {
        Self {
            proprietaire: AtomicU8::new(0),
            depuis_ns: AtomicU64::new(0),
            tenue_max_ns: AtomicU64::new(0),
            tenue_max_proprietaire: AtomicU8::new(0),
            prises: AtomicU64::new(0),
            contentions: AtomicU64::new(0),
            expirations: AtomicU64::new(0),
            derniere_liberation_ns: AtomicU64::new(0),
        }
    }

    pub fn note_prise(&self, qui: Proprietaire, maintenant_ns: u64) {
        self.depuis_ns.store(maintenant_ns, Ordering::Relaxed);
        self.proprietaire.store(qui.code(), Ordering::Release);
        self.prises.fetch_add(1, Ordering::Relaxed);
    }

    /// Rend la tenue mesuree, en nanosecondes.
    pub fn note_liberation(&self, maintenant_ns: u64) -> u64 {
        let qui = self.proprietaire.swap(0, Ordering::AcqRel);
        let depuis = self.depuis_ns.load(Ordering::Relaxed);
        let tenue = maintenant_ns.saturating_sub(depuis);
        // LE PIRE ET SON AUTEUR VOYAGENT ENSEMBLE.
        //
        // « La pire tenue fait 480 ms » ne se corrige pas ; « la pire tenue
        // fait 480 ms et c'est le systeme de fichiers » se corrige. Poser les
        // deux separement les laisserait diverger a la premiere course ; le
        // `compare_exchange` en fait une seule decision.
        let mut pire = self.tenue_max_ns.load(Ordering::Relaxed);
        while tenue > pire {
            match self.tenue_max_ns.compare_exchange(
                pire,
                tenue,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.tenue_max_proprietaire.store(qui, Ordering::Release);
                    break;
                }
                Err(vu) => pire = vu,
            }
        }
        self.derniere_liberation_ns
            .store(maintenant_ns, Ordering::Release);
        tenue
    }

    pub fn note_contention(&self) {
        self.contentions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_expiration(&self) {
        self.expirations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn etat(&self, maintenant_ns: u64) -> Etat {
        let code = self.proprietaire.load(Ordering::Acquire);
        let proprietaire = Proprietaire::depuis_code(code);
        let tenue_courante_ns = if proprietaire == Proprietaire::Aucun {
            0
        } else {
            maintenant_ns.saturating_sub(self.depuis_ns.load(Ordering::Relaxed))
        };
        Etat {
            proprietaire,
            tenue_courante_ns,
            tenue_max_ns: self.tenue_max_ns.load(Ordering::Relaxed),
            tenue_max_proprietaire: Proprietaire::depuis_code(
                self.tenue_max_proprietaire.load(Ordering::Acquire),
            ),
            prises: self.prises.load(Ordering::Relaxed),
            contentions: self.contentions.load(Ordering::Relaxed),
            expirations: self.expirations.load(Ordering::Relaxed),
            derniere_liberation_ns: self.derniere_liberation_ns.load(Ordering::Acquire),
        }
    }

    /// Le verrou est-il tenu depuis plus longtemps qu'aucun consommateur ne
    /// devrait le tenir ?
    ///
    /// Le seuil est un ARGUMENT : la reponse depend du chemin, pas du module.
    pub fn detenu_trop_longtemps(&self, maintenant_ns: u64, seuil_ns: u64) -> bool {
        let etat = self.etat(maintenant_ns);
        !etat.libre() && etat.tenue_courante_ns >= seuil_ns
    }
}
