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
//! secteurs 1 a 1024    table : 2048 entrees de 256 octets (chemin, taille)
//! secteurs 1025 a ...  contenu, chaque fichier aligne sur un secteur
//! ```
//!
//! La zone entiere occupe `SECTEURS_ZONE` secteurs, soit 128 Mio.
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
const ENTREES_MAX: usize = 2048;

/// Taille d'une entree de table, en octets.
const TAILLE_ENTREE: usize = 256;

/// Longueur maximale d'un chemin dans la table.
const CHEMIN_MAX: usize = TAILLE_ENTREE - 16;

/// Secteurs occupes par la table.
const SECTEURS_TABLE: u64 = (ENTREES_MAX * TAILLE_ENTREE / SECTOR_SIZE) as u64;

/// Premier secteur du contenu, relatif au debut de la zone.
const SECTEUR_CONTENU: u64 = 1 + SECTEURS_TABLE;

/// Taille de la zone, en secteurs. 128 Mio.
///
/// `tools/userland/mkdisk.sh` ajoute exactement autant de secteurs nuls a la
/// fin de l'image : les deux valeurs doivent bouger ensemble.
const SECTEURS_ZONE: u64 = 262144;

/// Racine des fichiers persistants dans le RAMFS.
pub const RACINE: &str = "/persist";

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


/// Un fichier retenu : son chemin sous [`RACINE`] et OU le trouver.
///
/// Le contenu n'est plus recopie a la collecte. Il l'etait pour tous les
/// fichiers, a chaque `fsync`, et la plupart ne sont pas ecrits : voir
/// [`synchronise`].
struct Entree {
    chemin: String,
    noeud: usize,
    longueur: usize,
}

// BOUCHAUD_PERSIST_ECRITURE_INCREMENTALE_V1
//
// CE QUE `fsync` COUTAIT
// ----------------------
// L'en-tete de ce fichier posait une hypothese : « quelques mega-octets ecrits
// rarement ». Le runtime l'a dementie. Ladybird stocke ses temoins et son cache
// dans SQLite, qui appelle `fsync` a chaque transaction -- et `fsync` reecrivait
// LA ZONE ENTIERE : chaque fichier recopie en memoire, puis chaque secteur
// repousse sur le disque en PIO, sous le gros verrou du noyau. `[BKL-SYSCALL]`
// donnait `fsync` a 17-18 % de detention.
//
// Un `fsync` qui suit l'ecriture d'un seul fichier reecrivait donc tous les
// autres, octet pour octet identiques.
//
// CE QUI EST GARDE
// ----------------
// L'empreinte de ce que le dernier `synchronise` REUSSI a laisse sur le disque :
// pour chaque entree, son chemin, le secteur ou elle commence, sa longueur et
// un sceau de son contenu. Une entree dont les quatre coincident est deja sur
// le disque, a cet endroit exact : la reecrire n'ecrirait rien de nouveau.
//
// POURQUOI C'EST SUR
// ------------------
// * L'empreinte n'est mise a jour qu'apres un `synchronise` COMPLETEMENT
//   reussi. Le moindre echec la vide, et le `sync` suivant reecrit tout.
// * Le secteur fait partie de la cle : si un fichier precedent change de
//   taille, tous ceux qui le suivent se decalent, leur secteur ne correspond
//   plus, et ils sont reecrits.
// * La table et l'en-tete sont TOUJOURS reecrits. Ils sont bornes
//   (1025 secteurs) et ce sont eux qui rendent la zone lisible ; les
//   economiser ferait courir un risque sans rapport avec le gain.
// * Le sceau est un couple de deux FNV-1a de bases differentes, plus la
//   longueur : 128 bits pour decider de NE PAS ecrire. Une collision serait
//   une perte de donnees, d'ou les deux.
//
// Ces variables ne sont touchees que par `synchronise` et `monte`, qui
// s'executent sous le gros verrou -- `FSYNC`, `FDATASYNC` et `SYNC` ne figurent
// pas dans `compat::linux::bkl::SANS_BKL`, et `tools/verifie-persistance.py`
// le verifie.
struct SurDisque {
    chemin: String,
    secteur: u64,
    longueur: usize,
    sceau: (u64, u64),
}

static mut DISQUE: Vec<SurDisque> = Vec::new();

/// Sceau de contenu : deux FNV-1a de bases differentes.
///
/// Sert uniquement a decider de ne PAS reecrire un secteur. Un sceau de 64 bits
/// suffirait en pratique ; deux en font 128, parce que le prix d'une collision
/// serait un fichier perime sur le disque, et le prix d'un second passage est
/// une multiplication par octet.
fn sceau(contenu: &[u8]) -> (u64, u64) {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x9e37_79b9_7f4a_7c15;
    for &octet in contenu {
        a = (a ^ octet as u64).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b ^ octet as u64).wrapping_mul(0x8864_0000_0000_003d);
    }
    (a, b)
}

/// L'entree `index` est-elle deja sur le disque, a ce secteur, avec ce sceau ?
fn deja_ecrite(index: usize, chemin: &str, secteur: u64, longueur: usize,
    sceau_courant: (u64, u64)) -> bool {
    let disque = unsafe { &*core::ptr::addr_of!(DISQUE) };
    match disque.get(index) {
        Some(connu) => {
            connu.chemin == chemin
                && connu.secteur == secteur
                && connu.longueur == longueur
                && connu.sceau == sceau_courant
        }
        None => false,
    }
}

/// Oublie ce qu'on croyait savoir du disque : le prochain `sync` reecrit tout.
///
/// Appele des qu'une ecriture echoue, et au montage. Ne jamais s'en passer :
/// une empreinte qui survit a un echec ferait sauter des ecritures qui n'ont
/// jamais eu lieu.
fn oublie_le_disque() {
    unsafe {
        let disque = &mut *core::ptr::addr_of_mut!(DISQUE);
        disque.clear();
    }
}

/// Deplie la zone persistante dans `/persist`. A appeler une fois au demarrage.
///
/// Rend le nombre de fichiers restaures.
pub fn monte() -> usize {
    // Rien n'est encore connu du disque de ce cote : le premier `sync` ecrira
    // tout. Le contenu lu ici a beau venir du disque, `depose` peut le
    // transformer -- un fichier deja present, un dossier manquant --, et une
    // empreinte tiree d'une lecture ne prouverait donc pas ce que le disque
    // porte apres coup.
    oublie_le_disque();
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
    // Chaque echec doit se nommer. `fsync` traduit un `-1` en EIO, et un
    // programme qui recoit EIO n'a plus que « disk I/O error » a dire : c'est
    // exactement ce que SQLite a rapporte au run 32424806818, sans qu'aucune
    // ligne du journal n'indique laquelle des quatre causes s'appliquait.
    let base = match debut() {
        Some(base) => base,
        None => {
            let (_, secteurs) = ata::capacities();
            crate::kernel::dmesg::log_fmt(format_args!(
                "persistance: sync refuse, disque de {} secteurs, il en faut plus de {}",
                secteurs,
                SECTEURS_ZONE + SECTEUR_CONTENU
            ));
            oublie_le_disque();
            return -1;
        }
    };

    let entrees = rassemble();
    if entrees.len() > ENTREES_MAX {
        crate::kernel::dmesg::log_fmt(format_args!(
            "persistance: sync refuse, {} fichiers sous {} pour {} entrees possibles",
            entrees.len(),
            RACINE,
            ENTREES_MAX
        ));
        oublie_le_disque();
        return -1;
    }

    let mut table = vec![0u8; (SECTEURS_TABLE as usize) * SECTOR_SIZE];
    let mut secteur = base + SECTEUR_CONTENU;
    let fin_zone = base + SECTEURS_ZONE;

    let mut nouveau: Vec<SurDisque> = Vec::with_capacity(entrees.len());
    let mut ecrites = 0usize;
    let mut sautees = 0usize;

    for (index, entree) in entrees.iter().enumerate() {
        let octets = entree.chemin.as_bytes();
        // Un chemin trop long etait TRONQUE en silence. La table gardait alors
        // un nom qui n'est celui d'aucun fichier, et le redemarrage suivant
        // restaurait le contenu sous ce faux nom -- une corruption discrete,
        // que rien dans le journal n'aurait signalee. Depuis que `NAME_LEN`
        // vaut 255, un seul composant peut a lui seul approcher ce plafond.
        if octets.len() >= CHEMIN_MAX {
            crate::kernel::dmesg::log_fmt(format_args!(
                "persistance: chemin trop long, {} octets pour {} possibles : '{}'",
                octets.len(),
                CHEMIN_MAX - 1,
                entree.chemin
            ));
            oublie_le_disque();
            return -1;
        }
        let longueur = octets.len();
        let debut_entree = index * TAILLE_ENTREE;
        table[debut_entree..debut_entree + longueur].copy_from_slice(&octets[..longueur]);
        ecrit_u64(
            &mut table[debut_entree + CHEMIN_MAX..debut_entree + CHEMIN_MAX + 8],
            entree.longueur as u64,
        );

        let secteurs = ((entree.longueur + SECTOR_SIZE - 1) / SECTOR_SIZE) as u64;
        if secteur + secteurs > fin_zone {
            crate::kernel::dmesg::log_fmt(format_args!(
                "persistance: zone pleine sur '{}', il faudrait le secteur {} et la zone s'arrete a {}",
                entree.chemin,
                secteur + secteurs,
                fin_zone
            ));
            oublie_le_disque();
            return -1;
        }
        // BOUCHAUD_PERSIST_ECRITURE_INCREMENTALE_V1 : le sceau se calcule sur
        // le contenu la ou il est. C'est une lecture memoire ; l'ecriture qu'il
        // evite est une rafale PIO vers l'ATA, plusieurs ordres de grandeur
        // plus chere.
        let sceau_courant = {
            let systeme = fs();
            sceau(&systeme.nodes[entree.noeud].content[..entree.longueur])
        };
        if secteurs > 0 {
            if deja_ecrite(index, &entree.chemin, secteur, entree.longueur, sceau_courant) {
                sautees += 1;
            } else {
                let mut tampon = vec![0u8; (secteurs as usize) * SECTOR_SIZE];
                {
                    let systeme = fs();
                    tampon[..entree.longueur]
                        .copy_from_slice(&systeme.nodes[entree.noeud].content[..entree.longueur]);
                }
                let ecrits = ata::write(Drive::Slave, secteur, secteurs as usize, &tampon);
                if ecrits != secteurs as usize {
                    crate::kernel::dmesg::log_fmt(format_args!(
                        "persistance: ecriture de '{}' incomplete, {} secteurs sur {} a partir de {}",
                        entree.chemin, ecrits, secteurs, secteur
                    ));
                    oublie_le_disque();
                    return -1;
                }
                ecrites += 1;
            }
        }
        nouveau.push(SurDisque {
            chemin: entree.chemin.clone(),
            secteur,
            longueur: entree.longueur,
            sceau: sceau_courant,
        });
        secteur += secteurs;
    }

    let table_ecrite = ata::write(Drive::Slave, base + 1, SECTEURS_TABLE as usize, &table);
    if table_ecrite != SECTEURS_TABLE as usize {
        crate::kernel::dmesg::log_fmt(format_args!(
            "persistance: table incomplete, {} secteurs sur {} a partir de {}",
            table_ecrite,
            SECTEURS_TABLE,
            base + 1
        ));
        oublie_le_disque();
        return -1;
    }

    let mut entete = vec![0u8; SECTOR_SIZE];
    entete[0..8].copy_from_slice(MAGIE);
    ecrit_u32(&mut entete[8..12], 1);
    ecrit_u32(&mut entete[12..16], entrees.len() as u32);
    if ata::write(Drive::Slave, base, 1, &entete) != 1 {
        crate::kernel::dmesg::log_fmt(format_args!(
            "persistance: en-tete non ecrit au secteur {}",
            base
        ));
        oublie_le_disque();
        return -1;
    }

    // Tout est passe : ce que le disque porte est maintenant connu. C'est le
    // SEUL endroit ou l'empreinte est adoptee.
    unsafe {
        let disque = &mut *core::ptr::addr_of_mut!(DISQUE);
        *disque = nouveau;
    }
    if sautees != 0 {
        crate::kernel::dmesg::log_fmt(format_args!(
            "persistance: sync, {} fichier(s) ecrit(s), {} inchange(s)",
            ecrites, sautees
        ));
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
                // Le contenu reste ou il est : `synchronise` le lit dans le
                // RAMFS au moment d'ecrire, et pour les entrees inchangees il
                // ne le lit que pour en calculer le sceau.
                entrees.push(Entree { chemin, noeud: index, longueur });
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
