//! Ce qu'une transition d'etat invalide : la reponse, une seule fois.
//!
//! # Le contrat
//!
//! `etat A -> etat B` produit des pixels differents. Ce module dit LESQUELS,
//! sous forme de rectangles. Le compositeur n'en presentera pas d'autres.
//!
//!     aucun pixel ne doit pouvoir changer sans degat correspondant
//!
//! # Pourquoi ce module et pas des appels a `Degats::ajoute` disperses
//!
//! Trois defauts identiques ont ete trouves dans la meme boucle :
//!
//!   * le tic d'horloge invalidait la barre du BAS alors que l'heure est en
//!     haut ;
//!   * un changement de survol dans le menu n'invalidait que la nouvelle
//!     ligne, jamais celle qui perdait la surbrillance ;
//!   * remonter une fenetre n'invalidait pas celle qui perdait le focus, dont
//!     la barre de titre et les bordures changent pourtant de couleur.
//!
//! Ce sont trois formes d'une seule erreur : l'etat ANCIEN n'est invalide par
//! personne. Ecrite a quinze endroits differents, la regle « ancien puis
//! nouveau » se perd a quelques-uns. Ecrite ici, elle se teste sur l'hote,
//! pixel par pixel, par l'oracle de transition (`tools/gui/test_transitions.rs`).
//!
//! # Ce que ce module ne peut pas prouver
//!
//! Qu'un appelant appelle la bonne fonction au bon moment. Il prouve que la
//! fonction appelee rend un degat SUFFISANT — ce qui est la moitie du probleme,
//! et la moitie qui a produit les trois defauts ci-dessus.

use super::disposition;
use super::protocole::Rect;

/// Une transition n'a jamais produit plus de quatre rectangles.
pub const MAX_RECTS: usize = 4;

// BOUCHAUD_GUI_CIBLE_DEGAT_V1
//
// POURQUOI UN RECTANGLE PORTE SA CIBLE
// ------------------------------------
// Une transition touche parfois DEUX elements de nature differente : ouvrir le
// menu repeint le menu ET le bouton Demarrer de la barre des taches ; changer
// le focus repeint deux fenetres ET les boutons de la barre.
//
// `Rects` ne rendait que des rectangles, alors l'appelant les etiquetait tous
// avec une seule `Origine` : `Origine::Menu` pour la barre des taches,
// `Origine::Fenetre` pour elle aussi. Les degats etaient JUSTES -- les bons
// pixels etaient bien presentes -- mais la mesure, elle, etait fausse.
//
// Cela s'est vu sur un vrai releve : `[GUI-DAMAGE] ... taskbar=0` pour une
// session entiere pendant que la barre des taches etait repeinte des dizaines
// de fois. Un compteur a zero est une invitation a chercher un bug qui n'existe
// pas, et il cache celui qui existe.
//
// Chaque rectangle porte donc ce qu'il vise. L'appelant traduit en `Origine`,
// et la trace redevient lisible.

/// Ce qu'un rectangle de degat vise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cible {
    Fenetre,
    Menu,
    BarreTaches,
    BarreHaute,
    Curseur,
    Icone,
}

/// Petite liste de rectangles etiquetes, sans allocation.
#[derive(Clone, Copy)]
pub struct Rects {
    tampon: [(Rect, Cible); MAX_RECTS],
    nombre: usize,
}

impl Default for Rects {
    fn default() -> Self {
        Self::vide()
    }
}

impl Rects {
    pub const fn vide() -> Self {
        Self {
            tampon: [(Rect::neuf(0, 0, 0, 0), Cible::Fenetre); MAX_RECTS],
            nombre: 0,
        }
    }

    /// Ajoute un rectangle. Un rectangle vide n'apporte rien et est ignore.
    ///
    /// Un debordement au-dela de `MAX_RECTS` est un defaut de programmation,
    /// pas une condition d'execution : aucune transition n'en produit autant.
    pub fn pousse(&mut self, rect: Rect, cible: Cible) {
        if rect.vide() {
            return;
        }
        debug_assert!(self.nombre < MAX_RECTS, "transition a plus de 4 rectangles");
        if self.nombre < MAX_RECTS {
            self.tampon[self.nombre] = (rect, cible);
            self.nombre += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.nombre
    }

    pub fn est_vide(&self) -> bool {
        self.nombre == 0
    }

    /// Les rectangles avec leur cible : c'est la forme que le compositeur
    /// consomme, parce que c'est lui qui sait traduire en `Origine`.
    pub fn iter(&self) -> impl Iterator<Item = (Rect, Cible)> + '_ {
        self.tampon[..self.nombre].iter().copied()
    }

    /// Les rectangles seuls, pour qui n'a que faire de la provenance.
    pub fn rects(&self) -> impl Iterator<Item = Rect> + '_ {
        self.tampon[..self.nombre].iter().map(|(rect, _)| *rect)
    }
}

impl core::iter::FromIterator<(Rect, Cible)> for Rects {
    fn from_iter<I: IntoIterator<Item = (Rect, Cible)>>(iterable: I) -> Self {
        let mut rects = Rects::vide();
        for (rect, cible) in iterable {
            rects.pousse(rect, cible);
        }
        rects
    }
}

/// La regle, une fois pour toutes : l'ancien etat ET le nouveau.
///
/// Presque toutes les transitions du bureau s'y ramenent. Ce qui a change de
/// place laisse derriere lui des pixels que personne d'autre ne connait.
fn ancien_puis_nouveau(ancien: Rect, nouveau: Rect, cible: Cible) -> Rects {
    let mut rects = Rects::vide();
    rects.pousse(ancien, cible);
    if nouveau != ancien {
        rects.pousse(nouveau, cible);
    }
    rects
}

// ─── Curseur ───────────────────────────────────────────────────────────────

/// Le curseur se deplace.
///
/// * QUEL ETAT : la position de la souris.
/// * QUELS PIXELS : la fleche, dans son empreinte 14x22.
/// * QUI INVALIDE L'ANCIEN : ici, `avant`.
/// * QUI INVALIDE LE NOUVEAU : ici, `apres`.
///
/// `avant = None` au tout premier deplacement : rien n'a encore ete dessine.
pub fn curseur_deplace(avant: Option<(i32, i32)>, apres: (i32, i32)) -> Rects {
    let mut rects = Rects::vide();
    if let Some((x, y)) = avant {
        rects.pousse(disposition::curseur(x, y), Cible::Curseur);
    }
    rects.pousse(disposition::curseur(apres.0, apres.1), Cible::Curseur);
    rects
}

// BOUCHAUD_GUI_CURSEUR_ADAPTATIF_V1
//
// LE CURSEUR EST DE L'ETAT DERIVE
// -------------------------------
// `draw_cursor` lit le pixel du backbuffer SOUS son point chaud et en deduit
// sa luminance : sur un fond clair il se peint noir cercle de blanc, sur un
// fond sombre l'inverse. Sa couleur est donc une fonction de ce qu'il y a
// dessous — c'est-a-dire de l'etat compose, pas de l'etat du bureau.
//
// Consequence : si un degat repeint ce qu'il y a sous le point chaud, la
// fleche entiere change de couleur, y compris les pixels que ce degat ne
// couvre pas. Un rectangle qui ne recouvre qu'une partie de l'empreinte
// laisse alors un curseur bicolore, moitie noir moitie blanc.
//
// La regle est donc : des qu'un degat touche l'empreinte du curseur,
// l'empreinte ENTIERE est invalidee. Cela coute au plus un rectangle de
// 14 x 22 par trame, et seulement quand quelque chose bouge sous la fleche.

/// L'empreinte a repeindre quand un degat touche le curseur, s'il y a lieu.
///
/// * QUEL ETAT : la couleur du curseur, deduite du pixel compose sous son
///   point chaud.
/// * QUELS PIXELS : toute la fleche.
/// * QUI INVALIDE L'ANCIEN et LE NOUVEAU : ici, et personne d'autre. Le degat
///   qui a change le fond ne connait pas cette dependance.
pub fn recoloration_curseur(regions: &[Rect], souris: (i32, i32)) -> Rects {
    let empreinte = disposition::curseur(souris.0, souris.1);
    let mut rects = Rects::vide();
    for region in regions.iter() {
        if !region.intersecte(&empreinte).vide() {
            rects.pousse(empreinte, Cible::Curseur);
            break;
        }
    }
    rects
}

// ─── Menu ──────────────────────────────────────────────────────────────────

/// Le survol passe d'une ligne du menu a une autre, ou entre et sort du menu.
///
/// * QUEL ETAT : la ligne sous le pointeur (`disposition::ligne_menu_survolee`).
/// * QUELS PIXELS : toute la ligne — fond, bordure de selection, couleur et
///   graisse du texte.
/// * QUI INVALIDE L'ANCIEN : ici, et personne d'autre. L'empreinte du curseur
///   ne recouvre que la nouvelle.
/// * QUI INVALIDE LE NOUVEAU : ici.
pub fn survol_menu_change(menu: Rect, avant: Option<usize>, apres: Option<usize>) -> Rects {
    let mut rects = Rects::vide();
    if avant == apres {
        return rects;
    }
    if let Some(index) = avant {
        rects.pousse(disposition::rect_ligne_menu(menu, index), Cible::Menu);
    }
    if let Some(index) = apres {
        rects.pousse(disposition::rect_ligne_menu(menu, index), Cible::Menu);
    }
    rects
}

/// Le menu s'ouvre ou se ferme.
///
/// * QUEL ETAT : `menu_open`.
/// * QUELS PIXELS : le menu et son ombre portee ; le bouton Demarrer, qui
///   change de couleur selon que le menu est ouvert.
/// * QUI INVALIDE L'ANCIEN et LE NOUVEAU : ici. Les deux etats occupent le
///   meme rectangle, il n'y en a donc qu'un a annoncer.
pub fn menu_bascule(menu: Rect, barre_taches: Rect) -> Rects {
    let mut rects = Rects::vide();
    rects.pousse(disposition::empreinte_avec_ombre(menu), Cible::Menu);
    // Le bouton Demarrer change de couleur avec l'ouverture du menu : c'est la
    // BARRE DES TACHES qui est repeinte, et la trace doit le dire.
    rects.pousse(barre_taches, Cible::BarreTaches);
    rects
}

// ─── Fenetres ──────────────────────────────────────────────────────────────

/// Une fenetre bouge, change de taille, se maximise ou se restaure.
///
/// * QUEL ETAT : `x, y, w, h`.
/// * QUELS PIXELS : le cadre A LA POSITION QUITTEE (le fond y reapparait) et a
///   la position atteinte — ombre portee comprise dans les deux cas.
/// * QUI INVALIDE L'ANCIEN : ici. Sans lui, la fenetre laisse une trainee.
/// * QUI INVALIDE LE NOUVEAU : ici.
///
/// Les deux arguments sont des CADRES ; l'ombre est ajoutee ici, pour que le
/// debord ne puisse pas etre oublie a un appelant.
pub fn fenetre_bougee(cadre_avant: Rect, cadre_apres: Rect) -> Rects {
    ancien_puis_nouveau(
        disposition::empreinte_avec_ombre(cadre_avant),
        disposition::empreinte_avec_ombre(cadre_apres),
        Cible::Fenetre,
    )
}

/// Le focus passe d'une fenetre a une autre.
///
/// * QUEL ETAT : quelle fenetre est au-dessus (`widgets::indice_focus`).
/// * QUELS PIXELS : dans les DEUX fenetres — barre de titre, ligne de
///   separation et quatre bordures changent de couleur avec le focus — plus les
///   boutons de la barre des taches.
/// * QUI INVALIDE L'ANCIEN : ici. C'est ce qui manquait : la fenetre
///   precedemment active gardait sa barre de titre bleue.
/// * QUI INVALIDE LE NOUVEAU : ici.
pub fn focus_transfere(
    cadre_perdu: Option<Rect>,
    cadre_gagne: Rect,
    barre_taches: Rect,
) -> Rects {
    let mut rects = Rects::vide();
    rects.pousse(disposition::empreinte_avec_ombre(cadre_gagne), Cible::Fenetre);
    if let Some(cadre) = cadre_perdu {
        rects.pousse(disposition::empreinte_avec_ombre(cadre), Cible::Fenetre);
    }
    // Les boutons de la barre des taches suivent le focus.
    rects.pousse(barre_taches, Cible::BarreTaches);
    rects
}

// ─── Barre du haut ─────────────────────────────────────────────────────────

/// Une seconde passe : l'heure, la charge CPU, la memoire et le disque bougent.
///
/// * QUEL ETAT : le temps. Personne ne l'annonce ; c'est la seule animation
///   permanente du bureau, et la raison d'etre de `PERIODE_HORLOGE_MS`.
/// * QUELS PIXELS : la barre du HAUT, et elle seule. Rien dans celle du bas ne
///   change avec le temps.
/// * QUI INVALIDE L'ANCIEN et LE NOUVEAU : ici. Les deux sont au meme endroit.
pub fn tic_horloge(largeur: u32) -> Rects {
    let mut rects = Rects::vide();
    rects.pousse(disposition::barre_haute(largeur), Cible::BarreHaute);
    rects
}
