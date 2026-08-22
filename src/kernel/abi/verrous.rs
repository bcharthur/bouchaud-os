//! Verrous d'enregistrement POSIX : `fcntl(F_GETLK/F_SETLK/F_SETLKW)`.
//!
//! ## Pourquoi cela existe
//!
//! SQLite — donc IndexedDB, les cookies SQL et tout le stockage persistant du
//! navigateur — construit sa concurrence entierement sur ces trois commandes
//! (`os_unix.c` : `unixLock`, `unixUnlock`, `unixCheckReservedLock`). Et son
//! algorithme ne regarde pas le code de retour de `F_GETLK` : il lit ce que le
//! noyau a **ecrit dans la structure** :
//!
//! ```c
//! lock.l_type = F_WRLCK;
//! osFcntl(fd, F_GETLK, &lock);
//! if (lock.l_type != F_UNLCK) reserved = 1;   // « quelqu'un me bloque »
//! ```
//!
//! `sys_fcntl` repondait auparavant `0` a toute commande inconnue sans jamais
//! toucher la structure. `l_type` restait donc a `F_WRLCK`, SQLite en concluait
//! qu'un autre processus tenait un verrou RESERVED, et rendait `SQLITE_BUSY`
//! sur chaque transaction en ecriture. Le defaut etait invisible dans le code
//! de retour ; `tools/userland/verrous-probe.c` le montre en cinq lignes.
//!
//! ## Le modele
//!
//! POSIX attache un verrou d'enregistrement au couple (processus, fichier), pas
//! au descripteur. L'identite du fichier est ici l'index du nœud RAMFS : deux
//! `open` du meme chemin donnent le meme nœud, donc les memes verrous, ce qui
//! est exactement la semantique attendue.
//!
//! Trois consequences que l'implementation doit respecter, et que la sonde
//! verifie :
//!
//! - un processus ne se bloque **jamais** lui-meme : reposer un verrou sur une
//!   plage qu'il tient deja remplace l'ancien ;
//! - `F_GETLK` ne rapporte que les verrous des **autres** processus ;
//! - les verrous d'un processus disparaissent avec lui, et avec la fermeture de
//!   n'importe lequel de ses descripteurs sur ce fichier.

use alloc::vec::Vec;

/// `l_type` de `struct flock`, valeurs de l'ABI Linux x86-64.
pub const F_RDLCK: i16 = 0;
pub const F_WRLCK: i16 = 1;
pub const F_UNLCK: i16 = 2;

/// Fin de plage d'un verrou ouvert (`l_len == 0` : « jusqu'a la fin »).
const SANS_FIN: u64 = u64::MAX;

#[derive(Clone, Copy)]
struct Verrou {
    /// Index du nœud RAMFS : l'identite du fichier.
    noeud: usize,
    /// Processus detenteur.
    pid: u32,
    /// `F_RDLCK` ou `F_WRLCK`.
    genre: i16,
    /// Premier octet couvert.
    debut: u64,
    /// Premier octet **non** couvert.
    fin: u64,
}

static mut VERROUS: Option<Vec<Verrou>> = None;

fn table() -> &'static mut Vec<Verrou> {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(VERROUS);
        slot.get_or_insert_with(Vec::new)
    }
}

fn se_chevauchent(a_debut: u64, a_fin: u64, b_debut: u64, b_fin: u64) -> bool {
    a_debut < b_fin && b_debut < a_fin
}

/// Deux verrous sont-ils incompatibles ?
///
/// Deux lectures coexistent ; toute ecriture exclut tout le reste.
fn incompatibles(a: i16, b: i16) -> bool {
    a == F_WRLCK || b == F_WRLCK
}

/// Le verrou d'un **autre** processus qui interdirait cette demande.
fn conflit(noeud: usize, pid: u32, genre: i16, debut: u64, fin: u64) -> Option<Verrou> {
    table()
        .iter()
        .find(|v| {
            v.noeud == noeud
                && v.pid != pid
                && incompatibles(v.genre, genre)
                && se_chevauchent(v.debut, v.fin, debut, fin)
        })
        .copied()
}

/// Retire la plage `[debut, fin)` des verrous que `pid` tient sur `noeud`.
///
/// Un verrou qui deborde de part et d'autre est coupe en deux : c'est ce que
/// POSIX demande, et ce dont depend `unixUnlock` quand SQLite relache la seule
/// plage de l'octet « pending » sans toucher au reste du fichier.
fn retire_plage(noeud: usize, pid: u32, debut: u64, fin: u64) {
    let verrous = table();
    let mut ajouts: Vec<Verrou> = Vec::new();

    verrous.retain_mut(|v| {
        if v.noeud != noeud || v.pid != pid || !se_chevauchent(v.debut, v.fin, debut, fin) {
            return true;
        }
        let garde_avant = v.debut < debut;
        let garde_apres = v.fin > fin;
        if garde_avant && garde_apres {
            // Coupe au milieu : on garde la tete ici, la queue part en ajout.
            ajouts.push(Verrou { fin: v.fin, debut: fin, ..*v });
            v.fin = debut;
            return true;
        }
        if garde_avant {
            v.fin = debut;
            return true;
        }
        if garde_apres {
            v.debut = fin;
            return true;
        }
        // Entierement couvert.
        false
    });

    verrous.extend(ajouts);
}

/// Resultat d'une demande `F_SETLK`/`F_SETLKW`.
pub enum Pose {
    Accorde,
    /// Un autre processus tient une plage incompatible.
    Occupe,
}

/// Pose (ou retire, si `genre == F_UNLCK`) un verrou.
pub fn pose(noeud: usize, pid: u32, genre: i16, debut: u64, longueur: u64) -> Pose {
    let fin = borne(debut, longueur);

    if genre == F_UNLCK {
        retire_plage(noeud, pid, debut, fin);
        return Pose::Accorde;
    }

    if conflit(noeud, pid, genre, debut, fin).is_some() {
        return Pose::Occupe;
    }

    // Un processus ne se bloque pas lui-meme : ses propres verrous sur la plage
    // sont remplaces, pas empiles.
    retire_plage(noeud, pid, debut, fin);
    table().push(Verrou { noeud, pid, genre, debut, fin });
    Pose::Accorde
}

/// Le verrou qui bloquerait cette demande, s'il en existe un.
///
/// Rend `(genre, debut, longueur, pid)`, ou `None` si la plage est libre pour
/// ce processus. `longueur` vaut 0 pour un verrou ouvert, comme a l'entree.
pub fn interroge(
    noeud: usize,
    pid: u32,
    genre: i16,
    debut: u64,
    longueur: u64,
) -> Option<(i16, u64, u64, u32)> {
    let fin = borne(debut, longueur);
    conflit(noeud, pid, genre, debut, fin).map(|v| {
        let longueur = if v.fin == SANS_FIN { 0 } else { v.fin - v.debut };
        (v.genre, v.debut, longueur, v.pid)
    })
}

/// Fin exclusive d'une plage, `longueur == 0` signifiant « jusqu'a la fin ».
fn borne(debut: u64, longueur: u64) -> u64 {
    if longueur == 0 {
        SANS_FIN
    } else {
        debut.saturating_add(longueur)
    }
}

/// Relache tous les verrous que `pid` tient sur `noeud`.
///
/// POSIX : fermer **n'importe quel** descripteur d'un processus sur un fichier
/// relache tous les verrous de ce processus sur ce fichier. SQLite en depend
/// pour rendre la base a la fermeture.
pub fn libere_fichier(noeud: usize, pid: u32) {
    table().retain(|v| !(v.noeud == noeud && v.pid == pid));
}

/// Relache tous les verrous d'un processus qui se termine.
pub fn libere_processus(pid: u32) {
    table().retain(|v| v.pid != pid);
}

/// Nombre de verrous actuellement tenus, pour le diagnostic systeme.
pub fn compte() -> usize {
    table().len()
}
