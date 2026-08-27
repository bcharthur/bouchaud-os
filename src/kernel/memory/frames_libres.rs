//! Quelles frames physiques sont actuellement dans la liste libre.
//!
//! # Le defaut que ce module supprime
//!
//! `free_frame` verifiait le double `free` en parcourant TOUTE la liste libre :
//!
//! ```text
//! let mut cursor = f.freed_head;
//! while let Some(free) = cursor {
//!     assert!(free != phys, "vmm: double free frame");
//!     let next = unsafe { *(phys_to_virt(free) as *const u64) };   // page FROIDE
//!     cursor = ...;
//! }
//! ```
//!
//! Chaque pas dereference une page physique qui n'est dans aucun cache : la
//! liste libre est chainee DANS les frames liberees elles-memes. Le cout d'un
//! `free` est donc `O(frames libres)` defauts de cache, et la liste libre
//! grandit avec l'age de la session.
//!
//! Liberer R frames coute alors `O(R x L)`. Pour un `madvise(DONTNEED)` de
//! 16 Mio (R = 4096) sur une machine dont la liste libre porte 100 000 frames,
//! cela fait 4 x 10^8 lectures memoire froides — des dizaines de secondes sous
//! TCG, **le gros verrou tenu**. C'est ce qui a produit une tenue de 15 s
//! attribuee a `madvise`, et c'est aussi pourquoi la lenteur EMPIRE avec la
//! duree de la session : la liste libre ne cesse de s'allonger.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne supprime pas l'assertion. Un `free` d'une frame deja libre reste une
//! erreur fatale — c'est meme desormais une erreur DETECTEE PLUS TOT, puisque
//! le test devient exact au lieu de dependre de la position dans la liste, et
//! qu'un `free` d'une frame etrangere aux regions est detecte lui aussi.
//!
//! Le test passe simplement de `O(L)` lectures froides a un bit.
//!
//! # Cout memoire
//!
//! Un bit par frame de RAM utilisable : 32 Kio pour 1 Gio de RAM, 128 Kio pour
//! 4 Gio. C'est le prix d'un `free` en temps constant.
//!
//! Module pur : ni verrou, ni allocateur de frames, ni MMU. Il se teste sur
//! l'hote.

use alloc::vec::Vec;

/// Taille d'une page physique. Doit rester egale a `vmm::PAGE_SIZE`.
pub const TAILLE_PAGE: u64 = 4096;

/// Ensemble de frames, une par bit.
pub struct FramesLibres {
    /// Adresse physique correspondant au bit 0.
    base: u64,
    /// Une frame par bit, 64 frames par mot.
    mots: Vec<u64>,
    /// Nombre de bits a 1. Evite de recompter pour un diagnostic.
    compte: usize,
}

impl FramesLibres {
    pub const fn neuf() -> Self {
        Self { base: 0, mots: Vec::new(), compte: 0 }
    }

    /// Etend la couverture a `[debut, fin)`.
    ///
    /// Appele par `add_region`, donc uniquement a l'amorcage, avant qu'une
    /// seule frame ait ete liberee. Une region qui arrive SOUS la base actuelle
    /// impose de rebaser : c'est admis tant que l'ensemble est vide, et c'est
    /// verifie plutot que suppose.
    pub fn couvre(&mut self, debut: u64, fin: u64) {
        if fin <= debut {
            return;
        }
        let debut = debut & !(TAILLE_PAGE - 1);
        let fin = (fin + TAILLE_PAGE - 1) & !(TAILLE_PAGE - 1);

        if self.mots.is_empty() {
            self.base = debut;
            self.mots = alloc::vec![0u64; mots_pour(fin - debut)];
            return;
        }

        if debut < self.base {
            assert!(
                self.compte == 0,
                "frames_libres: rebasage avec {} frames deja libres",
                self.compte,
            );
            let ancienne_fin = self.base + (self.mots.len() as u64) * 64 * TAILLE_PAGE;
            self.base = debut;
            self.mots = alloc::vec![0u64; mots_pour(ancienne_fin.max(fin) - debut)];
            return;
        }

        let besoin = mots_pour(fin - self.base);
        if besoin > self.mots.len() {
            self.mots.resize(besoin, 0);
        }
    }

    /// Position du bit de `phys`, ou `None` si l'adresse est hors couverture.
    fn position(&self, phys: u64) -> Option<(usize, u64)> {
        if phys < self.base {
            return None;
        }
        let rang = (phys - self.base) / TAILLE_PAGE;
        let mot = (rang / 64) as usize;
        if mot >= self.mots.len() {
            return None;
        }
        Some((mot, 1u64 << (rang % 64)))
    }

    /// Cette frame est-elle couverte par une region connue ?
    pub fn couverte(&self, phys: u64) -> bool {
        self.position(phys).is_some()
    }

    /// Cette frame est-elle actuellement dans la liste libre ?
    pub fn est_libre(&self, phys: u64) -> bool {
        match self.position(phys) {
            Some((mot, bit)) => self.mots[mot] & bit != 0,
            None => false,
        }
    }

    /// Marque la frame comme libre. Rend `false` si elle l'etait deja —
    /// c'est-a-dire un double `free`.
    pub fn marque_libre(&mut self, phys: u64) -> bool {
        let Some((mot, bit)) = self.position(phys) else { return false };
        if self.mots[mot] & bit != 0 {
            return false;
        }
        self.mots[mot] |= bit;
        self.compte += 1;
        true
    }

    /// Marque la frame comme occupee. Rend `false` si elle ne figurait pas
    /// dans la liste libre — c'est-a-dire une allocation incoherente.
    pub fn marque_occupee(&mut self, phys: u64) -> bool {
        let Some((mot, bit)) = self.position(phys) else { return false };
        if self.mots[mot] & bit == 0 {
            return false;
        }
        self.mots[mot] &= !bit;
        self.compte -= 1;
        true
    }

    /// Nombre de frames actuellement libres.
    pub fn compte(&self) -> usize {
        self.compte
    }

    /// Frames couvertes par le bitmap.
    pub fn capacite(&self) -> usize {
        self.mots.len() * 64
    }
}

fn mots_pour(octets: u64) -> usize {
    let frames = octets.div_ceil(TAILLE_PAGE);
    (frames.div_ceil(64)) as usize
}
