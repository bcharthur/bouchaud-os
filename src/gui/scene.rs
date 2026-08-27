//! Ce que le compositeur doit reellement dessiner pour un rectangle donne.
//!
//! # Le probleme, mesure
//!
//! La composition dessinait la scene ENTIERE une fois par rectangle de degat :
//!
//! ```text
//!     pour chaque rectangle sale {
//!         set_clip(rectangle)
//!         draw_desktop(...)   // fond + filigrane + icones + barre + fenetres
//!         draw_menu(...)
//!         draw_taskbar(...)
//!         draw_cursor(...)
//!         reset_clip()
//!     }
//! ```
//!
//! La decoupe empeche bien les ECRITURES hors zone -- `drawn_pixels` ne compte
//! que ce qui atteint le tampon. Mais elle n'empeche RIEN d'autre. Redessiner
//! la barre du haut pour un rectangle de curseur de 16x16 en bas de l'ecran
//! coutait quand meme, a chaque rectangle :
//!
//!   * une lecture de l'horloge temps reel par ports d'E/S ;
//!   * un formatage de chaine pour l'heure et pour les statistiques CPU ;
//!   * trois rasterisations de texte TrueType ;
//!   * `BAR_H` remplissages pleine largeur pour le degrade.
//!
//! Rien de tout cela n'apparait dans `drawn_pixels`, et tout cela consomme du
//! processeur. C'est pourquoi le ratio `drawn_pixels / presented_pixels` ne
//! raconte qu'une partie de l'histoire : le travail evite ici est en grande
//! partie du travail qui ne s'ecrivait deja pas.
//!
//! # Deux regles, dans cet ordre
//!
//! 1. INTERSECTION. Un calque dont les bornes ne touchent pas le rectangle
//!    n'est pas dessine du tout.
//! 2. OCCLUSION. Si un calque OPAQUE recouvre entierement le rectangle, tout ce
//!    qui est en dessous est invisible : on part de celui-la.
//!
//! La seconde est celle qui fait tomber `drawn_pixels` : sous une fenetre de
//! navigateur qui occupe l'ecran, le fond d'ecran etait repeint puis
//! integralement recouvert, a chaque rectangle de chaque trame.
//!
//! # Pourquoi ce module ne dessine rien
//!
//! Il ne repond qu'a « quoi, dans quel ordre » a partir de rectangles. Aucun
//! framebuffer, aucune police, aucune fenetre : le harnais hote l'exerce
//! directement, et une regle de culling fausse se voit en test au lieu de se
//! voir a l'ecran.

use super::protocole::Rect;

/// Ce qu'un calque represente. Le compositeur traduit en appels de dessin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Element {
    /// Fond d'ecran. Opaque, plein ecran, tout en dessous.
    Fond,
    /// Filigrane « Bouchaud OS ».
    Filigrane,
    /// Une icone du bureau, par indice.
    Icone(usize),
    /// Barre superieure : horloge, statistiques systeme.
    BarreHaute,
    /// Une fenetre, par indice dans la liste (l'ordre de la liste EST le
    /// z-order : la derniere est au-dessus).
    Fenetre(usize),
    /// Zone PLEINE d'un calque qui deborde ses propres bornes. Ne dessine rien.
    ///
    /// Une fenetre -- et le menu -- portent une ombre qui deborde de leur
    /// cadre : leurs bornes de DESSIN doivent inclure l'ombre, mais l'ombre
    /// laisse voir le fond. Un seul calque ne peut pas dire les deux, et les
    /// confondre ferait disparaitre le fond sous l'ombre.
    ///
    /// Ce calque-ci porte l'OPACITE -- le cadre seul --, le calque qui le
    /// precede porte le DESSIN. Il vient donc juste apres lui : l'occlusion se
    /// calcule de haut en bas, et il doit etre au-dessus de ce qu'il occulte.
    ZonePleine,
    /// Menu deroulant.
    Menu,
    /// Barre des taches.
    BarreTaches,
    /// Curseur logiciel. Jamais opaque : il a des bords transparents.
    Curseur,
}

/// Un calque : ce qu'il est, ou il est, et s'il cache ce qu'il y a dessous.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Calque {
    pub element: Element,
    /// Zone que ce calque peut toucher. Doit MAJORER ce qu'il dessine : un
    /// calque qui deborde ses bornes laisserait des trainees.
    pub bornes: Rect,
    /// Ce calque peint-il chacun de ses pixels, sans transparence ?
    ///
    /// Dans le doute, `false`. Un calque declare opaque a tort fait disparaitre
    /// ce qu'il y a dessous ; un calque declare transparent a tort ne coute
    /// qu'un peu de travail.
    pub opaque: bool,
}

impl Calque {
    pub const fn neuf(element: Element, bornes: Rect, opaque: bool) -> Self {
        Self { element, bornes, opaque }
    }
}

/// Deux rectangles se touchent-ils ?
pub fn se_touchent(a: &Rect, b: &Rect) -> bool {
    !a.vide() && !b.vide() && !a.intersecte(b).vide()
}

/// `contenant` recouvre-t-il entierement `zone` ?
///
/// Comparaison en `i64` : `droite()` et `bas()` y sont deja pour qu'un
/// rectangle hostile ne puisse pas deborder l'addition et pretendre contenir
/// l'ecran.
pub fn recouvre(contenant: &Rect, zone: &Rect) -> bool {
    if zone.vide() {
        return true;
    }
    if contenant.vide() {
        return false;
    }
    contenant.x as i64 <= zone.x as i64
        && contenant.y as i64 <= zone.y as i64
        && contenant.droite() >= zone.droite()
        && contenant.bas() >= zone.bas()
}

/// Index du premier calque a dessiner pour `zone`, occlusion comprise.
///
/// Parcourt de HAUT en BAS et s'arrete au premier calque opaque qui recouvre
/// entierement la zone : tout ce qui est en dessous est invisible.
///
/// Rend `0` quand rien ne recouvre la zone -- il faut alors repartir du fond.
pub fn premier_calque(calques: &[Calque], zone: &Rect) -> usize {
    for index in (0..calques.len()).rev() {
        let calque = &calques[index];
        if calque.opaque && recouvre(&calque.bornes, zone) {
            return index;
        }
    }
    0
}

/// Ce calque doit-il etre dessine pour `zone` ?
///
/// A n'appeler que sur les calques a partir de [`premier_calque`] : cette
/// fonction ne connait pas l'occlusion, seulement l'intersection.
pub fn doit_dessiner(calque: &Calque, zone: &Rect) -> bool {
    se_touchent(&calque.bornes, zone)
}

/// Nombre de calques que la composition d'une zone va reellement traverser.
///
/// Sert a la mesure : le rapport entre ce nombre et `calques.len()` dit ce que
/// le culling economise, indepandamment des pixels.
pub fn calques_retenus(calques: &[Calque], zone: &Rect) -> usize {
    let debut = premier_calque(calques, zone);
    calques[debut..]
        .iter()
        .filter(|calque| doit_dessiner(calque, zone))
        .count()
}
