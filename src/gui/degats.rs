//! Ce qu'un evenement du bureau salit reellement, et d'ou cela vient.
//!
//! # Pourquoi ce fichier existe
//!
//! Tout ce qui est ici est de la geometrie : des rectangles, une union, et le
//! compte de ce qui a ete demande. Aucun framebuffer, aucun pilote, aucune
//! architecture. C'est ce qui permet de l'exercer sur la machine de
//! developpement -- voir `tools/gui/test_degats.rs`.
//!
//! La regle a verifier ne se lit pas dans un journal : « cent clics et cent
//! crans de molette dans une page ne doivent produire AUCUN degat plein
//! ecran ». Une regle pareille se demontre, elle ne s'observe pas.
//!
//! `Degats` porte les bornes de l'ecran plutot que de les lire quelque part :
//! la politique n'a aucune raison de savoir sur quel materiel elle tourne, et
//! un test peut ainsi travailler sur un ecran de dix pixels.

use crate::gui::protocole::Rect;
use core::sync::atomic::{AtomicU64, Ordering};

// BOUCHAUD_GUI_DAMAGE_ORIGIN_V1
//
// D'ou vient un degat, et non seulement quelle taille il fait.
//
// Un compteur unique ne repond pas a la question qui se pose : « pourquoi
// l'ecran entier a-t-il ete repeint ? ». Le clavier l'a montre -- le coupable
// n'etait pas le rendu, c'etait UNE ligne dans la boucle d'entree. La meme
// question va se poser pour la souris, la molette, le deplacement de fenetre.
// L'origine est donc portee par le degat lui-meme.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Origine {
    /// Change reellement la majorite de l'ecran : ouverture ou fermeture de
    /// fenetre, changement de session. Doit rester rare.
    PleinEcran,
    /// Une fenetre : cadre, contenu dessine par le bureau, deplacement.
    Fenetre,
    /// L'empreinte du curseur logiciel, ancienne ou nouvelle.
    Curseur,
    /// Une surface cliente recopiee : c'est le client qui a annonce ce degat.
    Client,
    /// La barre des taches.
    BarreTaches,
    /// Le menu Demarrer.
    Menu,
    /// Une icone du bureau.
    Icone,
}

const NOMBRE_ORIGINES: usize = 7;
static DEGATS_PAR_ORIGINE: [AtomicU64; NOMBRE_ORIGINES] =
    [const { AtomicU64::new(0) }; NOMBRE_ORIGINES];
static PIXELS_PRESENTES: AtomicU64 = AtomicU64::new(0);
static TRAMES_PRESENTEES: AtomicU64 = AtomicU64::new(0);

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
        }
    }
}

/// Degat accumule d'une trame, avec la provenance de chaque contribution.
///
/// Une seule boite englobante, comme avant -- le repli assume du jalon 2. Ce
/// qui est nouveau, c'est que rien n'y entre sans dire d'ou il vient : c'est ce
/// qui permet d'affirmer « cent clics n'ont produit aucun degat plein ecran »
/// au lieu de l'esperer.
#[derive(Clone, Copy)]
pub struct Degats {
    region: Rect,
    ecran: Rect,
}

impl Degats {
    /// `ecran` borne ce que `tout()` designe. Le passer plutot que le lire
    /// evite a cette politique de dependre du materiel -- et permet a un test
    /// de raisonner sur un ecran de dix pixels de cote.
    pub fn neuf(ecran: Rect) -> Self {
        Self { region: Rect::default(), ecran }
    }

    /// Ajoute une region en la rattachant a son origine.
    ///
    /// Un rectangle vide n'est pas compte : il n'a rien sali, et le compter
    /// ferait mentir la mesure dans le sens le plus trompeur -- celui qui
    /// laisse croire a une activite qui n'existe pas.
    pub fn ajoute(&mut self, origine: Origine, rect: Rect) {
        if rect.vide() {
            return;
        }
        DEGATS_PAR_ORIGINE[origine.index()].fetch_add(1, Ordering::Relaxed);
        self.region = self.region.union(&rect);
    }

    /// Tout l'ecran. Ecrit ainsi pour que `grep -n 'tout()'` trouve tous les
    /// endroits qui se l'autorisent, et qu'ils restent comptables.
    pub fn tout(&mut self) {
        self.ajoute(Origine::PleinEcran, self.ecran);
    }

    pub fn region(&self) -> Rect {
        self.region
    }

    pub fn vide(&self) -> bool {
        self.region.vide()
    }

    pub fn efface(&mut self) {
        self.region = Rect::default();
    }
}

/// Compteurs de composition : (par origine, trames presentees, pixels copies).
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


/// Note une trame reellement copiee vers l'ecran.
pub fn note_presentation(rect: Rect) {
    TRAMES_PRESENTEES.fetch_add(1, Ordering::Relaxed);
    PIXELS_PRESENTES.fetch_add(
        rect.largeur as u64 * rect.hauteur as u64,
        Ordering::Relaxed,
    );
}

/// Remet les compteurs a zero. Reserve aux tests : deux scenarios qui se
/// suivent doivent pouvoir affirmer chacun sur son propre compte.
pub fn remise_a_zero() {
    for compteur in DEGATS_PAR_ORIGINE.iter() {
        compteur.store(0, Ordering::Relaxed);
    }
    TRAMES_PRESENTEES.store(0, Ordering::Relaxed);
    PIXELS_PRESENTES.store(0, Ordering::Relaxed);
}

/// Nombre de degats plein ecran demandes depuis le demarrage.
///
/// Publie par `[GUI-INPUT]` : c'est le chiffre qui a servi a prouver qu'une
/// frappe ne repeint plus le bureau, et le meme doit servir pour le clic et la
/// molette. Il se lit dans le compteur de l'origine, sans second compteur a
/// tenir a jour -- deux compteurs pour une meme chose finissent toujours par
/// diverger.
pub fn degats_plein_ecran() -> u64 {
    DEGATS_PAR_ORIGINE[Origine::PleinEcran.index()].load(Ordering::Relaxed)
}
