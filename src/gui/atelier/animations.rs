//! Registre d'animations Atelier, PUR (BOUCHAUD_ATELIER_ANIMATIONS_V1).
//!
//! # Ce que ce module est, et ce qu'il n'est pas
//!
//! Un registre BORNE d'animations datees sur l'horloge monotone : qui anime,
//! depuis quand, pour combien de temps, et quelle zone l'ecran doit recomposer.
//! Il ne dessine rien et ne connait ni le framebuffer ni l'allocateur : le
//! harnais hote l'inclut tel quel.
//!
//! Il n'est PAS le calcul des degats (`gui::transition`) : celui-ci dit quels
//! pixels une transition DISCRETE invalide. Ici on dit, a chaque trame d'une
//! transition CONTINUE, quelle zone la trame doit recouvrir -- et le
//! compositeur la verse dans les memes `Degats` que le reste.
//!
//! # Les trois regles qui empechent une animation de couter au repos
//!
//! 1. Un registre vide n'est pas `actif` : la boucle du bureau ne demande
//!    alors aucune trame de plus qu'avant (le test hote le verifie).
//! 2. Une animation finie est RETIREE par `termine`, qui rend sa derniere zone
//!    pour une ultime trame propre : aucune ne survit a sa duree.
//! 3. La capacite est fixe (`CAPACITE`). Lancer sur un registre plein evince la
//!    plus ancienne et la RENDS a l'appelant, qui recompose sa zone et libere
//!    ce qu'elle portait : une rafale d'ouvertures ne fait rien fuir.

use super::dock::Boite;
use super::mouvement::{applique, Courbe, PLEIN};

/// Nombre d'animations simultanees au plus.
pub const CAPACITE: usize = 8;

/// `debut_ms` d'une animation lancee mais pas encore montree.
///
/// Une animation ne commence pas quand on la LANCE, mais a la premiere trame
/// qui la COMPOSE. Sous TCG, la premiere trame apres l'ouverture d'une fenetre
/// (plein ecran, contenu neuf) coute pres de 100 ms : datee de la creation,
/// une ouverture de 200 ms etait a moitie finie avant d'etre vue, et la
/// mesure n'en montrait qu'une trame.
pub const EN_ATTENTE: u64 = u64::MAX;

/// Reglage utilisateur du mouvement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Fondu, translation et echelle.
    Normal,
    /// Fondu seul, plus court : aucun deplacement a l'ecran.
    Reduit,
    /// Aucune animation : l'etat final s'affiche a la trame suivante.
    Aucun,
}

/// Ce qui est anime.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nature {
    /// Une fenetre apparait : opacite 0 -> 1, monte de `DECALAGE_OUVERTURE`.
    Ouverture,
    /// Une fenetre disparait : son INSTANTANE (copie de pixels, jamais sa
    /// surface) s'efface et se reduit legerement.
    Fermeture,
    /// Une cellule du dock prend le survol : voile et levee de l'icone.
    Survol,
}

/// Une animation en cours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Animation {
    /// Identifiant de ce qui est anime (fenetre ou instantane).
    pub cle: u64,
    pub nature: Nature,
    pub debut_ms: u64,
    pub duree_ms: u32,
    /// Opacite de depart d'une fermeture (une fenetre fermee en cours
    /// d'ouverture part de l'opacite qu'elle avait, sans saut).
    pub depart: u8,
    /// Zone que la DERNIERE trame a recouverte. La trame suivante recouvre
    /// celle-ci ET la nouvelle : une fenetre deplacee pendant son ouverture ne
    /// laisse pas de bande derriere elle.
    pub bornes: Boite,
}

impl Animation {
    pub const fn fin_ms(&self) -> u64 {
        self.debut_ms.saturating_add(self.duree_ms as u64)
    }

    /// Avancement brut, 0..=PLEIN.
    pub fn avancement(&self, maintenant_ms: u64) -> u32 {
        if self.debut_ms == EN_ATTENTE {
            return if self.duree_ms == 0 { PLEIN } else { 0 };
        }
        if self.duree_ms == 0 || maintenant_ms >= self.fin_ms() {
            return PLEIN;
        }
        let ecoule = maintenant_ms.saturating_sub(self.debut_ms);
        (ecoule * PLEIN as u64 / self.duree_ms as u64) as u32
    }

    /// Avancement apres la courbe de la nature.
    pub fn progression(&self, maintenant_ms: u64) -> u32 {
        applique(courbe(self.nature), self.avancement(maintenant_ms))
    }
}

/// La courbe de chaque nature : une ouverture decelere (elle arrive), une
/// fermeture accelere puis se pose (elle s'en va sans trainer).
pub const fn courbe(nature: Nature) -> Courbe {
    match nature {
        Nature::Ouverture => Courbe::SortieCubique,
        Nature::Fermeture => Courbe::EntreeSortieCubique,
        Nature::Survol => Courbe::SortieCubique,
    }
}

/// Duree d'une nature dans un mode. Zero veut dire « pas d'animation ».
pub const fn duree_ms(mode: Mode, nature: Nature) -> u32 {
    match (mode, nature) {
        (Mode::Normal, Nature::Ouverture) => 200,
        (Mode::Normal, Nature::Fermeture) => 150,
        (Mode::Reduit, Nature::Ouverture) => 110,
        (Mode::Reduit, Nature::Fermeture) => 90,
        (Mode::Normal, Nature::Survol) => 120,
        (Mode::Reduit, Nature::Survol) => 80,
        (Mode::Aucun, _) => 0,
    }
}

/// Translation verticale d'une ouverture, en pixels, au depart.
pub const DECALAGE_OUVERTURE: i32 = 10;
/// Reduction d'une fermeture, en pour mille, a l'arrivee.
pub const REDUCTION_FERMETURE: u32 = 40;

/// Decalage vers le bas d'une fenetre qui s'ouvre, a la progression `p`.
pub const fn decalage(mode: Mode, p: u32) -> i32 {
    match mode {
        Mode::Normal => {
            let p = if p > PLEIN { PLEIN } else { p };
            (DECALAGE_OUVERTURE as u32 * (PLEIN - p) / PLEIN) as i32
        }
        _ => 0,
    }
}

/// Echelle (pour mille) d'un instantane qui se ferme, a la progression `p`.
pub const fn echelle(mode: Mode, p: u32) -> u32 {
    match mode {
        Mode::Normal => {
            let p = if p > PLEIN { PLEIN } else { p };
            PLEIN - REDUCTION_FERMETURE * p / PLEIN
        }
        _ => PLEIN,
    }
}

/// Opacite (0..=255) a la progression `p`.
pub const fn opacite(nature: Nature, depart: u8, p: u32) -> u8 {
    let p = if p > PLEIN { PLEIN } else { p };
    match nature {
        Nature::Ouverture => (255 * p / PLEIN) as u8,
        Nature::Fermeture => (depart as u32 * (PLEIN - p) / PLEIN) as u8,
        Nature::Survol => (255 * p / PLEIN) as u8,
    }
}

/// Union de deux boites ; une boite vide est neutre.
pub const fn union(a: Boite, b: Boite) -> Boite {
    if a.w <= 0 || a.h <= 0 {
        return b;
    }
    if b.w <= 0 || b.h <= 0 {
        return a;
    }
    let x0 = if a.x < b.x { a.x } else { b.x };
    let y0 = if a.y < b.y { a.y } else { b.y };
    let ax1 = a.x + a.w;
    let bx1 = b.x + b.w;
    let ay1 = a.y + a.h;
    let by1 = b.y + b.h;
    let x1 = if ax1 > bx1 { ax1 } else { bx1 };
    let y1 = if ay1 > by1 { ay1 } else { by1 };
    Boite { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// Le registre. Aucune allocation : `CAPACITE` cases.
#[derive(Clone, Copy, Debug)]
pub struct Registre {
    cases: [Option<Animation>; CAPACITE],
}

impl Registre {
    pub const fn neuf() -> Self {
        Self { cases: [None; CAPACITE] }
    }

    /// Y a-t-il une trame a demander ?
    pub fn actif(&self) -> bool {
        self.cases.iter().any(|c| c.is_some())
    }

    pub fn nombre(&self) -> usize {
        self.cases.iter().filter(|c| c.is_some()).count()
    }

    /// Lance `a`. Remplace une animation de meme cle et de meme nature. Sur un
    /// registre plein, evince la plus ancienne et la rend : l'appelant doit
    /// recomposer sa zone et liberer ce qu'elle portait.
    pub fn lance(&mut self, a: Animation) -> Option<Animation> {
        if let Some(c) = self.cases.iter_mut()
            .find(|c| matches!(c, Some(x) if x.cle == a.cle && x.nature == a.nature))
        {
            let ancienne = c.take();
            *c = Some(Animation { bornes: union(a.bornes, ancienne.map_or(a.bornes, |x| x.bornes)), ..a });
            return None;
        }
        if let Some(c) = self.cases.iter_mut().find(|c| c.is_none()) {
            *c = Some(a);
            return None;
        }
        let (rang, _) = self.cases.iter().enumerate()
            .min_by_key(|(_, c)| c.map_or(u64::MAX, |x| x.debut_ms))?;
        let evincee = self.cases[rang].take();
        self.cases[rang] = Some(a);
        evincee
    }

    /// L'animation de `cle` et `nature`, si elle est en cours.
    pub fn cherche(&self, cle: u64, nature: Nature) -> Option<Animation> {
        self.cases.iter().flatten().copied().find(|x| x.cle == cle && x.nature == nature)
    }

    /// Retire l'animation de `cle` (toutes natures) et rend la premiere trouvee.
    pub fn annule(&mut self, cle: u64, nature: Nature) -> Option<Animation> {
        let c = self.cases.iter_mut()
            .find(|c| matches!(c, Some(x) if x.cle == cle && x.nature == nature))?;
        c.take()
    }

    /// Note la zone recouverte par la trame courante, et rend celle a
    /// invalider : l'ancienne ET la nouvelle.
    pub fn recouvre(&mut self, cle: u64, nature: Nature, zone: Boite) -> Option<Boite> {
        let a = self.cases.iter_mut().flatten()
            .find(|x| x.cle == cle && x.nature == nature)?;
        let total = union(a.bornes, zone);
        a.bornes = zone;
        Some(total)
    }

    /// Retire les animations finies a `maintenant_ms`, en rendant chacune a
    /// `f` (derniere zone a recomposer, ressources a liberer).
    pub fn termine(&mut self, maintenant_ms: u64, mut f: impl FnMut(Animation)) {
        for c in self.cases.iter_mut() {
            if let Some(a) = *c {
                if maintenant_ms >= a.fin_ms() {
                    *c = None;
                    f(a);
                }
            }
        }
    }

    /// Date de `maintenant_ms` chaque animation encore en attente : appele
    /// au debut d'une trame composee.
    pub fn demarre(&mut self, maintenant_ms: u64) {
        for a in self.cases.iter_mut().flatten() {
            if a.debut_ms == EN_ATTENTE {
                a.debut_ms = maintenant_ms;
            }
        }
    }

    /// Les animations en cours.
    pub fn en_cours(&self) -> impl Iterator<Item = &Animation> {
        self.cases.iter().flatten()
    }
}
