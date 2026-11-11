//! BOUCHAUD_FIND_PARCOURS_V1 -- parcours d'une arborescence, PUR et borne.
//!
//! Teste sur l'hote (`tools/fs/test_parcours.rs`, lance par
//! `tools/ci/run_host_tests.sh`). Aucun acces au noyau ni a un verrou : le
//! parcours travaille sur une `Arbre`, une vue en lecture seule que l'appelant
//! fournit SOUS SON verrou, une seule fois.
//!
//! # Ce que l'ancien `find` faisait
//!
//! `find` tenait le verrou du RAMFS et appelait `find_rec`, qui le reprenait :
//! lockdep panique (construction de debogage, `LOCKDEP inversion ... vfs(50)`),
//! sinon le coeur boucle pour toujours. `find_rec` se rappelait en plus
//! recursivement (pile noyau proportionnelle a la profondeur), balayait les
//! 4096 noeuds pour chaque repertoire (quadratique), imprimait sous le verrou,
//! et descendait dans des repertoires que l'utilisateur ne peut pas lire.
//!
//! # Ce que fait celui-ci
//!
//! - un index des enfants construit en UN passage (lineaire) ;
//! - une pile EXPLICITE (aucune recursion, quelle que soit la profondeur) ;
//! - chaque noeud visite au plus une fois (des liens `parent` corrompus qui
//!   formeraient un cycle ne font pas boucler) ;
//! - on ne descend que dans les repertoires que l'appelant peut lire ET
//!   traverser ; les autres sont signales (`refus`), pas parcourus ;
//! - la sortie est bornee (`octets_max`) : au-dela, `tronque` ;
//! - l'ordre est celui de l'ancien `find` : prefixe, enfants par indice
//!   croissant.
//!
//! La vue est un INSTANTANE : l'appelant la lit sous un verrou pris une fois.
//! Une modification concurrente de l'arborescence se voit au `find` suivant ;
//! elle ne peut ni corrompre ni bloquer celui-ci.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Une arborescence vue en lecture seule.
pub trait Arbre {
    /// Nombre d'emplacements (indices `0..taille`).
    fn taille(&self) -> usize;
    fn utilise(&self, i: usize) -> bool;
    fn parent(&self, i: usize) -> usize;
    fn dossier(&self, i: usize) -> bool;
    fn nom(&self, i: usize) -> &str;
    /// L'appelant peut-il lister CE repertoire (lecture et traversee) ?
    fn lisible(&self, i: usize) -> bool;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resultat {
    /// Chemins absolus des noeuds retenus, dans l'ordre du parcours.
    pub chemins: Vec<String>,
    /// Repertoires non parcourus faute de droit (chemins absolus).
    pub refus: Vec<String>,
    /// La borne de sortie a ete atteinte : le resultat est incomplet.
    pub tronque: bool,
    /// Noeuds examines.
    pub visites: usize,
}

/// Chemin absolu de `i` (`/` pour la racine 0). Borne par la taille de
/// l'arbre : une chaine de parents qui ne remonte pas a la racine (cycle) est
/// coupee et marquee `/?`.
pub fn chemin<A: Arbre>(a: &A, i: usize) -> String {
    if i == 0 {
        return String::from("/");
    }
    let mut morceaux: Vec<usize> = Vec::new();
    let mut c = i;
    let mut pas = 0usize;
    while c != 0 {
        if pas > a.taille() || c >= a.taille() {
            morceaux.clear();
            return String::from("/?");
        }
        morceaux.push(c);
        c = a.parent(c);
        pas += 1;
    }
    let mut s = String::new();
    for &m in morceaux.iter().rev() {
        s.push('/');
        s.push_str(a.nom(m));
    }
    s
}

/// Parcourt les DESCENDANTS de `depart` (lui-meme exclu, comme l'ancien
/// `find`), retient ceux dont le nom contient `filtre`.
pub fn parcours<A: Arbre>(a: &A, depart: usize, filtre: Option<&str>, octets_max: usize) -> Resultat {
    let n = a.taille();
    let mut r = Resultat::default();
    if depart >= n || !a.utilise(depart) {
        return r;
    }
    // Index des enfants, en un passage : `debut[p]..debut[p+1]` dans `enfants`.
    let mut compte = vec![0u32; n + 1];
    for i in 0..n {
        if a.utilise(i) && i != 0 {
            let p = a.parent(i);
            if p < n {
                compte[p + 1] += 1;
            }
        }
    }
    for p in 0..n {
        compte[p + 1] += compte[p];
    }
    let mut enfants = vec![0u32; compte[n] as usize];
    let mut pose = compte.clone();
    for i in 0..n {
        if a.utilise(i) && i != 0 {
            let p = a.parent(i);
            if p < n {
                enfants[pose[p] as usize] = i as u32;
                pose[p] += 1;
            }
        }
    }
    let mut vu = vec![false; n];
    vu[depart] = true;
    // Pile explicite : (noeud, prochain enfant a examiner).
    let mut pile: Vec<(usize, usize)> = Vec::new();
    if a.dossier(depart) {
        if a.lisible(depart) {
            pile.push((depart, compte[depart] as usize));
        } else {
            r.refus.push(chemin(a, depart));
        }
    }
    let mut octets = 0usize;
    loop {
        let Some(sommet) = pile.last_mut() else { break };
        let p = sommet.0;
        if sommet.1 >= compte[p + 1] as usize {
            pile.pop();
            continue;
        }
        let e = enfants[sommet.1] as usize;
        sommet.1 += 1;
        if vu[e] {
            continue; // cycle dans des liens corrompus
        }
        vu[e] = true;
        r.visites += 1;
        if filtre.map_or(true, |f| a.nom(e).contains(f)) {
            let c = chemin(a, e);
            if octets + c.len() + 1 > octets_max {
                r.tronque = true;
                return r;
            }
            octets += c.len() + 1;
            r.chemins.push(c);
        }
        if a.dossier(e) {
            if a.lisible(e) {
                pile.push((e, compte[e] as usize));
            } else {
                r.refus.push(chemin(a, e));
            }
        }
    }
    r
}
