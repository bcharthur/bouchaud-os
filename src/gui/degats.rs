//! Politique de degats du bureau : regions fixes, provenance et metriques.
//!
//! Gate 1A remplace la boite englobante unique par une petite region sparse
//! sans allocation. Deux zones eloignees restent deux rectangles : le
//! compositeur ne repeint plus tout l'espace vide qui les separe.
//!
//! Le module reste de la geometrie pure et se teste sur l'hote.

use crate::gui::protocole::Rect;
use core::sync::atomic::{AtomicU64, Ordering};

// BOUCHAUD_GUI_DAMAGE_REGION_V2
pub const CAPACITE_REGIONS: usize = 16;

// Une fusion est acceptee si sa boite englobante coute au plus 25 % de pixels
// supplementaires par rapport aux deux rectangles separes.
const FUSION_NUMERATEUR: u64 = 5;
const FUSION_DENOMINATEUR: u64 = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Origine {
    PleinEcran,
    Fenetre,
    Curseur,
    Client,
    BarreTaches,
    /// Barre du HAUT : horloge, charge CPU, memoire, disque.
    ///
    /// Distincte de `BarreTaches` parce que ce sont deux barres differentes, a
    /// deux extremites de l'ecran. Les confondre a fige l'horloge a l'ecran
    /// pendant que le compositeur presentait consciencieusement la barre du bas.
    BarreHaute,
    Menu,
    Icone,
}

const NOMBRE_ORIGINES: usize = 8;
static DEGATS_PAR_ORIGINE: [AtomicU64; NOMBRE_ORIGINES] =
    [const { AtomicU64::new(0) }; NOMBRE_ORIGINES];

static PIXELS_PRESENTES: AtomicU64 = AtomicU64::new(0);
static RECTS_PRESENTES: AtomicU64 = AtomicU64::new(0);
static TRAMES_PRESENTEES: AtomicU64 = AtomicU64::new(0);

static PIXELS_DEMANDES: AtomicU64 = AtomicU64::new(0);
static PIXELS_BOITE_ENGLOBANTE: AtomicU64 = AtomicU64::new(0);
static FUSIONS_RECTS: AtomicU64 = AtomicU64::new(0);
static DEBORDEMENTS_REGION: AtomicU64 = AtomicU64::new(0);
/// Degats crees, toutes origines confondues. Deuxieme maillon de la chaine
/// `entree -> degat -> trame -> present -> LFB` (voir `gui::chaine`).
static DEGATS_TOTAL: AtomicU64 = AtomicU64::new(0);

impl Origine {
    fn index(self) -> usize {
        match self {
            Origine::PleinEcran => 0,
            Origine::Fenetre => 1,
            Origine::Curseur => 2,
            Origine::Client => 3,
            Origine::BarreTaches => 4,
            Origine::Menu => 5,
            Origine::Icone => 6,
            Origine::BarreHaute => 7,
        }
    }
}

#[inline]
fn aire(rect: Rect) -> u64 {
    (rect.largeur as u64).saturating_mul(rect.hauteur as u64)
}

#[inline]
fn fusion_rentable(a: Rect, b: Rect) -> bool {
    if a.vide() || b.vide() {
        return true;
    }
    let union = a.union(&b);
    let separes = aire(a).saturating_add(aire(b));
    let fusion = aire(union);

    fusion.saturating_mul(FUSION_DENOMINATEUR)
        <= separes.saturating_mul(FUSION_NUMERATEUR)
}

/// Ensemble de degats d'une trame.
///
/// Aucun heap : le bureau reste utilisable avant que l'allocateur ne devienne
/// une dependance de sa politique de rendu.
#[derive(Clone, Copy)]
pub struct Degats {
    regions: [Rect; CAPACITE_REGIONS],
    nombre: usize,
    ecran: Rect,
}

impl Degats {
    pub fn neuf(ecran: Rect) -> Self {
        Self {
            regions: [Rect::default(); CAPACITE_REGIONS],
            nombre: 0,
            ecran,
        }
    }

    /// Ajoute un degat, borne a l'ecran.
    pub fn ajoute(&mut self, origine: Origine, rect: Rect) {
        let rect = rect.intersecte(&self.ecran);
        if rect.vide() {
            return;
        }

        DEGATS_PAR_ORIGINE[origine.index()].fetch_add(1, Ordering::Relaxed);
        DEGATS_TOTAL.fetch_add(1, Ordering::Relaxed);
        PIXELS_DEMANDES.fetch_add(aire(rect), Ordering::Relaxed);
        self.insere(rect);
    }

    fn retire_index(&mut self, index: usize) {
        debug_assert!(index < self.nombre);
        self.nombre -= 1;
        self.regions[index] = self.regions[self.nombre];
        self.regions[self.nombre] = Rect::default();
    }

    fn insere(&mut self, mut rect: Rect) {
        // Fusion transitive : si la fusion avec A rend ensuite la fusion avec B
        // rentable, recommencer depuis le debut.
        let mut i = 0usize;
        while i < self.nombre {
            let existant = self.regions[i];
            if fusion_rentable(existant, rect) {
                rect = existant.union(&rect);
                self.retire_index(i);
                FUSIONS_RECTS.fetch_add(1, Ordering::Relaxed);
                i = 0;
                continue;
            }
            i += 1;
        }

        if self.nombre < CAPACITE_REGIONS {
            self.regions[self.nombre] = rect;
            self.nombre += 1;
            return;
        }

        // Region pleine : ne jamais perdre un pixel sale. Fusionner le nouveau
        // rectangle avec le slot qui cree la plus petite boite finale.
        DEBORDEMENTS_REGION.fetch_add(1, Ordering::Relaxed);
        let mut meilleur = 0usize;
        let mut meilleure_aire = u64::MAX;
        for index in 0..self.nombre {
            let candidate = self.regions[index].union(&rect);
            let candidate_aire = aire(candidate);
            if candidate_aire < meilleure_aire {
                meilleure_aire = candidate_aire;
                meilleur = index;
            }
        }
        self.regions[meilleur] = self.regions[meilleur].union(&rect);
        FUSIONS_RECTS.fetch_add(1, Ordering::Relaxed);
    }

    /// Plein ecran intentionnel : une seule region, pas 16 fragments.
    pub fn tout(&mut self) {
        DEGATS_PAR_ORIGINE[Origine::PleinEcran.index()].fetch_add(1, Ordering::Relaxed);
        DEGATS_TOTAL.fetch_add(1, Ordering::Relaxed);
        PIXELS_DEMANDES.fetch_add(aire(self.ecran), Ordering::Relaxed);
        self.regions = [Rect::default(); CAPACITE_REGIONS];
        self.regions[0] = self.ecran;
        self.nombre = 1;
    }

    /// Regions effectives a presenter.
    pub fn regions(&self) -> &[Rect] {
        &self.regions[..self.nombre]
    }

    pub fn nombre_regions(&self) -> usize {
        self.nombre
    }

    /// Somme des pixels des regions sparse. Les recouvrements rentables sont
    /// fusionnes a l'insertion, donc ce nombre mesure le travail de presentation.
    pub fn pixels_regions(&self) -> u64 {
        self.regions()
            .iter()
            .copied()
            .fold(0u64, |total, rect| total.saturating_add(aire(rect)))
    }

    /// Compatibilite/diagnostic : boite englobante qu'aurait utilisee Gate 0.
    pub fn region(&self) -> Rect {
        let mut boite = Rect::default();
        for rect in self.regions() {
            boite = boite.union(rect);
        }
        boite
    }

    pub fn vide(&self) -> bool {
        self.nombre == 0
    }

    pub fn efface(&mut self) {
        self.regions = [Rect::default(); CAPACITE_REGIONS];
        self.nombre = 0;
    }
}

/// Compteurs historiques : (par origine, trames, pixels presentes).
pub fn stats_degats() -> ([u64; NOMBRE_ORIGINES], u64, u64) {
    let mut par_origine = [0u64; NOMBRE_ORIGINES];
    for (index, compteur) in DEGATS_PAR_ORIGINE.iter().enumerate() {
        par_origine[index] = compteur.load(Ordering::Relaxed);
    }
    (
        par_origine,
        TRAMES_PRESENTEES.load(Ordering::Relaxed),
        PIXELS_PRESENTES.load(Ordering::Relaxed),
    )
}

/// Gate 1A : (rects presentes, pixels demandes, pixels boite Gate0,
/// fusions, debordements).
pub fn stats_regions() -> (u64, u64, u64, u64, u64) {
    (
        RECTS_PRESENTES.load(Ordering::Relaxed),
        PIXELS_DEMANDES.load(Ordering::Relaxed),
        PIXELS_BOITE_ENGLOBANTE.load(Ordering::Relaxed),
        FUSIONS_RECTS.load(Ordering::Relaxed),
        DEBORDEMENTS_REGION.load(Ordering::Relaxed),
    )
}

/// Une trame logique. La boite englobante est comptabilisee pour mesurer le
/// travail qu'aurait fait l'ancien moteur sur exactement la meme trame.
pub fn note_trame(degats: &Degats) {
    if degats.vide() {
        return;
    }
    TRAMES_PRESENTEES.fetch_add(1, Ordering::Relaxed);
    PIXELS_BOITE_ENGLOBANTE.fetch_add(aire(degats.region()), Ordering::Relaxed);
}

/// Une copie physique d'un rectangle sparse vers l'ecran.
pub fn note_presentation(rect: Rect) {
    if rect.vide() {
        return;
    }
    RECTS_PRESENTES.fetch_add(1, Ordering::Relaxed);
    PIXELS_PRESENTES.fetch_add(aire(rect), Ordering::Relaxed);
}

pub fn remise_a_zero() {
    for compteur in DEGATS_PAR_ORIGINE.iter() {
        compteur.store(0, Ordering::Relaxed);
    }
    DEGATS_TOTAL.store(0, Ordering::Relaxed);
    PIXELS_PRESENTES.store(0, Ordering::Relaxed);
    RECTS_PRESENTES.store(0, Ordering::Relaxed);
    TRAMES_PRESENTEES.store(0, Ordering::Relaxed);
    PIXELS_DEMANDES.store(0, Ordering::Relaxed);
    PIXELS_BOITE_ENGLOBANTE.store(0, Ordering::Relaxed);
    FUSIONS_RECTS.store(0, Ordering::Relaxed);
    DEBORDEMENTS_REGION.store(0, Ordering::Relaxed);
}

/// Degats crees depuis le demarrage, toutes origines confondues.
pub fn total_degats() -> u64 {
    DEGATS_TOTAL.load(Ordering::Relaxed)
}

pub fn degats_plein_ecran() -> u64 {
    DEGATS_PAR_ORIGINE[Origine::PleinEcran.index()].load(Ordering::Relaxed)
}
