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

// BOUCHAUD_C5_MIGRATION_V1_SANS_ECRASEMENT_V1
//
// LA PREMIERE ECRITURE V2 DETRUISAIT LA V1 AVANT DE LA REMPLACER
//
// Sur un disque V1, `superbloc_courant` rend `None`, donc `prochain(None)`
// choisissait la demi-zone 0. Elle commence au secteur 2 et recouvre la table
// V1 (1..1024) DES sa premiere ecriture -- alors que l'en-tete V1, au secteur
// 0, est encore intact et dit toujours « zone V1 valide ».
//
// Une coupure a ce moment laissait donc exactement l'etat que ce format existe
// pour interdire : une magie valide qui designe une table dechiquetee. Le repli
// V1 montait des donnees corrompues, et le chantier 5 aurait promis le contraire
// de ce qu'il faisait.
//
// La correction : tant qu'un etat V1 est vivant sur le disque, la premiere
// ecriture V2 va dans la demi-zone qui NE LE RECOUVRE PAS. Le commit -- un seul
// secteur, au secteur 0 ou 1 -- est alors le premier octet a toucher la V1, et
// il la remplace d'un coup.

/// Dernier secteur occupe par un etat V1 encore vivant sur le disque.
///
/// Zero signifie « aucune V1 a preserver » : disque vierge, ou deja migre.
/// Renseigne par le montage, qui lit deja la table V1 pour restaurer les
/// fichiers -- l'etendue lui coute une addition.
static V1_FIN_SECTEUR: AtomicU64 = AtomicU64::new(0);

/// Retient l'etendue d'un etat V1 monte, relative au debut de la zone.
fn note_etendue_v1(fin_relative: u64) {
    V1_FIN_SECTEUR.store(fin_relative, Ordering::Release);
}

/// Oublie l'etat V1 : il vient d'etre remplace par un commit V2.
fn oublie_la_v1() {
    V1_FIN_SECTEUR.store(0, Ordering::Release);
}

/// Une demi-zone recouvre-t-elle l'etat V1 encore vivant ?
fn demi_recouvre_la_v1(demi: u32) -> bool {
    let fin_v1 = V1_FIN_SECTEUR.load(Ordering::Acquire);
    if fin_v1 == 0 {
        return false;
    }
    // Les deux regions sont relatives au debut de la zone. La V1 occupe
    // `0..=fin_v1` ; la demi-zone `debut_demi(demi)..fin_demi(demi)`.
    debut_demi(demi) <= fin_v1
}

/// La demi-zone ou ecrire, en preservant un eventuel etat V1.
///
/// Rend `None` quand les DEUX demi-zones recouvrent la V1 : le contenu V1
/// depasse alors la moitie de la zone, et il ne tiendrait de toute facon pas
/// dans une demi-zone. La synchronisation echouerait plus loin sur le meme
/// motif ; la refuser ici la fait echouer AVANT d'avoir rien detruit, ce qui
/// est la seule difference qui compte.
fn demi_sure(demi_prevue: u32) -> Option<u32> {
    if !demi_recouvre_la_v1(demi_prevue) {
        return Some(demi_prevue);
    }
    let autre = 1 - demi_prevue;
    if !demi_recouvre_la_v1(autre) {
        return Some(autre);
    }
    None
}

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
