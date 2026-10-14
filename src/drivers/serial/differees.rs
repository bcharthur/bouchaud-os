//! Les lignes qui n'ont pas obtenu le jeton serie, rangees entieres
//! (BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1).
//!
//! Un anneau par coeur. Un seul ECRIVAIN : le coeur proprietaire, interruptions
//! masquees (le repli se decide dans la course masquee de `sous_jeton`) --
//! rien ne peut l'interrompre au milieu d'une ligne. Un seul LECTEUR a la
//! fois : le detenteur du jeton, qui vide les anneaux AVANT sa propre ligne
//! (l'ordre d'un ecrivain est garde : sa ligne suivante ne peut pas doubler
//! celle qui attend). Chaque enregistrement : longueur (2 octets) puis la
//! ligne ; une ligne qui ne tient pas est refusee entiere.
//!
//! Module pur (atomiques et `UnsafeCell`) : `tools/serie/test_differees.rs`.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

pub struct Anneau<const N: usize> {
    /// Position de lecture (croissante), ecrite par le lecteur seul.
    tete: AtomicUsize,
    /// Position d'ecriture (croissante), ecrite par l'ecrivain seul.
    queue: AtomicUsize,
    octets: UnsafeCell<[u8; N]>,
}

// SAFETY : un ecrivain unique (coeur proprietaire, interruptions masquees)
// et un lecteur unique a la fois (detenteur du jeton) ; chacun ne touche que
// les octets que l'autre a publies (Release/Acquire sur `queue` et `tete`).
unsafe impl<const N: usize> Sync for Anneau<N> {}

impl<const N: usize> Anneau<N> {
    pub const fn neuf() -> Self {
        Self { tete: AtomicUsize::new(0), queue: AtomicUsize::new(0), octets: UnsafeCell::new([0; N]) }
    }

    /// Range une ligne ENTIERE. Rend `false` -- rien d'ecrit -- si elle ne
    /// tient pas. Ecrivain unique.
    pub fn range(&self, ligne: &[u8]) -> bool {
        if ligne.is_empty() {
            return true;
        }
        if ligne.len() > u16::MAX as usize {
            return false;
        }
        let q = self.queue.load(Ordering::Relaxed);
        let t = self.tete.load(Ordering::Acquire);
        let libre = N - (q - t);
        if libre < 2 + ligne.len() {
            return false;
        }
        let n = ligne.len() as u16;
        let base = self.octets.get() as *mut u8;
        let pose = |k: usize, b: u8| unsafe { *base.add((q + k) % N) = b };
        pose(0, n as u8);
        pose(1, (n >> 8) as u8);
        for (k, &b) in ligne.iter().enumerate() {
            pose(2 + k, b);
        }
        self.queue.store(q + 2 + ligne.len(), Ordering::Release);
        true
    }

    /// Vide les lignes rangees, dans l'ordre, chacune entiere : `emet` recoit
    /// une ligne en un ou deux morceaux (l'anneau qui tourne). Rend le nombre
    /// de lignes. Lecteur unique.
    pub fn vide(&self, mut emet: impl FnMut(&[u8])) -> usize {
        let mut t = self.tete.load(Ordering::Relaxed);
        let q = self.queue.load(Ordering::Acquire);
        let base = self.octets.get() as *const u8;
        let lis = |k: usize| unsafe { *base.add(k % N) };
        let mut lignes = 0;
        while t < q {
            let n = lis(t) as usize | (lis(t + 1) as usize) << 8;
            let debut = (t + 2) % N;
            let premier = n.min(N - debut);
            // SAFETY : [debut, debut + premier) et [0, n - premier) sont dans
            // l'anneau, publies par l'ecrivain avant `queue`.
            unsafe {
                emet(core::slice::from_raw_parts(base.add(debut), premier));
                if n > premier {
                    emet(core::slice::from_raw_parts(base, n - premier));
                }
            }
            t += 2 + n;
            lignes += 1;
        }
        self.tete.store(t, Ordering::Release);
        lignes
    }

    pub fn est_vide(&self) -> bool {
        self.tete.load(Ordering::Acquire) == self.queue.load(Ordering::Acquire)
    }
}
