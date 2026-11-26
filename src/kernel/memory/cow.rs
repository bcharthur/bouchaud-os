//! BOUCHAUD_COW_V1 -- la propriete PARTAGEE des frames privees.
//!
//! # Les classes de frames
//!
//! Un espace d'adressage distingue deja ce qu'il POSSEDE (`AddressSpace::
//! pages` : pages anonymes, copies privees de fichiers) de ce qu'il EMPRUNTE
//! (cache de pages propres, plages `MAP_SHARED`, framebuffer, DMA) : seules
//! les premieres sont rendues a l'allocateur avec l'espace. Ce module ajoute
//! UNE classe, et une seule :
//!
//!   * frame privee EXCLUSIVE : possedee par un seul espace. Absente de la
//!     table -- c'est le cas ordinaire, et il ne coute rien ;
//!   * frame privee PARTAGEE en copie-a-l'ecriture : possedee par N >= 2
//!     espaces apres `fork`. Presente dans la table avec son compte.
//!
//! Les frames empruntees, de peripherique ou reservees n'entrent jamais ici :
//! leur duree de vie a deja un proprietaire (cache, pilote, carte memoire).
//! Les tables de pages et les frames du noyau non plus.
//!
//! # Les invariants
//!
//! 1. Une frame est dans la table SSI au moins deux espaces la possedent ;
//!    son compte est alors exactement ce nombre (>= 2).
//! 2. `rend` est appele UNE fois par espace qui cesse de posseder la frame
//!    (`munmap`, destruction, copie a l'ecriture). Il dit si la frame doit
//!    etre rendue a l'allocateur : seulement quand plus personne ne l'a.
//! 3. Un `rend` d'une frame absente signifie « dernier proprietaire » : la
//!    frame est liberee. Un second `rend` de la meme frame serait donc un
//!    double `free`, que l'allocateur detecte (bitmap) -- la table ne le
//!    masque pas.
//! 4. Une frame redevenue exclusive (compte 2 -> 1) QUITTE la table : son
//!    dernier proprietaire peut la rendre inscriptible sans copie.
//!
//! Module PUR : ni `use crate::`, ni verrou. Le noyau tient la table sous son
//! verrou ; `tools/memoire/test_cow.rs` la contredit sur l'hote.

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rendu {
    Liberer,
    Garder,
}

// BOUCHAUD_TABLE_COW_HACHEE_V1 -- la table etait un `BTreeMap<u64, u32>`.
// Un `fork` de 64 Mio y faisait 16 509 insertions (11,5 ms sous TCG, plus 10 ms
// de tri pour dedoublonner) ; chaque destruction d'espace et chaque casse COW y
// faisait une recherche. Ici : adressage ouvert, sonde lineaire, cle = frame + 1
// (une frame est alignee sur 4 Kio : la cle est impaire, jamais 0 = vide ni
// u64::MAX = tombe). O(1) attendu, aucune allocation hors croissance.
const VIDE: u64 = 0;
const TOMBE: u64 = u64::MAX;
const CAPACITE_MIN: usize = 64;

pub struct TableCow {
    cles: Vec<u64>,
    comptes: Vec<u32>,
    vivantes: usize,
    tombes: usize,
    partages: u64,
    liberees_partagees: u64,
}

#[inline]
fn cle(frame: u64) -> u64 {
    frame | 1
}

#[inline]
fn depart(frame: u64, masque: usize) -> usize {
    ((frame >> 12).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 17) as usize & masque
}

impl TableCow {
    pub const fn neuve() -> Self {
        Self { cles: Vec::new(), comptes: Vec::new(), vivantes: 0, tombes: 0, partages: 0, liberees_partagees: 0 }
    }

    /// L'emplacement de `frame`, s'il y est.
    fn trouve(&self, frame: u64) -> Option<usize> {
        if self.cles.is_empty() {
            return None;
        }
        let masque = self.cles.len() - 1;
        let k = cle(frame);
        let mut i = depart(frame, masque);
        loop {
            match self.cles[i] {
                VIDE => return None,
                c if c == k => return Some(i),
                _ => i = (i + 1) & masque,
            }
        }
    }

    /// Reconstruit la table a `capacite` emplacements (puissance de deux),
    /// sans tombes.
    fn retaille(&mut self, capacite: usize) -> bool {
        let mut nouvelles_cles = Vec::new();
        let mut nouveaux_comptes = Vec::new();
        if nouvelles_cles.try_reserve_exact(capacite).is_err()
            || nouveaux_comptes.try_reserve_exact(capacite).is_err()
        {
            return false;
        }
        nouvelles_cles.resize(capacite, VIDE);
        nouveaux_comptes.resize(capacite, 0);
        let anciennes = core::mem::replace(&mut self.cles, nouvelles_cles);
        let comptes = core::mem::replace(&mut self.comptes, nouveaux_comptes);
        self.tombes = 0;
        let masque = capacite - 1;
        for (c, n) in anciennes.into_iter().zip(comptes) {
            if c != VIDE && c != TOMBE {
                let mut i = depart(c & !1, masque);
                while self.cles[i] != VIDE {
                    i = (i + 1) & masque;
                }
                self.cles[i] = c;
                self.comptes[i] = n;
            }
        }
        true
    }

    /// Reserve avant toute mutation des comptes. Les frames du lot sont
    /// distinctes ; le verrou COW couvre la reservation et les insertions.
    fn reserve_partages(&mut self, frames: &[u64]) -> bool {
        let majorant = self.vivantes + self.tombes + frames.len();
        let seuil = self.cles.len() / 4 * 3;
        if majorant <= seuil {
            return true;
        }
        let nouvelles = frames.iter().filter(|&&frame| self.trouve(frame).is_none()).count();
        if self.vivantes + self.tombes + nouvelles <= seuil {
            return true;
        }
        let voulu = ((self.vivantes + nouvelles) * 2).max(CAPACITE_MIN).next_power_of_two();
        self.retaille(voulu)
    }

    /// Une place libre pour `frame` (absente), apres reservation.
    fn place(&mut self, frame: u64) -> usize {
        debug_assert!((self.vivantes + self.tombes) < self.cles.len());
        let masque = self.cles.len() - 1;
        let mut i = depart(frame, masque);
        loop {
            match self.cles[i] {
                VIDE => return i,
                TOMBE => {
                    self.tombes -= 1;
                    return i;
                }
                _ => i = (i + 1) & masque,
            }
        }
    }

    pub fn partage_lot(&mut self, frames: &[u64]) -> bool {
        if !self.reserve_partages(frames) {
            return false;
        }
        for &frame in frames {
            self.partage(frame);
        }
        true
    }

    fn partage(&mut self, frame: u64) {
        self.partages += 1;
        if let Some(i) = self.trouve(frame) {
            let n = self.comptes[i].checked_add(1).expect("cow: debordement du compte de proprietaires");
            self.comptes[i] = n;
            return;
        }
        let i = self.place(frame);
        self.cles[i] = cle(frame);
        self.comptes[i] = 2;
        self.vivantes += 1;
    }

    pub fn rend(&mut self, frame: u64) -> Rendu {
        match self.trouve(frame) {
            None => Rendu::Liberer,
            Some(i) => {
                debug_assert!(self.comptes[i] >= 2, "cow: frame en table avec un seul proprietaire");
                self.comptes[i] -= 1;
                if self.comptes[i] <= 1 {
                    self.cles[i] = TOMBE;
                    self.comptes[i] = 0;
                    self.vivantes -= 1;
                    self.tombes += 1;
                    self.liberees_partagees += 1;
                }
                Rendu::Garder
            }
        }
    }

    pub fn proprietaires(&self, frame: u64) -> u32 {
        self.trouve(frame).map_or(1, |i| self.comptes[i])
    }

    pub fn est_partagee(&self, frame: u64) -> bool {
        self.trouve(frame).is_some()
    }

    pub fn frames_partagees(&self) -> usize {
        self.vivantes
    }

    pub fn cumuls(&self) -> (u64, u64) {
        (self.partages, self.liberees_partagees)
    }
}
