// L'acces disque de la persistance, par la couche bloc et non par la nappe.
//
// # Ce qui restait a faire
//
// `format::debut()` prenait deja sa capacite du VOLUME, et disait a cote que
// les lectures et ecritures, elles, passaient encore par `ata::`. C'etait
// honnete et c'etait un demi-chemin : tant que l'entree-sortie designe une
// nappe ATA en dur, la persistance ne peut vivre QUE sur un disque ATA -- donc
// pas sur le NVMe d'une machine installee, ou il n'y a pas de nappe du tout.
//
// # La barriere, qui n'existait pas
//
// Le commit A/B repose sur un ORDRE : contenu, puis table, puis superbloc. Cet
// ordre est le bon, et il ne vaut rien face a un cache d'ecriture -- un disque
// a le droit de rendre la main avant que les octets soient sur le plateau, et
// de les ecrire dans l'ordre qui l'arrange. Une coupure peut alors laisser le
// SUPERBLOC sur le plateau et le contenu qu'il designe encore en cache : au
// redemarrage, un etat coherent en apparence designe des secteurs qui n'ont
// jamais ete ecrits.
//
// `api::bloc` a une vidange depuis le debut, et personne ne l'appelait. Elle
// est posee ici entre la table et le superbloc, la ou l'ordre doit devenir une
// garantie.
//
// Et elle est HONNETE : `bloc::vidange` rend `false` quand le pilote ne sait
// pas vraiment vider son cache. On ne transforme donc pas un `false` en
// echec -- un disque sans barriere reste utilisable, avec la garantie plus
// faible qu'il avait deja --, mais on le COMPTE, pour qu'un systeme qui croit
// avoir une barriere qu'il n'a pas puisse le decouvrir autrement qu'apres une
// coupure de courant.

use crate::drivers::bloc::{self as couche_bloc, Volume};

/// Le volume que la persistance occupe.
pub const VOLUME_PERSISTANT: Volume = Volume::DONNEES;

/// Taille de bloc supposee par le format de la zone.
pub const SECTOR_SIZE: usize = 512;

static BARRIERES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static BARRIERES_ABSENTES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Capacite du volume, en secteurs.
pub fn volume_secteurs() -> u64 {
    let d = couche_bloc::descripteur(VOLUME_PERSISTANT);
    // Un volume dont le bloc ne fait pas 512 octets ne porte pas ce format :
    // toutes ses adresses seraient decalees d'un facteur. Le declarer vide est
    // la seule sortie sure -- `debut()` rend alors `None`, et le systeme
    // demarre sans persistance plutot qu'avec une persistance qui ecrit a cote.
    if d.taille_bloc != SECTOR_SIZE {
        return 0;
    }
    d.blocs
}

/// Lit `n` secteurs. Rend le nombre de secteurs lus.
pub fn volume_lit(lba: u64, n: usize, sortie: &mut [u8]) -> usize {
    couche_bloc::lit(VOLUME_PERSISTANT, lba, n, sortie).blocs()
}

/// Ecrit `n` secteurs. Rend le nombre de secteurs ecrits.
pub fn volume_ecrit(lba: u64, n: usize, donnees: &[u8]) -> usize {
    couche_bloc::ecrit(VOLUME_PERSISTANT, lba, n, donnees).blocs()
}

/// Exige que tout ce qui precede soit durable.
///
/// Rend `true` quand la barriere est REELLE. Un `false` n'est pas une erreur :
/// c'est l'aveu que le disque n'en offre pas, et l'appelant continue avec la
/// garantie d'ordre seule -- celle qu'il avait avant. Ce qui change, c'est
/// qu'il le SAIT.
pub fn volume_barriere() -> bool {
    if couche_bloc::vidange(VOLUME_PERSISTANT) {
        BARRIERES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        true
    } else {
        BARRIERES_ABSENTES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        false
    }
}

/// Barrieres reelles, puis barrieres demandees sans etre obtenues.
pub fn volume_statistiques() -> (u64, u64) {
    (
        BARRIERES.load(core::sync::atomic::Ordering::Relaxed),
        BARRIERES_ABSENTES.load(core::sync::atomic::Ordering::Relaxed),
    )
}
