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
    let (_, secteurs) = ata::capacities();
    if secteurs <= SECTEURS_ZONE + SECTEUR_CONTENU {
        return None;
    }
    Some(secteurs - SECTEURS_ZONE)
}
