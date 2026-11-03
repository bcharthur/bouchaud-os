//! Mouvement Atelier (BOUCHAUD_ATELIER_MOUVEMENT_V1).
//!
//! Pur, entier, sans flottant : une animation ne doit pas couter plus que ce
//! qu'elle montre. Le temps est en millisecondes monotones ; une progression
//! est en milliemes (0..=1000).
//!
//! # Le contrat
//!
//! Une animation est une [`Interpolation`] : une valeur de depart, une valeur
//! d'arrivee, un instant de debut, une duree et une courbe. Elle est FINIE
//! quand sa duree est ecoulee, et sa valeur est alors EXACTEMENT l'arrivee --
//! jamais « presque ». Le compositeur ne redemande une trame que tant qu'au
//! moins une interpolation n'est pas finie : un bureau immobile ne consomme
//! rien. (Branchement dans la boucle du compositeur : increment 2, voir
//! `docs/current/ATELIER_DESIGN_SYSTEM.md`.)

/// Milliemes : 1000 = 100 %.
pub const PLEIN: u32 = 1000;

/// Courbe d'acceleration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Courbe {
    /// Vitesse constante. Reserve aux indicateurs de progression.
    Lineaire,
    /// Depart vif, arrivee douce : la courbe par defaut des transitions
    /// d'interface (equivalent de `cubic-bezier(0.33, 1, 0.68, 1)`).
    SortieCubique,
    /// Symetrique : deplacements d'un etat stable a un autre.
    EntreeSortieCubique,
}

/// Applique la courbe a une progression `p` en milliemes.
pub const fn applique(courbe: Courbe, p: u32) -> u32 {
    let p = (if p > PLEIN { PLEIN } else { p }) as u64;
    let n = PLEIN as u64;
    let v = match courbe {
        Courbe::Lineaire => p,
        // 1 - (1 - p)^3
        Courbe::SortieCubique => {
            let q = n - p;
            n - q * q * q / (n * n)
        }
        // p < 1/2 : 4 p^3 ; sinon 1 - (-2p + 2)^3 / 2
        Courbe::EntreeSortieCubique => {
            if p * 2 < n {
                4 * p * p * p / (n * n)
            } else {
                let q = 2 * n - 2 * p;
                n - q * q * q / (2 * n * n)
            }
        }
    };
    v as u32
}

/// Une valeur qui va de `depart` a `arrivee` en `duree_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interpolation {
    pub depart: i32,
    pub arrivee: i32,
    pub debut_ms: u64,
    pub duree_ms: u32,
    pub courbe: Courbe,
}

impl Interpolation {
    pub const fn nouvelle(depart: i32, arrivee: i32, debut_ms: u64, duree_ms: u32,
        courbe: Courbe) -> Self {
        Self { depart, arrivee, debut_ms, duree_ms, courbe }
    }

    /// Progression brute en milliemes a l'instant `maintenant_ms`.
    pub const fn progression(&self, maintenant_ms: u64) -> u32 {
        if self.duree_ms == 0 || maintenant_ms >= self.debut_ms + self.duree_ms as u64 {
            return PLEIN;
        }
        if maintenant_ms <= self.debut_ms {
            return 0;
        }
        ((maintenant_ms - self.debut_ms) * PLEIN as u64 / self.duree_ms as u64) as u32
    }

    /// Valeur a l'instant `maintenant_ms`. Exactement `arrivee` une fois finie.
    pub const fn valeur(&self, maintenant_ms: u64) -> i32 {
        let p = self.progression(maintenant_ms);
        if p >= PLEIN {
            return self.arrivee;
        }
        let e = applique(self.courbe, p) as i64;
        let ecart = self.arrivee as i64 - self.depart as i64;
        (self.depart as i64 + ecart * e / PLEIN as i64) as i32
    }

    pub const fn finie(&self, maintenant_ms: u64) -> bool {
        self.progression(maintenant_ms) >= PLEIN
    }
}

/// Melange deux couleurs `0xRRGGBB` selon une progression en milliemes.
pub const fn melange_couleur(a: u32, b: u32, p: u32) -> u32 {
    let p = if p > PLEIN { PLEIN } else { p };
    (canal(a, b, 16, p) << 16) | (canal(a, b, 8, p) << 8) | canal(a, b, 0, p)
}

const fn canal(a: u32, b: u32, decalage: u32, p: u32) -> u32 {
    let ca = (a >> decalage) & 0xff;
    let cb = (b >> decalage) & 0xff;
    (ca * (PLEIN - p) + cb * p + PLEIN / 2) / PLEIN
}
