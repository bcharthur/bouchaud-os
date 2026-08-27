//! Le cout d'une liberation dans le cache de pages propres, et son indice.
//!
//! # Ce que ces tests mesurent
//!
//! Le cache etait un `Vec<Arc<Entry>>` :
//!
//!   * `acquire`, `retain` et `release` cherchaient la cle par balayage ;
//!   * `release` appelait `reclaim_excess`, qui COMPTAIT les entrees
//!     recuperables en prenant le verrou de CHAQUE entree — un balayage
//!     complet, avec `C` prises de verrou, A CHAQUE LIBERATION.
//!
//! Un `madvise(DONTNEED)` rendant K pages propres coutait `O(K x C)` prises de
//! verrou, le gros verrou tenu.
//!
//! # L'indice
//!
//! `RECUPERABLES` remplace le comptage par une lecture atomique. Ce n'est pas
//! un invariant : il est mis a jour aux transitions, parfois sans le verrou du
//! cache. Il ne sert qu'a decider s'il faut REGARDER ; la recuperation
//! revalide chaque candidat sous les verrous.
//!
//! Deux choses doivent donc etre vraies, et elles sont testees ici : l'indice
//! ne peut pas passer sous zero — un compteur qui reboucle empecherait toute
//! recuperation pour toujours — et la file de candidats reste bornee.
//!
//! Lance par `tools/smp/test-cache-pages.sh`.

extern crate alloc;

use alloc::collections::{BTreeMap, VecDeque};

const MAX_RECUPERABLES: usize = 2048;
const LIMITE_CANDIDATS: usize = 4 * MAX_RECUPERABLES;

// ─── Le cout ───────────────────────────────────────────────────────────────

/// L'ancien : recherche par balayage, PUIS comptage par balayage a chaque
/// liberation.
fn cout_ancien(entrees: usize, liberations: usize) -> u64 {
    // `find` pour la cle, puis `filter().count()` sur tout le cache, puis
    // `position()` si le seuil est franchi. On ne compte que les deux premiers,
    // ce qui MINORE l'ancien cout.
    (entrees as u64) * 2 * (liberations as u64)
}

/// Le nouveau : `O(log C)` pour la cle, une lecture atomique pour la decision.
fn cout_nouveau(entrees: usize, liberations: usize) -> u64 {
    let log = 64 - (entrees.max(1) as u64).leading_zeros() as u64 + 1;
    (log + 1) * liberations as u64
}

#[test]
fn liberer_des_pages_propres_ne_balaye_plus_le_cache() {
    // Le cache plein, et un madvise qui rend 4096 pages propres.
    let ancien = cout_ancien(MAX_RECUPERABLES, 4096);
    let nouveau = cout_nouveau(MAX_RECUPERABLES, 4096);
    assert!(
        ancien > 16_000_000,
        "l'ancien doit bien couter des millions de prises de verrou ({ancien})"
    );
    assert!(
        nouveau * 200 < ancien,
        "le nouveau doit couter au moins deux cents fois moins ({nouveau} contre {ancien})"
    );
}

// ─── L'indice ──────────────────────────────────────────────────────────────

/// Modele des transitions reelles du cache.
struct Modele {
    /// cle -> nombre de mappings ; `None` = entree en echec.
    entrees: BTreeMap<u32, Option<usize>>,
    candidats: VecDeque<u32>,
    indice: usize,
}

impl Modele {
    fn neuf() -> Self {
        Self { entrees: BTreeMap::new(), candidats: VecDeque::new(), indice: 0 }
    }

    fn devient_recuperable(&mut self, key: u32) {
        self.indice += 1;
        if self.candidats.len() < LIMITE_CANDIDATS {
            self.candidats.push_back(key);
        }
    }

    fn cesse_d_etre_recuperable(&mut self) {
        self.indice = self.indice.saturating_sub(1);
    }

    fn charge(&mut self, key: u32, reussi: bool) {
        self.entrees.insert(key, if reussi { Some(1) } else { None });
        if !reussi {
            self.devient_recuperable(key);
        }
    }

    fn acquiert(&mut self, key: u32) {
        if let Some(Some(mappings)) = self.entrees.get_mut(&key) {
            let avant = *mappings;
            *mappings += 1;
            if avant == 0 {
                self.cesse_d_etre_recuperable();
            }
        }
    }

    fn libere(&mut self, key: u32) {
        let devenue_libre = match self.entrees.get_mut(&key) {
            Some(Some(mappings)) => {
                assert!(*mappings != 0, "double release");
                *mappings -= 1;
                *mappings == 0
            }
            _ => panic!("release d'une entree inconnue"),
        };
        if devenue_libre {
            self.devient_recuperable(key);
        }
    }

    /// Le comptage exact, tel que `lifetime_stats` le fait au releve.
    fn exact(&self) -> usize {
        self.entrees.values().filter(|m| matches!(m, None | Some(0))).count()
    }

    fn retire_un_candidat(&mut self) -> Option<u32> {
        while let Some(key) = self.candidats.pop_front() {
            let sortable = matches!(self.entrees.get(&key), Some(None) | Some(Some(0)));
            if sortable {
                self.cesse_d_etre_recuperable();
                self.entrees.remove(&key);
                return Some(key);
            }
        }
        let victime = *self
            .entrees
            .iter()
            .find(|(_, m)| matches!(m, None | Some(0)))
            .map(|(k, _)| k)?;
        self.cesse_d_etre_recuperable();
        self.entrees.remove(&victime);
        Some(victime)
    }
}

#[test]
fn l_indice_suit_le_compte_exact_sur_un_usage_simple() {
    let mut modele = Modele::neuf();
    for key in 0..100u32 {
        modele.charge(key, true);
    }
    assert_eq!(modele.indice, 0);
    for key in 0..40u32 {
        modele.libere(key);
    }
    assert_eq!(modele.indice, 40);
    assert_eq!(modele.exact(), 40, "l'indice colle au compte exact");
    for key in 0..15u32 {
        modele.acquiert(key);
    }
    assert_eq!(modele.indice, 25);
    assert_eq!(modele.exact(), 25);
}

/// LA propriete qui compte : un indice qui reboucle sous zero empecherait
/// toute recuperation pour toujours, et le cache grandirait sans fin.
#[test]
fn l_indice_ne_peut_pas_passer_sous_zero() {
    let mut modele = Modele::neuf();
    modele.charge(1, true);
    // Une salve de decrements parasites, comme une course pourrait en produire.
    for _ in 0..1000 {
        modele.cesse_d_etre_recuperable();
    }
    assert_eq!(modele.indice, 0, "saturant, jamais negatif");
    modele.libere(1);
    assert_eq!(modele.indice, 1, "et la recuperation redevient possible");
}

#[test]
fn un_candidat_reacquis_est_ignore_a_la_sortie() {
    let mut modele = Modele::neuf();
    modele.charge(7, true);
    modele.libere(7);            // 7 devient candidat
    modele.acquiert(7);          // ... puis est repris
    modele.charge(8, true);
    modele.libere(8);
    assert_eq!(modele.retire_un_candidat(), Some(8), "7 est jete, 8 sort");
    assert!(modele.entrees.contains_key(&7), "7 reste dans le cache");
}

#[test]
fn une_entree_en_echec_est_recuperable_immediatement() {
    let mut modele = Modele::neuf();
    modele.charge(3, false);
    assert_eq!(modele.indice, 1);
    assert_eq!(modele.exact(), 1);
    assert_eq!(modele.retire_un_candidat(), Some(3));
    assert_eq!(modele.indice, 0);
}

#[test]
fn la_file_de_candidats_reste_bornee() {
    let mut modele = Modele::neuf();
    for key in 0..(LIMITE_CANDIDATS as u32 * 3) {
        modele.charge(key, true);
        modele.libere(key);
    }
    assert_eq!(
        modele.candidats.len(), LIMITE_CANDIDATS,
        "la file ne doit pas croitre sans fin"
    );
    assert!(modele.indice > LIMITE_CANDIDATS, "l'indice, lui, compte tout");
}

/// Apres un debordement de la file, la recuperation doit quand meme trouver
/// une victime : c'est le repli par balayage.
#[test]
fn le_repli_par_balayage_trouve_une_victime_apres_debordement() {
    let mut modele = Modele::neuf();
    for key in 0..(LIMITE_CANDIDATS as u32 + 50) {
        modele.charge(key, true);
        modele.libere(key);
    }
    // On vide la file par des sorties successives.
    let mut sorties = 0;
    while modele.retire_un_candidat().is_some() {
        sorties += 1;
    }
    assert_eq!(
        sorties, LIMITE_CANDIDATS + 50,
        "toutes les entrees recuperables doivent finir par sortir"
    );
    assert!(modele.entrees.is_empty());
}

#[test]
fn une_liberation_qui_ne_tombe_pas_a_zero_ne_propose_rien() {
    let mut modele = Modele::neuf();
    modele.charge(1, true);
    modele.acquiert(1); // mappings = 2
    modele.libere(1);   // mappings = 1
    assert_eq!(modele.indice, 0);
    assert!(modele.candidats.is_empty());
    modele.libere(1);   // mappings = 0
    assert_eq!(modele.indice, 1);
    assert_eq!(modele.candidats.len(), 1);
}
