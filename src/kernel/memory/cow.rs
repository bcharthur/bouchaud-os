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

use alloc::collections::BTreeMap;

/// Ce que l'appelant doit faire de la frame qu'il cesse de posseder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rendu {
    /// Plus personne ne la possede : la rendre a l'allocateur.
    Liberer,
    /// Un autre espace la possede encore : ne rien liberer.
    Garder,
}

/// Les frames privees possedees par plusieurs espaces.
pub struct TableCow {
    proprietaires: BTreeMap<u64, u32>,
    partages: u64,
    liberees_partagees: u64,
}

impl TableCow {
    pub const fn neuve() -> Self {
        Self { proprietaires: BTreeMap::new(), partages: 0, liberees_partagees: 0 }
    }

    /// Un espace de plus possede `frame` (`fork`). Rend le nouveau compte.
    pub fn partage(&mut self, frame: u64) -> u32 {
        let compte = self.proprietaires.entry(frame).or_insert(1);
        *compte = compte.checked_add(1).expect("cow: debordement du compte de proprietaires");
        self.partages += 1;
        *compte
    }

    /// Un espace cesse de posseder `frame`.
    pub fn rend(&mut self, frame: u64) -> Rendu {
        match self.proprietaires.get_mut(&frame) {
            None => Rendu::Liberer,
            Some(compte) => {
                debug_assert!(*compte >= 2, "cow: frame en table avec un seul proprietaire");
                *compte -= 1;
                if *compte <= 1 {
                    self.proprietaires.remove(&frame);
                    self.liberees_partagees += 1;
                }
                Rendu::Garder
            }
        }
    }

    /// Nombre d'espaces qui possedent `frame`, pour une frame possedee par
    /// l'appelant (1 si elle n'est pas en table).
    pub fn proprietaires(&self, frame: u64) -> u32 {
        self.proprietaires.get(&frame).copied().unwrap_or(1)
    }

    /// `frame` est-elle possedee par un autre espace que l'appelant ?
    pub fn est_partagee(&self, frame: u64) -> bool {
        self.proprietaires.contains_key(&frame)
    }

    /// Frames actuellement partagees.
    pub fn frames_partagees(&self) -> usize {
        self.proprietaires.len()
    }

    /// (partages cumules, frames redevenues exclusives)
    pub fn cumuls(&self) -> (u64, u64) {
        (self.partages, self.liberees_partagees)
    }
}
