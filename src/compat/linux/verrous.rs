//! Verrous d'enregistrement POSIX : `fcntl(F_GETLK/F_SETLK/F_SETLKW)`.
//!
//! P0-NG1 donne enfin a cette table son propre domaine de synchronisation.
//! L'ancienne `static mut Vec` n'etait sure que parce que l'ABI gardait le BKL.
//! La verification de conflit et la pose sont maintenant une transaction sous
//! un RankedSpinLock, ce qui ferme aussi la course check-then-insert.

use alloc::vec::Vec;
use crate::kernel::sync::{lockdep::LockClass, portee, Domaine, RankedSpinLock};

pub const F_RDLCK: i16 = 0;
pub const F_WRLCK: i16 = 1;
pub const F_UNLCK: i16 = 2;
const SANS_FIN: u64 = u64::MAX;

#[derive(Clone, Copy)]
struct Verrou {
    noeud: usize,
    pid: u32,
    genre: i16,
    debut: u64,
    fin: u64,
}

static VERROUS: RankedSpinLock<Vec<Verrou>> =
    RankedSpinLock::new(LockClass::PosixRecord, Vec::new());

fn se_chevauchent(a_debut: u64, a_fin: u64, b_debut: u64, b_fin: u64) -> bool {
    a_debut < b_fin && b_debut < a_fin
}
fn incompatibles(a: i16, b: i16) -> bool { a == F_WRLCK || b == F_WRLCK }

fn conflit_dans(
    table: &[Verrou], noeud: usize, pid: u32, genre: i16, debut: u64, fin: u64,
) -> Option<Verrou> {
    table.iter().find(|v| {
        v.noeud == noeud && v.pid != pid && incompatibles(v.genre, genre)
            && se_chevauchent(v.debut, v.fin, debut, fin)
    }).copied()
}

fn retire_plage_dans(verrous: &mut Vec<Verrou>, noeud: usize, pid: u32, debut: u64, fin: u64) {
    let mut ajouts: Vec<Verrou> = Vec::new();
    verrous.retain_mut(|v| {
        if v.noeud != noeud || v.pid != pid || !se_chevauchent(v.debut, v.fin, debut, fin) {
            return true;
        }
        let garde_avant = v.debut < debut;
        let garde_apres = v.fin > fin;
        if garde_avant && garde_apres {
            ajouts.push(Verrou { fin: v.fin, debut: fin, ..*v });
            v.fin = debut;
            true
        } else if garde_avant {
            v.fin = debut;
            true
        } else if garde_apres {
            v.debut = fin;
            true
        } else {
            false
        }
    });
    verrous.extend(ajouts);
}

pub enum Pose { Accorde, Occupe }

pub fn pose(noeud: usize, pid: u32, genre: i16, debut: u64, longueur: u64) -> Pose {
    let _domaine = portee(Domaine::VerrouEnregistrement);
    let fin = borne(debut, longueur);
    let mut table = VERROUS.lock();
    if genre == F_UNLCK {
        retire_plage_dans(&mut table, noeud, pid, debut, fin);
        return Pose::Accorde;
    }
    if conflit_dans(&table, noeud, pid, genre, debut, fin).is_some() {
        return Pose::Occupe;
    }
    retire_plage_dans(&mut table, noeud, pid, debut, fin);
    table.push(Verrou { noeud, pid, genre, debut, fin });
    Pose::Accorde
}

pub fn interroge(
    noeud: usize, pid: u32, genre: i16, debut: u64, longueur: u64,
) -> Option<(i16, u64, u64, u32)> {
    let _domaine = portee(Domaine::VerrouEnregistrement);
    let fin = borne(debut, longueur);
    let table = VERROUS.lock();
    conflit_dans(&table, noeud, pid, genre, debut, fin).map(|v| {
        let longueur = if v.fin == SANS_FIN { 0 } else { v.fin - v.debut };
        (v.genre, v.debut, longueur, v.pid)
    })
}

fn borne(debut: u64, longueur: u64) -> u64 {
    if longueur == 0 { SANS_FIN } else { debut.saturating_add(longueur) }
}

pub fn libere_fichier(noeud: usize, pid: u32) {
    let _domaine = portee(Domaine::VerrouEnregistrement);
    VERROUS.lock().retain(|v| !(v.noeud == noeud && v.pid == pid));
}

pub fn libere_processus(pid: u32) {
    let _domaine = portee(Domaine::VerrouEnregistrement);
    VERROUS.lock().retain(|v| v.pid != pid);
}

pub fn compte() -> usize {
    let _domaine = portee(Domaine::VerrouEnregistrement); VERROUS.lock().len() }
