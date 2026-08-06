//! Une zone inscriptible sur le disque de donnees.
//!
//! Le RAMFS oublie tout a l'extinction. C'est sans consequence pour un binaire
//! deplie depuis l'archive — il y sera encore au prochain demarrage — mais cela
//! rend impossible tout ce qu'un systeme est cense retenir : les temoins de
//! connexion d'un navigateur, son cache, ses reglages, un fichier telecharge.
//!
//! ## Ou l'ecriture a lieu
//!
//! Le disque de donnees porte une archive `tar` **au debut**, lue une fois au
//! demarrage. La zone persistante occupe les derniers secteurs du meme disque :
//! les deux ne se rencontrent jamais tant que l'image est plus grande que
//! l'archive, ce dont `mkdisk.sh` se charge en la completant.
//!
//! Se passer d'un troisieme disque n'est pas qu'une economie : le noyau ne
//! choisit pas combien de disques QEMU lui presente, et une image de plus
//! serait un fichier de plus a oublier d'attacher.
//!
//! ## Le format
//!
//! ```text
//! secteur 0            en-tete : magie, version, nombre d'entrees
//! secteurs 1 a 128     table : 256 entrees de 256 octets (chemin, taille)
//! secteurs 129 a ...   contenu, chaque fichier aligne sur un secteur
//! ```
//!
//! Aucune allocation de blocs, aucune table d'inodes : la zone est reecrite en
//! entier a chaque `sync`. C'est ce qui convient a quelques mega-octets ecrits
//! rarement, et cela supprime la moitie des facons de corrompre un systeme de
//! fichiers. Un vrai systeme de fichiers viendra quand le besoin sera d'ecrire
//! souvent et beaucoup — ce n'est pas celui du navigateur.
//!
//! ## Ce qui est garanti, et ce qui ne l'est pas
//!
//! Une zone dont la magie ne correspond pas est traitee comme vide : un disque
//! neuf, ou un disque dont l'archive a grandi jusqu'a mordre sur la zone, ne
//! font pas echouer le demarrage. En revanche l'ecriture n'est pas atomique :
//! une coupure de courant au milieu d'un `sync` laisse la zone incoherente, et
//! le prochain demarrage la trouvera vide plutot que corrompue seulement si
//! l'en-tete n'a pas encore ete ecrit — c'est pourquoi il est ecrit **en
//! dernier**.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::drivers::ata::{self, Drive, SECTOR_SIZE};
use crate::fs::ramfs::{fs, NodeKind};

/// Reconnait une zone deja formatee.
const MAGIE: &[u8; 8] = b"BOPERSI1";

/// Nombre maximal de fichiers retenus.
const ENTREES_MAX: usize = 256;

/// Taille d'une entree de table, en octets.
const TAILLE_ENTREE: usize = 256;

/// Longueur maximale d'un chemin dans la table.
const CHEMIN_MAX: usize = TAILLE_ENTREE - 16;

/// Secteurs occupes par la table.
const SECTEURS_TABLE: u64 = (ENTREES_MAX * TAILLE_ENTREE / SECTOR_SIZE) as u64;

/// Premier secteur du contenu, relatif au debut de la zone.
const SECTEUR_CONTENU: u64 = 1 + SECTEURS_TABLE;

/// Taille de la zone, en secteurs. 8 Mio.
const SECTEURS_ZONE: u64 = 16384;

/// Racine des fichiers persistants dans le RAMFS.
pub const RACINE: &str = "/persist";

/// Premier secteur de la zone sur le disque de donnees, ou `None`.
///
/// La zone occupe la fin du disque. Un disque trop petit pour la porter n'en a
/// pas : mieux vaut aucune persistance qu'une persistance qui ecrase l'archive.
fn debut() -> Option<u64> {
    let (_, secteurs) = ata::capacities();
    if secteurs <= SECTEURS_ZONE + SECTEUR_CONTENU {
        return None;
    }
    Some(secteurs - SECTEURS_ZONE)
}

/// Ce nœud est-il sous [`RACINE`] ?
///
/// C'est ce qui permet a `fsync` de n'ecrire sur le disque que lorsque le
/// descripteur en cause designe vraiment un fichier persistant : les programmes
/// appellent `fsync` sans compter, et chacun coute sinon une reecriture de toute
/// la zone.
pub fn sous_racine(mut noeud: usize) -> bool {
    let systeme = fs();
    let racine = match systeme.resolve(RACINE, 0) {
        Some(idx) => idx,
        None => return false,
    };
    // La remontee est bornee par le nombre de nœuds : un cycle dans les parents
    // ne doit pas faire boucler le noyau.
    for _ in 0..systeme.nodes.len() {
        if noeud == racine {
            return true;
        }
        let parent = systeme.nodes[noeud].parent;
        if parent == noeud {
            return false;
        }
        noeud = parent;
    }
    false
}


/// Un fichier retenu : son chemin sous [`RACINE`] et son contenu.
struct Entree {
    chemin: String,
    contenu: Vec<u8>,
}

/// Deplie la zone persistante dans `/persist`. A appeler une fois au demarrage.
///
/// Rend le nombre de fichiers restaures.
pub fn monte() -> usize {
    let systeme = fs();
    let racine = match systeme.resolve(RACINE, 0) {
        Some(idx) => idx,
        None => match systeme.mkdir_at(0, "persist") {
            Ok(idx) => idx,
            Err(_) => return 0,
        },
    };

    let base = match debut() {
        Some(base) => base,
        None => {
            crate::kernel::dmesg::log("persistance: disque trop petit, zone absente");
            return 0;
        }
    };

    let mut entete = vec![0u8; SECTOR_SIZE];
    if ata::read(Drive::Slave, base, 1, &mut entete) != 1 {
        crate::kernel::dmesg::log("persistance: zone illisible");
        return 0;
    }
    if &entete[0..8] != MAGIE {
        // Zone neuve : rien a restaurer, et ce n'est pas une erreur.
        crate::kernel::dmesg::log("persistance: zone vierge");
        return 0;
    }
    let nombre = lit_u32(&entete[12..16]) as usize;
    if nombre == 0 || nombre > ENTREES_MAX {
        return 0;
    }

    let mut table = vec![0u8; (SECTEURS_TABLE as usize) * SECTOR_SIZE];
    if ata::read(Drive::Slave, base + 1, SECTEURS_TABLE as usize, &mut table)
        != SECTEURS_TABLE as usize
    {
        return 0;
    }

    let mut restaures = 0usize;
    let mut secteur = base + SECTEUR_CONTENU;
    for index in 0..nombre {
        let debut_entree = index * TAILLE_ENTREE;
        let brut = &table[debut_entree..debut_entree + TAILLE_ENTREE];
        let taille = lit_u64(&brut[CHEMIN_MAX..CHEMIN_MAX + 8]) as usize;
        let chemin = chaine(&brut[..CHEMIN_MAX]);
        let secteurs = ((taille + SECTOR_SIZE - 1) / SECTOR_SIZE) as u64;

        if !chemin.is_empty() && taille > 0 {
            let mut tampon = vec![0u8; (secteurs as usize) * SECTOR_SIZE];
            if ata::read(Drive::Slave, secteur, secteurs as usize, &mut tampon)
                == secteurs as usize
            {
                tampon.truncate(taille);
                if depose(racine, &chemin, &tampon) {
                    restaures += 1;
                }
            }
        }
        secteur += secteurs;
    }

    crate::kernel::dmesg::log_fmt(format_args!(
        "persistance: {} fichier(s) restaure(s) depuis le disque",
        restaures
    ));
    restaures
}

/// Ecrit `/persist` sur le disque. Rend le nombre de fichiers ecrits, ou -1.
///
/// L'en-tete part **en dernier** : jusqu'a ce qu'il soit ecrit, la zone porte
/// encore l'ancienne magie, donc l'ancien contenu. Une coupure au milieu laisse
/// la version precedente, pas un melange des deux.
pub fn synchronise() -> i64 {
    let base = match debut() {
        Some(base) => base,
        None => return -1,
    };

    let entrees = rassemble();
    if entrees.len() > ENTREES_MAX {
        crate::kernel::dmesg::log("persistance: trop de fichiers, sync refuse");
        return -1;
    }

    let mut table = vec![0u8; (SECTEURS_TABLE as usize) * SECTOR_SIZE];
    let mut secteur = base + SECTEUR_CONTENU;
    let fin_zone = base + SECTEURS_ZONE;

    for (index, entree) in entrees.iter().enumerate() {
        let octets = entree.chemin.as_bytes();
        let longueur = core::cmp::min(octets.len(), CHEMIN_MAX - 1);
        let debut_entree = index * TAILLE_ENTREE;
        table[debut_entree..debut_entree + longueur].copy_from_slice(&octets[..longueur]);
        ecrit_u64(
            &mut table[debut_entree + CHEMIN_MAX..debut_entree + CHEMIN_MAX + 8],
            entree.contenu.len() as u64,
        );

        let secteurs = ((entree.contenu.len() + SECTOR_SIZE - 1) / SECTOR_SIZE) as u64;
        if secteur + secteurs > fin_zone {
            crate::kernel::dmesg::log("persistance: zone pleine, sync tronque");
            return -1;
        }
        if secteurs > 0 {
            let mut tampon = vec![0u8; (secteurs as usize) * SECTOR_SIZE];
            tampon[..entree.contenu.len()].copy_from_slice(&entree.contenu);
            if ata::write(Drive::Slave, secteur, secteurs as usize, &tampon)
                != secteurs as usize
            {
                return -1;
            }
        }
        secteur += secteurs;
    }

    if ata::write(Drive::Slave, base + 1, SECTEURS_TABLE as usize, &table)
        != SECTEURS_TABLE as usize
    {
        return -1;
    }

    let mut entete = vec![0u8; SECTOR_SIZE];
    entete[0..8].copy_from_slice(MAGIE);
    ecrit_u32(&mut entete[8..12], 1);
    ecrit_u32(&mut entete[12..16], entrees.len() as u32);
    if ata::write(Drive::Slave, base, 1, &entete) != 1 {
        return -1;
    }

    entrees.len() as i64
}

/// Tous les fichiers sous `/persist`, chemins relatifs a cette racine.
fn rassemble() -> Vec<Entree> {
    let systeme = fs();
    let racine = match systeme.resolve(RACINE, 0) {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let mut entrees = Vec::new();
    collecte(racine, &String::new(), &mut entrees);
    entrees
}

fn collecte(dossier: usize, prefixe: &str, entrees: &mut Vec<Entree>) {
    let systeme = fs();
    // Les indices sont releves d'abord : la collecte n'ecrit pas, mais elle
    // emprunte le systeme de fichiers a chaque tour, et garder un iterateur
    // ouvert par-dessus serait fragile.
    let mut enfants = Vec::new();
    for index in 0..systeme.nodes.len() {
        if systeme.nodes[index].used && systeme.nodes[index].parent == dossier
            && index != dossier
        {
            enfants.push(index);
        }
    }

    for index in enfants {
        let nom = systeme.nodes[index].name_str();
        let chemin = if prefixe.is_empty() {
            String::from(nom)
        } else {
            format!("{}/{}", prefixe, nom)
        };
        match systeme.nodes[index].kind {
            NodeKind::Dir => collecte(index, &chemin, entrees),
            NodeKind::File => {
                let longueur = systeme.nodes[index].content_len();
                if longueur == 0 || chemin.len() >= CHEMIN_MAX {
                    continue;
                }
                let mut contenu = Vec::with_capacity(longueur);
                contenu.extend_from_slice(&systeme.nodes[index].content[..longueur]);
                entrees.push(Entree { chemin, contenu });
            }
        }
    }
}

/// Cree (dossiers compris) puis remplit un fichier sous `/persist`.
fn depose(racine: usize, chemin: &str, contenu: &[u8]) -> bool {
    let systeme = fs();
    let mut parent = racine;
    let mut morceaux = chemin.split('/').filter(|m| !m.is_empty()).peekable();

    while let Some(morceau) = morceaux.next() {
        if morceaux.peek().is_none() {
            let noeud = match systeme.find_child(parent, morceau) {
                Some(idx) => idx,
                None => match systeme.touch_at(parent, morceau) {
                    Ok(idx) => idx,
                    Err(_) => return false,
                },
            };
            return systeme.write_node_bytes(noeud, contenu);
        }
        parent = match systeme.find_child(parent, morceau) {
            Some(idx) => idx,
            None => match systeme.mkdir_at(parent, morceau) {
                Ok(idx) => idx,
                Err(_) => return false,
            },
        };
    }
    false
}

// --- Lecture et ecriture des champs -------------------------------------------

fn lit_u32(octets: &[u8]) -> u32 {
    u32::from_le_bytes([octets[0], octets[1], octets[2], octets[3]])
}

fn lit_u64(octets: &[u8]) -> u64 {
    let mut brut = [0u8; 8];
    brut.copy_from_slice(&octets[..8]);
    u64::from_le_bytes(brut)
}

fn ecrit_u32(cible: &mut [u8], valeur: u32) {
    cible[..4].copy_from_slice(&valeur.to_le_bytes());
}

fn ecrit_u64(cible: &mut [u8], valeur: u64) {
    cible[..8].copy_from_slice(&valeur.to_le_bytes());
}

fn chaine(octets: &[u8]) -> String {
    let fin = octets.iter().position(|&c| c == 0).unwrap_or(octets.len());
    String::from_utf8_lossy(&octets[..fin]).into_owned()
}
