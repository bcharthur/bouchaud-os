//! La couche bloc generique : ce que le systeme de fichiers voit du stockage.
//!
//! # Ce qui existait, et pourquoi cela ne suffisait pas
//!
//! `api::block` definit deja un trait `BlockDevice`. Il est correct, et il est
//! inutilisable pour ajouter NVMe : ses fonctions libres prennent un
//! `ata::Drive`, et le systeme de fichiers les appelle directement
//! (`ata::read(Drive::Slave, ...)`). Le trait existe ; personne ne passe par
//! lui. Ajouter un second pilote demanderait donc de reecrire les appelants,
//! c'est-a-dire exactement ce que le trait devait eviter.
//!
//! # Ce que cette couche etablit
//!
//! Un VOLUME est un numero. Le systeme de fichiers parle a un volume, pas a une
//! nappe. Un pilote s'enregistre, et le volume devient utilisable sans qu'un
//! seul appelant change. C'est la condition pour que NVMe s'ajoute sans
//! toucher au systeme de fichiers -- pas un souhait d'architecture, une
//! propriete verifiable : `verifie-couche-bloc.py` echoue si un appelant
//! reprend une nappe en dur.
//!
//! # Soumission et achevement, meme quand le pilote est synchrone
//!
//! ATA en PIO est synchrone : la soumission EST l'achevement. NVMe ne l'est
//! pas -- il a des files, et une completion arrive plus tard. Ecrire l'API
//! comme si tout etait synchrone obligerait a la reecrire le jour ou elle ne
//! l'est plus.
//!
//! La forme est donc celle d'une soumission qui rend un ACHEVEMENT, et d'un
//! pilote qui declare sa profondeur de file. ATA declare une profondeur de un
//! et acheve immediatement ; le contrat est deja le bon, et le jour ou un
//! pilote rend `EnCours`, les appelants ont deja la forme pour l'attendre.
//!
//! # La vidange n'est pas une ecriture
//!
//! `Vidange` (flush/barrier) est un genre de requete a part, et pas une option
//! d'ecriture. Une barriere qui serait un drapeau sur une ecriture ne pourrait
//! pas etre demandee seule -- or c'est exactement ce qu'un commit fait : ecrire,
//! puis EXIGER que ce qui precede soit sur le plateau avant d'ecrire le
//! superbloc. Sans ce point, le commit A/B de la persistance ne repose que sur
//! l'ordre d'emission, ce qui ne vaut rien face a un cache d'ecriture.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::SpinLockIrq;

/// Volumes suivis simultanement.
pub const VOLUMES_MAX: usize = 8;

/// Taille de bloc supposee par les appelants historiques.
pub const TAILLE_BLOC: usize = 512;

/// Un volume : le seul identifiant que le systeme de fichiers manipule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Volume(pub u8);

impl Volume {
    /// Le disque de demarrage, porteur de l'archive.
    pub const AMORCE: Volume = Volume(0);
    /// Le disque de donnees, porteur de la zone persistante.
    pub const DONNEES: Volume = Volume(1);

    #[inline]
    pub const fn indice(self) -> usize { self.0 as usize }
}

/// Ce qu'une requete demande.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Genre {
    Lecture,
    Ecriture,
    /// Exiger que tout ce qui precede soit durable. Sans charge utile.
    Vidange,
}

/// Une requete d'entree-sortie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requete {
    pub genre: Genre,
    pub lba: u64,
    pub blocs: usize,
}

impl Requete {
    pub const fn lecture(lba: u64, blocs: usize) -> Self {
        Self { genre: Genre::Lecture, lba, blocs }
    }
    pub const fn ecriture(lba: u64, blocs: usize) -> Self {
        Self { genre: Genre::Ecriture, lba, blocs }
    }
    pub const fn vidange() -> Self {
        Self { genre: Genre::Vidange, lba: 0, blocs: 0 }
    }
}

/// Ce qu'une requete rend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Achevement {
    /// Termine : autant de blocs transferes.
    Fait(usize),
    /// Soumise, pas encore terminee. Aucun pilote ne le rend aujourd'hui ; la
    /// forme existe pour que NVMe n'oblige pas a reecrire les appelants.
    EnCours,
    /// Le volume ne repond pas, ou la requete est hors bornes.
    Erreur,
    /// Aucun pilote n'est enregistre pour ce volume.
    Absent,
}

impl Achevement {
    #[inline]
    pub const fn blocs(self) -> usize {
        match self { Self::Fait(n) => n, _ => 0 }
    }
    #[inline]
    pub const fn reussi(self) -> bool {
        matches!(self, Self::Fait(_))
    }
}

/// Ce qu'un pilote declare de lui-meme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descripteur {
    pub taille_bloc: usize,
    pub blocs: u64,
    /// Requetes que le pilote accepte en vol. Un pilote synchrone declare 1.
    pub profondeur_file: usize,
    /// Le pilote sait-il vraiment vider son cache ?
    ///
    /// Le declarer faux quand on ne sait pas est le seul choix honnete : un
    /// commit qui croit avoir une barriere qu'il n'a pas est pire qu'un commit
    /// qui sait qu'il n'en a pas.
    pub vidange_reelle: bool,
    pub nom: &'static str,
}

impl Descripteur {
    pub const fn absent() -> Self {
        Self {
            taille_bloc: TAILLE_BLOC,
            blocs: 0,
            profondeur_file: 0,
            vidange_reelle: false,
            nom: "absent",
        }
    }
}

/// Ce qu'un pilote de stockage doit savoir faire.
pub trait PiloteBloc: Sync + Send {
    fn descripteur(&self) -> Descripteur;

    /// Soumet une requete. `tampon` porte les donnees a lire ou a ecrire ; il
    /// est ignore pour une vidange.
    fn soumet(&self, requete: Requete, tampon: &mut [u8]) -> Achevement;

    /// Variante en lecture seule du tampon, pour les ecritures.
    fn soumet_ecriture(&self, requete: Requete, donnees: &[u8]) -> Achevement;
}

// --- Le registre -------------------------------------------------------------

struct Emplacement {
    soumissions: AtomicU64,
    achevements: AtomicU64,
    erreurs: AtomicU64,
    vidanges: AtomicU64,
    blocs_lus: AtomicU64,
    blocs_ecrits: AtomicU64,
}

impl Emplacement {
    const fn vide() -> Self {
        Self {
            soumissions: AtomicU64::new(0),
            achevements: AtomicU64::new(0),
            erreurs: AtomicU64::new(0),
            vidanges: AtomicU64::new(0),
            blocs_lus: AtomicU64::new(0),
            blocs_ecrits: AtomicU64::new(0),
        }
    }
}

static REGISTRE: [Emplacement; VOLUMES_MAX] = [const { Emplacement::vide() }; VOLUMES_MAX];

/// Les pilotes enregistres.
///
/// Un `&dyn` est un pointeur GRAS : deux mots, qu'aucun atomique ne porte. Le
/// registre est donc un verrou tournant par volume, pris uniquement pour
/// RECOPIER la reference -- une poignee d'instructions --, jamais pendant
/// l'entree-sortie elle-meme. Un volume s'enregistre au demarrage et ne change
/// plus ; la contention est nulle en pratique, et le cout est celui d'un
/// verrou non dispute face a une operation qui dure des millisecondes.
static PILOTES: [SpinLockIrq<Option<&'static dyn PiloteBloc>>; VOLUMES_MAX] =
    [const { SpinLockIrq::new(None) }; VOLUMES_MAX];

/// Enregistre un pilote pour un volume.
///
/// La reference est `'static` : un pilote qui disparaitrait laisserait le
/// registre pointer dans le vide, et le systeme de fichiers ne verifie pas.
pub fn enregistre(volume: Volume, pilote: &'static dyn PiloteBloc) -> bool {
    if volume.indice() >= VOLUMES_MAX {
        return false;
    }
    *PILOTES[volume.indice()].lock() = Some(pilote);
    true
}

fn pilote(volume: Volume) -> Option<&'static dyn PiloteBloc> {
    if volume.indice() >= VOLUMES_MAX {
        return None;
    }
    *PILOTES[volume.indice()].lock()
}

/// Le descripteur d'un volume, ou `Descripteur::absent()`.
pub fn descripteur(volume: Volume) -> Descripteur {
    pilote(volume).map(|p| p.descripteur()).unwrap_or(Descripteur::absent())
}

/// Un volume est-il present ?
pub fn present(volume: Volume) -> bool {
    descripteur(volume).blocs != 0
}

/// Lit des blocs.
pub fn lit(volume: Volume, lba: u64, blocs: usize, out: &mut [u8]) -> Achevement {
    let Some(p) = pilote(volume) else { return Achevement::Absent };
    note_soumission(volume);
    let resultat = p.soumet(Requete::lecture(lba, blocs), out);
    note_achevement(volume, resultat, Genre::Lecture);
    resultat
}

/// Ecrit des blocs.
pub fn ecrit(volume: Volume, lba: u64, blocs: usize, donnees: &[u8]) -> Achevement {
    let Some(p) = pilote(volume) else { return Achevement::Absent };
    note_soumission(volume);
    let resultat = p.soumet_ecriture(Requete::ecriture(lba, blocs), donnees);
    note_achevement(volume, resultat, Genre::Ecriture);
    resultat
}

/// Exige que tout ce qui precede soit durable.
///
/// Rend `false` quand le pilote ne sait pas vraiment vider son cache : un
/// appelant qui commit doit pouvoir SAVOIR qu'il n'a pas de barriere, plutot
/// que de croire en avoir une.
pub fn vidange(volume: Volume) -> bool {
    let Some(p) = pilote(volume) else { return false };
    note_soumission(volume);
    let resultat = p.soumet_ecriture(Requete::vidange(), &[]);
    note_achevement(volume, resultat, Genre::Vidange);
    resultat.reussi() && p.descripteur().vidange_reelle
}

fn note_soumission(volume: Volume) {
    REGISTRE[volume.indice()].soumissions.fetch_add(1, Ordering::Relaxed);
}

fn note_achevement(volume: Volume, achevement: Achevement, genre: Genre) {
    let e = &REGISTRE[volume.indice()];
    match achevement {
        Achevement::Fait(blocs) => {
            e.achevements.fetch_add(1, Ordering::Relaxed);
            match genre {
                Genre::Lecture => { e.blocs_lus.fetch_add(blocs as u64, Ordering::Relaxed); }
                Genre::Ecriture => { e.blocs_ecrits.fetch_add(blocs as u64, Ordering::Relaxed); }
                Genre::Vidange => { e.vidanges.fetch_add(1, Ordering::Relaxed); }
            }
        }
        Achevement::EnCours => {}
        _ => { e.erreurs.fetch_add(1, Ordering::Relaxed); }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compteurs {
    pub soumissions: u64,
    pub achevements: u64,
    pub erreurs: u64,
    pub vidanges: u64,
    pub blocs_lus: u64,
    pub blocs_ecrits: u64,
}

pub fn compteurs(volume: Volume) -> Compteurs {
    if volume.indice() >= VOLUMES_MAX {
        return Compteurs::default();
    }
    let e = &REGISTRE[volume.indice()];
    Compteurs {
        soumissions: e.soumissions.load(Ordering::Relaxed),
        achevements: e.achevements.load(Ordering::Relaxed),
        erreurs: e.erreurs.load(Ordering::Relaxed),
        vidanges: e.vidanges.load(Ordering::Relaxed),
        blocs_lus: e.blocs_lus.load(Ordering::Relaxed),
        blocs_ecrits: e.blocs_ecrits.load(Ordering::Relaxed),
    }
}

pub fn log_stats() {
    for indice in 0..VOLUMES_MAX {
        let volume = Volume(indice as u8);
        let d = descripteur(volume);
        if d.blocs == 0 {
            continue;
        }
        let c = compteurs(volume);
        crate::serial_println!(
            "[BLOC-NG] volume={} pilote={} blocs={} file={} vidange_reelle={} soumissions={} achevements={} erreurs={} lus={} ecrits={} vidanges={}",
            indice, d.nom, d.blocs, d.profondeur_file, d.vidange_reelle,
            c.soumissions, c.achevements, c.erreurs, c.blocs_lus, c.blocs_ecrits,
            c.vidanges
        );
    }
}
