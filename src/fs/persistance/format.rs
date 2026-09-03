/// Nombre de secteurs reellement necessaires pour `nombre` entrees.
///
/// Deux entrees de 256 octets tiennent dans un secteur de 512 octets. Cette
/// fonction ne change PAS le format : le contenu commence toujours au secteur
/// 1025. Elle ne fait que supprimer les centaines de secteurs nuls qui etaient
/// relus et reecrits a chaque `fsync`.
#[inline]
fn secteurs_table_utiles(nombre: usize) -> usize {
    let octets = nombre.saturating_mul(TAILLE_ENTREE);
    (octets + SECTOR_SIZE - 1) / SECTOR_SIZE
}

/// Premier secteur de la zone sur le disque de donnees, ou `None`.
///
/// La zone occupe la fin du disque. Un disque trop petit pour la porter n'en a
/// pas : mieux vaut aucune persistance qu'une persistance qui ecrase l'archive.
///
/// Le seuil porte sur la zone **augmentee de son en-tete et de sa table** : une
/// image dont l'archive tient dans moins de `SECTEUR_CONTENU` secteurs n'aurait
/// pas de quoi ecrire une table complete. `mkdisk.sh` complete donc la region
/// d'archive jusqu'a ce plancher avant d'ajouter la zone.
fn debut() -> Option<u64> {
    // La capacite vient du VOLUME, pas de la nappe. C'est le premier appelant
    // du systeme de fichiers a passer par la couche bloc generique : le jour ou
    // le volume 1 sera servi par NVMe, cette fonction ne changera pas.
    //
    // Les lectures et ecritures elles-memes passent encore par `ata::` : les
    // migrer demande de traiter les chemins d'erreur un par un, et ce commit
    // n'en fait qu'un a la fois.
    let secteurs = crate::drivers::bloc::descripteur(
        crate::drivers::bloc::Volume::DONNEES).blocs;
    if secteurs <= SECTEURS_ZONE + SECTEUR_CONTENU {
        return None;
    }
    Some(secteurs - SECTEURS_ZONE)
}


// --- Format V2 : deux demi-zones -------------------------------------------

/// Superblocs, au tout debut de la zone.
const SECTEURS_SUPERBLOCS: u64 = 2;

/// Secteurs d'une demi-zone.
///
/// La capacite persistante par demi-zone est la moitie de l'ancienne. C'est le
/// prix du commit atomique, et il est explicite : sans deux moitiees, une
/// synchronisation ecrase l'etat dont on depend encore.
const SECTEURS_DEMI: u64 = (SECTEURS_ZONE - SECTEURS_SUPERBLOCS) / 2;

/// Premier secteur d'une demi-zone, relatif au debut de la zone.
#[inline]
const fn debut_demi(demi: u32) -> u64 {
    SECTEURS_SUPERBLOCS + (demi as u64) * SECTEURS_DEMI
}

/// Premier secteur de contenu d'une demi-zone.
#[inline]
const fn contenu_demi(demi: u32) -> u64 {
    debut_demi(demi) + SECTEURS_TABLE
}

/// Dernier secteur utilisable d'une demi-zone.
#[inline]
const fn fin_demi(demi: u32) -> u64 {
    debut_demi(demi) + SECTEURS_DEMI
}

// Une demi-zone doit pouvoir porter sa table ET du contenu.
const _: () = assert!(SECTEURS_DEMI > SECTEURS_TABLE + 16);

/// Lit les deux superblocs de la zone et rend celui a monter.
///
/// Les erreurs de lecture se lisent comme « pas de superbloc a cet
/// emplacement » : un secteur illisible et un secteur dechire demandent la meme
/// chose -- se replier sur l'autre.
fn superbloc_courant(base: u64) -> Option<(usize, Superbloc)> {
    let mut secteur = vec![0u8; SECTOR_SIZE];
    let mut lus = [None, None];
    for emplacement in 0..2usize {
        if ata::read(Drive::Slave, base + emplacement as u64, 1, &mut secteur) == 1 {
            lus[emplacement] = Superbloc::decode(&secteur);
        }
    }
    choisit(lus[0], lus[1])
}
