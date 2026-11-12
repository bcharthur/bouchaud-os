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
//! - la sortie est bornee (`octets_max`), chemins retenus ET chemins refuses
//!   compris (revue #300 : des refus hors budget gardaient des megaoctets sous
//!   le verrou) : au-dela, `tronque` ;
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
    // Un seul budget pour tout ce qui est retenu : chemins et refus.
    let mut octets = 0usize;
    // Pile explicite : (noeud, prochain enfant a examiner).
    let mut pile: Vec<(usize, usize)> = Vec::new();
    if a.dossier(depart) {
        if a.lisible(depart) {
            pile.push((depart, compte[depart] as usize));
        } else {
            let c = chemin(a, depart);
            if c.len() + 1 > octets_max {
                r.tronque = true;
                return r;
            }
            octets += c.len() + 1;
            r.refus.push(c);
        }
    }
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
                let c = chemin(a, e);
                if octets + c.len() + 1 > octets_max {
                    r.tronque = true;
                    return r;
                }
                octets += c.len() + 1;
                r.refus.push(c);
            }
        }
    }
    r
}

// --- `tree` (BOUCHAUD_TREE_ITERATIF_V1) ---------------------------------------

/// Une ligne de `tree`.
#[derive(Debug, PartialEq, Eq)]
pub struct Ligne {
    pub profondeur: usize,
    pub nom: String,
    pub dossier: bool,
    /// Ligne « permission refusee » : le contenu d'un repertoire illisible.
    pub refuse: bool,
}

/// Indentation affichee au plus : au-dela, la profondeur est ECRITE au lieu
/// d'etre dessinee. L'ancien `tree` indentait chaque niveau (un `print!` par
/// niveau !) : 15 Mo de console pour 3 860 niveaux, plus de 180 s sous KVM.
pub const INDENTATION_MAX: usize = 32;

/// Le debut d'une ligne de `tree` a `profondeur` : lineaire en la profondeur
/// jusqu'a `INDENTATION_MAX`, borne au-dela.
pub fn prefixe(profondeur: usize) -> String {
    let mut s = String::new();
    for _ in 0..profondeur.min(INDENTATION_MAX) {
        s.push_str("  ");
    }
    if profondeur > INDENTATION_MAX {
        s.push_str(&alloc::format!("[niveau {}] ", profondeur));
    }
    s
}

/// Les lignes de `tree` sous `depart`, dans l'ordre de l'ancien `tree`
/// (prefixe, enfants par indice croissant). Iteratif, chaque noeud vu une
/// fois ; on ne descend que dans les repertoires lisibles et traversables, les
/// autres donnent une ligne `refuse`. Au plus `lignes_max` lignes (`true` :
/// tronque).
pub fn lignes_arbre<A: Arbre>(a: &A, depart: usize, lignes_max: usize) -> (Vec<Ligne>, bool) {
    let n = a.taille();
    let mut lignes = Vec::new();
    if depart >= n || !a.utilise(depart) || !a.dossier(depart) {
        return (lignes, false);
    }
    if !a.lisible(depart) {
        lignes.push(Ligne { profondeur: 0, nom: String::new(), dossier: false, refuse: true });
        return (lignes, false);
    }
    let mut compte = vec![0u32; n + 1];
    for i in 1..n {
        if a.utilise(i) && a.parent(i) < n {
            compte[a.parent(i) + 1] += 1;
        }
    }
    for p in 0..n {
        compte[p + 1] += compte[p];
    }
    let mut enfants = vec![0u32; compte[n] as usize];
    let mut pose = compte.clone();
    for i in 1..n {
        if a.utilise(i) && a.parent(i) < n {
            let p = a.parent(i);
            enfants[pose[p] as usize] = i as u32;
            pose[p] += 1;
        }
    }
    let mut vu = vec![false; n];
    vu[depart] = true;
    // (noeud, prochain enfant, profondeur de ses enfants)
    let mut pile: Vec<(usize, usize, usize)> = vec![(depart, compte[depart] as usize, 0)];
    loop {
        let Some(sommet) = pile.last_mut() else { break };
        let (p, d) = (sommet.0, sommet.2);
        if sommet.1 >= compte[p + 1] as usize {
            pile.pop();
            continue;
        }
        let e = enfants[sommet.1] as usize;
        sommet.1 += 1;
        if vu[e] {
            continue;
        }
        vu[e] = true;
        if lignes.len() >= lignes_max {
            return (lignes, true);
        }
        let dossier = a.dossier(e);
        lignes.push(Ligne { profondeur: d, nom: String::from(a.nom(e)), dossier, refuse: false });
        if dossier {
            if a.lisible(e) {
                pile.push((e, compte[e] as usize, d + 1));
            } else {
                if lignes.len() >= lignes_max {
                    return (lignes, true);
                }
                lignes.push(Ligne { profondeur: d + 1, nom: String::new(), dossier: false, refuse: true });
            }
        }
    }
    (lignes, false)
}
