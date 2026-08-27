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

/// Deux bitmaps : ce qui EXISTE, et ce qui est LIBRE.
///
/// # Pourquoi deux et non un
///
/// Les regions physiques de RAM ne sont pas contigues. Une machine typique
/// donne quelque chose comme :
///
/// ```text
///     region A [0x0010_0000, 0x0009_0000)
///     TROU     reserve BIOS / ACPI / MMIO
///     region B [0x0100_0000, 0x8000_0000)
/// ```
///
/// Un bitmap unique indexe par `(phys - base) / TAILLE_PAGE` represente
/// l'ENVELOPPE `A..B`, pas l'union `A U B`. Une adresse dans le TROU y a un
/// index valide, et `couverte()` repondait donc « oui » pour une frame qui
/// n'existe pas.
///
/// Ce n'etait pas theorique : `free_frame` affirme
/// « free d'une frame hors des regions de l'allocateur ». Une assertion qui
/// accepte le trou n'affirme rien.
///
/// `valides` porte donc l'union EXACTE des regions, posee une fois a
/// l'amorcage. `libres` porte l'appartenance a la liste libre, et un bit ne
/// peut y etre pose que si le meme bit est valide.
///
/// Le cout est un second bit par frame : 64 Kio par Gio de RAM au lieu de 32.
/// L'alternative — balayer `FrameAllocator.regions` — serait exacte aussi,
/// mais `regions` est MUTEE par le chemin d'allocation par avancee
/// (`region.0 += PAGE_SIZE`), si bien qu'une frame distribuee par ce chemin
/// tombe hors de sa propre region des qu'elle est allouee. Verifier contre
/// `regions` rejetterait donc la liberation de toute frame jamais allouee par
/// avancee. Le bitmap, lui, garde les bornes d'origine.
pub struct FramesLibres {
    /// Adresse physique correspondant au bit 0 des deux bitmaps.
    base: u64,
    /// Frames qui EXISTENT : union exacte des regions declarees.
    valides: Vec<u64>,
    /// Frames actuellement dans la liste libre. Sous-ensemble de `valides`.
    libres: Vec<u64>,
    /// Nombre de bits a 1 dans `libres`. Evite de recompter pour un diagnostic.
    compte: usize,
}

impl FramesLibres {
    pub const fn neuf() -> Self {
        Self { base: 0, valides: Vec::new(), libres: Vec::new(), compte: 0 }
    }

    /// Etend la couverture a `[debut, fin)`.
    ///
    /// Appele par `add_region`, donc uniquement a l'amorcage, avant qu'une
    /// seule frame ait ete liberee. Une region qui arrive SOUS la base actuelle
    /// impose de rebaser : c'est admis tant que l'ensemble est vide, et c'est
    /// verifie plutot que suppose.
    pub fn couvre(&mut self, debut: u64, fin: u64) {
        // Une frame n'existe que si elle tient ENTIEREMENT dans la region :
        // debut arrondi vers le haut, fin vers le bas. Arrondir dans l'autre
        // sens declarerait valide une page a cheval sur le trou voisin.
        let debut = (debut + TAILLE_PAGE - 1) & !(TAILLE_PAGE - 1);
        let fin = fin & !(TAILLE_PAGE - 1);
        if fin <= debut {
            return;
        }

        if self.valides.is_empty() {
            self.base = debut;
            self.valides = alloc::vec![0u64; mots_pour(fin - debut)];
            self.libres = alloc::vec![0u64; mots_pour(fin - debut)];
        } else if debut < self.base {
            // Rebasage : admis tant qu'aucune frame n'a ete liberee, ce qui
            // est le cas a l'amorcage. Verifie plutot que suppose.
            assert!(
                self.compte == 0,
                "frames_libres: rebasage avec {} frames deja libres",
                self.compte,
            );
            let ancienne_base = self.base;
            let anciennes = core::mem::take(&mut self.valides);
            let ancienne_fin =
                ancienne_base + (anciennes.len() as u64) * 64 * TAILLE_PAGE;
            self.base = debut;
            let taille = mots_pour(ancienne_fin.max(fin) - debut);
            self.valides = alloc::vec![0u64; taille];
            self.libres = alloc::vec![0u64; taille];
            // Reporter l'ancienne validite a la nouvelle base : un rebasage ne
            // doit pas faire disparaitre les regions deja declarees.
            for (mot, bits) in anciennes.iter().copied().enumerate() {
                if bits == 0 {
                    continue;
                }
                for bit in 0..64u64 {
                    if bits & (1u64 << bit) != 0 {
                        let phys =
                            ancienne_base + ((mot as u64) * 64 + bit) * TAILLE_PAGE;
                        self.pose_valide(phys);
                    }
                }
            }
        } else {
            let besoin = mots_pour(fin - self.base);
            if besoin > self.valides.len() {
                self.valides.resize(besoin, 0);
                self.libres.resize(besoin, 0);
            }
        }

        let mut phys = debut;
        while phys < fin {
            self.pose_valide(phys);
            phys += TAILLE_PAGE;
        }
    }

    fn pose_valide(&mut self, phys: u64) {
        if let Some((mot, bit)) = self.position(phys) {
            self.valides[mot] |= bit;
        }
    }

    /// Index du bit de `phys` dans l'ENVELOPPE, sans rien dire de sa validite.
    fn position(&self, phys: u64) -> Option<(usize, u64)> {
        if phys < self.base {
            return None;
        }
        let rang = (phys - self.base) / TAILLE_PAGE;
        let mot = (rang / 64) as usize;
        if mot >= self.valides.len() {
            return None;
        }
        Some((mot, 1u64 << (rang % 64)))
    }

    /// Position d'une frame qui EXISTE reellement.
    ///
    /// C'est la seule porte d'entree des trois operations : une adresse dans un
    /// trou entre deux regions n'en obtient pas.
    fn position_valide(&self, phys: u64) -> Option<(usize, u64)> {
        let (mot, bit) = self.position(phys)?;
        if self.valides[mot] & bit == 0 {
            return None;
        }
        Some((mot, bit))
    }

    /// Cette frame appartient-elle a une region declaree ?
    ///
    /// Union EXACTE des regions, pas leur enveloppe : une adresse situee entre
    /// deux regions rend `false`.
    pub fn couverte(&self, phys: u64) -> bool {
        self.position_valide(phys).is_some()
    }

    /// Cette frame est-elle actuellement dans la liste libre ?
    pub fn est_libre(&self, phys: u64) -> bool {
        match self.position_valide(phys) {
            Some((mot, bit)) => self.libres[mot] & bit != 0,
            None => false,
        }
    }

    /// Marque la frame comme libre. Rend `false` si elle l'etait deja —
    /// c'est-a-dire un double `free`.
    pub fn marque_libre(&mut self, phys: u64) -> bool {
        let Some((mot, bit)) = self.position_valide(phys) else { return false };
        if self.libres[mot] & bit != 0 {
            return false;
        }
        self.libres[mot] |= bit;
        self.compte += 1;
        true
    }

    /// Marque la frame comme occupee. Rend `false` si elle ne figurait pas
    /// dans la liste libre — c'est-a-dire une allocation incoherente.
    pub fn marque_occupee(&mut self, phys: u64) -> bool {
        let Some((mot, bit)) = self.position_valide(phys) else { return false };
        if self.libres[mot] & bit == 0 {
            return false;
        }
        self.libres[mot] &= !bit;
        self.compte -= 1;
        true
    }

    /// Nombre de frames actuellement libres.
    pub fn compte(&self) -> usize {
        self.compte
    }

    /// Frames que l'ENVELOPPE peut indexer, trous compris.
    pub fn capacite(&self) -> usize {
        self.valides.len() * 64
    }

    /// Frames qui existent reellement : somme des regions declarees.
    pub fn frames_valides(&self) -> usize {
        self.valides.iter().map(|mot| mot.count_ones() as usize).sum()
    }
}

fn mots_pour(octets: u64) -> usize {
    let frames = octets.div_ceil(TAILLE_PAGE);
    (frames.div_ceil(64)) as usize
}
