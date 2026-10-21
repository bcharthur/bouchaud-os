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

    /// Octets occupes (enregistrements et leurs longueurs).
    pub fn occupation(&self) -> usize {
        self.queue.load(Ordering::Acquire) - self.tete.load(Ordering::Acquire)
    }

    /// BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : range UN enregistrement de `total`
    /// octets ecrit en plusieurs morceaux (prefixe, puis contenu...). `ecrit`
    /// recoit une fonction de pose a appeler dans l'ordre. L'enregistrement
    /// n'est PUBLIE que si exactement `total` octets ont ete poses : une ligne
    /// est entiere ou absente. Rend `false` -- rien de publie -- si elle ne
    /// tient pas ou si le compte ne tombe pas juste. Ecrivain unique.
    pub fn range_morceaux(&self, total: usize, ecrit: impl FnOnce(&mut dyn FnMut(&[u8]))) -> bool {
        if total == 0 {
            return true;
        }
        if total > u16::MAX as usize {
            return false;
        }
        let q = self.queue.load(Ordering::Relaxed);
        let t = self.tete.load(Ordering::Acquire);
        if N - (q - t) < 2 + total {
            return false;
        }
        let base = self.octets.get() as *mut u8;
        let n = total as u16;
        // SAFETY : ces positions sont libres (`q - t + 2 + total <= N`), le
        // lecteur ne les lit qu'apres la publication de `queue`.
        unsafe {
            *base.add(q % N) = n as u8;
            *base.add((q + 1) % N) = (n >> 8) as u8;
        }
        let mut pose = 0usize;
        ecrit(&mut |morceau: &[u8]| {
            for &b in morceau {
                if pose < total {
                    unsafe { *base.add((q + 2 + pose) % N) = b; }
                }
                pose += 1;
            }
        });
        if pose != total {
            return false;
        }
        self.queue.store(q + 2 + total, Ordering::Release);
        true
    }

    /// BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : l'enregistrement de tete, SANS le
    /// retirer -- `(position, longueur)`. Avec `lis` et `retire`, un
    /// enregistrement se lit en plusieurs fois, par des lecteurs successifs
    /// (un seul a la fois). Lecteur unique.
    pub fn premier(&self) -> Option<(usize, usize)> {
        let t = self.tete.load(Ordering::Relaxed);
        let q = self.queue.load(Ordering::Acquire);
        if t >= q {
            return None;
        }
        let base = self.octets.get() as *const u8;
        // SAFETY : [t, t + 2) est publie (t < q).
        let n = unsafe { *base.add(t % N) as usize | (*base.add((t + 1) % N) as usize) << 8 };
        Some((t, n))
    }

    /// Les octets `[de, a)` de l'enregistrement en `t` (rendu par `premier`),
    /// en un ou deux morceaux. Lecteur unique.
    pub fn lis(&self, t: usize, de: usize, a: usize, mut emet: impl FnMut(&[u8])) {
        if de >= a {
            return;
        }
        let base = self.octets.get() as *const u8;
        let debut = (t + 2 + de) % N;
        let n = a - de;
        let premier = n.min(N - debut);
        // SAFETY : [t + 2 + de, t + 2 + a) est dans l'enregistrement publie.
        unsafe {
            emet(core::slice::from_raw_parts(base.add(debut), premier));
            if n > premier {
                emet(core::slice::from_raw_parts(base, n - premier));
            }
        }
    }

    /// Retire l'enregistrement en `t`, de `n` octets (rendu par `premier`).
    /// Lecteur unique.
    pub fn retire(&self, t: usize, n: usize) {
        self.tete.store(t + 2 + n, Ordering::Release);
    }

    /// Vide UN enregistrement, s'il y en a un : `emet` le recoit en un ou deux
    /// morceaux. Lecteur unique.
    pub fn vide_un(&self, mut emet: impl FnMut(&[u8])) -> bool {
        let t = self.tete.load(Ordering::Relaxed);
        let q = self.queue.load(Ordering::Acquire);
        if t >= q {
            return false;
        }
        let base = self.octets.get() as *const u8;
        let lis = |k: usize| unsafe { *base.add(k % N) };
        let n = lis(t) as usize | (lis(t + 1) as usize) << 8;
        let debut = (t + 2) % N;
        let premier = n.min(N - debut);
        // SAFETY : comme `vide`.
        unsafe {
            emet(core::slice::from_raw_parts(base.add(debut), premier));
            if n > premier {
                emet(core::slice::from_raw_parts(base, n - premier));
            }
        }
        self.tete.store(t + 2 + n, Ordering::Release);
        true
    }
}

/// BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : longueur de `octets` une fois un prefixe de
/// `prefixe` octets pose devant chaque debut de ligne non vide. `debut_ligne` :
/// le dernier octet range avant celui-ci etait-il un saut de ligne ?
pub fn longueur_prefixee(octets: &[u8], prefixe: usize, mut debut_ligne: bool) -> usize {
    let mut n = octets.len();
    for &b in octets {
        if debut_ligne && b != b'\n' {
            n += prefixe;
        }
        debut_ligne = b == b'\n';
    }
    n
}

/// Pose `octets`, `prefixe` devant chaque debut de ligne non vide, dans
/// l'ordre ; rend le nouvel etat « debut de ligne ». Le meme parcours que
/// `longueur_prefixee` : `range_morceaux` recoit exactement ce qui a ete compte.
pub fn pose_prefixee(octets: &[u8], prefixe: &[u8], mut debut_ligne: bool, pose: &mut dyn FnMut(&[u8])) -> bool {
    let mut depuis = 0usize;
    for (i, &b) in octets.iter().enumerate() {
        if debut_ligne && b != b'\n' {
            pose(&octets[depuis..i]);
            pose(prefixe);
            depuis = i;
        }
        debut_ligne = b == b'\n';
    }
    pose(&octets[depuis..]);
    debut_ligne
}
