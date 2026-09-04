//! ATA derriere la couche bloc generique.
//!
//! ATA reste le SEUL backend de stockage, et il reste le repli. Ce qui change,
//! c'est qu'il n'est plus le seul chemin possible : un pilote NVMe qui
//! implemente `PiloteBloc` s'enregistre sur un volume et devient utilisable
//! sans qu'un appelant du systeme de fichiers change.
//!
//! # Ce qu'ATA declare honnetement
//!
//! `profondeur_file: 1` -- le PIO est synchrone, la soumission EST
//! l'achevement. Declarer plus laisserait croire a un parallelisme qui
//! n'existe pas.
//!
//! `vidange_reelle: false` -- le pilote n'emet pas `FLUSH CACHE`. Un disque
//! avec un cache d'ecriture peut donc avoir accepte une ecriture sans l'avoir
//! posee sur le plateau. Le declarer FAUX est le seul choix honnete : un commit
//! qui croit avoir une barriere qu'il n'a pas est pire qu'un commit qui sait
//! qu'il n'en a pas -- le premier se croit sur, le second peut compenser.

use crate::drivers::ata::{self, Drive, SECTOR_SIZE};
use crate::drivers::bloc::{
    enregistre, Achevement, Descripteur, Genre, PiloteBloc, Requete, Volume,
};

pub struct AtaPilote {
    nappe: Drive,
    nom: &'static str,
}

impl AtaPilote {
    const fn neuf(nappe: Drive, nom: &'static str) -> Self {
        Self { nappe, nom }
    }

    fn secteurs(&self) -> u64 {
        let (maitre, esclave) = ata::capacities();
        match self.nappe {
            Drive::Master => maitre,
            Drive::Slave => esclave,
        }
    }

    /// La requete tient-elle dans le volume ?
    ///
    /// Un LBA hors bornes passe au controleur, qui rend une erreur -- mais
    /// seulement apres un aller-retour materiel. Le refuser ici est plus
    /// rapide, et surtout le compte comme une erreur DE REQUETE et non comme
    /// une panne du disque.
    fn dans_les_bornes(&self, requete: &Requete) -> bool {
        let blocs = self.secteurs();
        if blocs == 0 {
            return false;
        }
        match requete.lba.checked_add(requete.blocs as u64) {
            Some(fin) => fin <= blocs,
            None => false,
        }
    }
}

impl PiloteBloc for AtaPilote {
    fn descripteur(&self) -> Descripteur {
        Descripteur {
            taille_bloc: SECTOR_SIZE,
            blocs: self.secteurs(),
            profondeur_file: 1,
            vidange_reelle: false,
            nom: self.nom,
        }
    }

    fn soumet(&self, requete: Requete, tampon: &mut [u8]) -> Achevement {
        match requete.genre {
            Genre::Lecture => {
                if !self.dans_les_bornes(&requete) {
                    return Achevement::Erreur;
                }
                let lus = ata::read(self.nappe, requete.lba, requete.blocs, tampon);
                if lus == requete.blocs { Achevement::Fait(lus) } else { Achevement::Erreur }
            }
            // Une ecriture par ce chemin n'a pas de donnees a poser : c'est une
            // erreur d'appelant, pas une panne.
            Genre::Ecriture => Achevement::Erreur,
            Genre::Vidange => Achevement::Fait(0),
        }
    }

    fn soumet_ecriture(&self, requete: Requete, donnees: &[u8]) -> Achevement {
        match requete.genre {
            Genre::Ecriture => {
                if !self.dans_les_bornes(&requete) {
                    return Achevement::Erreur;
                }
                let ecrits = ata::write(self.nappe, requete.lba, requete.blocs, donnees);
                if ecrits == requete.blocs { Achevement::Fait(ecrits) } else { Achevement::Erreur }
            }
            // La vidange REUSSIT, et `vidange_reelle: false` dit qu'elle ne
            // garantit rien. Les deux ensemble sont la verite : l'appel ne
            // casse pas, et l'appelant sait qu'il n'a pas de barriere.
            Genre::Vidange => Achevement::Fait(0),
            Genre::Lecture => Achevement::Erreur,
        }
    }
}

static AMORCE: AtaPilote = AtaPilote::neuf(Drive::Master, "ata-maitre");
static DONNEES: AtaPilote = AtaPilote::neuf(Drive::Slave, "ata-esclave");

/// Enregistre ATA sur les deux volumes historiques.
///
/// A appeler une fois au demarrage, apres la detection ATA. Un volume dont le
/// disque est absent s'enregistre quand meme : son descripteur porte alors
/// `blocs: 0`, ce qui est la bonne facon de dire « present dans le registre,
/// absent du materiel ».
pub fn installe() {
    enregistre(Volume::AMORCE, &AMORCE);
    enregistre(Volume::DONNEES, &DONNEES);
    crate::kernel::dmesg::log_fmt(format_args!(
        "bloc-ng: volume 0 = {} ({} secteurs), volume 1 = {} ({} secteurs)",
        AMORCE.nom, AMORCE.secteurs(), DONNEES.nom, DONNEES.secteurs(),
    ));
}
