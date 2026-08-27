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
//! 2. OCCLUSION. Si la zone OPAQUE d'un calque recouvre entierement le
//!    rectangle, tout ce qui est en dessous est invisible : on part de
//!    celui-la -- INCLUS, puisque c'est lui qui peint ces pixels.
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
    /// Menu deroulant.
    Menu,
    /// Barre des taches.
    BarreTaches,
    /// Curseur logiciel. Jamais opaque : il a des bords transparents.
    Curseur,
}

/// Un calque : ce qu'il est, ou il dessine, et ou il cache ce qu'il y a dessous.
///
/// # Pourquoi DEUX rectangles et non un booleen
///
/// Une fenetre porte une ombre qui deborde de son cadre de quatre pixels, et
/// cette ombre LAISSE VOIR le fond. Les deux questions n'ont donc pas la meme
/// reponse :
///
///   * « quels pixels ce calque peut-il toucher ? »  -> cadre + ombre ;
///   * « quels pixels rend-il inutile de peindre ? » -> cadre seul.
///
/// Un booleen `opaque` ne peut en exprimer qu'une. La premiere version l'avait
/// contourne en ajoutant un calque `ZonePleine` qui ne dessinait rien et ne
/// portait que l'opacite -- et c'est ce contournement qui a produit les
/// trainees de curseur : `premier_calque` rendait l'index de la `ZonePleine`,
/// placee APRES la `Fenetre`, si bien que la boucle
/// `for calque in &calques[debut..]` excluait la fenetre. Le seul calque
/// capable de repeindre ces pixels etait ecarte, et l'ancien curseur restait
/// a l'ecran.
///
/// Deux rectangles suppriment le probleme a la racine : le calque qui declare
/// l'opacite EST celui qui dessine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Calque {
    pub element: Element,
    /// Zone que ce calque peut toucher. Doit MAJORER ce qu'il dessine : un
    /// calque qui deborde ses bornes laisserait des trainees.
    pub bornes_dessin: Rect,
    /// Zone que ce calque peint integralement, sans transparence -- s'il y en
    /// a une. Doit MINORER ce qu'il couvre reellement : une zone opaque
    /// annoncee trop large fait disparaitre ce qu'il y a dessous.
    ///
    /// Les deux exigences sont opposees, et c'est voulu : `bornes_dessin`
    /// majore, `opaque_sur` minore. Dans le doute, `None`.
    pub opaque_sur: Option<Rect>,
}

impl Calque {
    /// Calque qui ne cache rien : curseur, filigrane, icone.
    pub const fn transparent(element: Element, bornes_dessin: Rect) -> Self {
        Self { element, bornes_dessin, opaque_sur: None }
    }

    /// Calque entierement opaque sur ses propres bornes : fond, barre.
    pub const fn plein(element: Element, bornes: Rect) -> Self {
        Self { element, bornes_dessin: bornes, opaque_sur: Some(bornes) }
    }

    /// Calque dont la zone opaque est plus petite que ce qu'il dessine :
    /// fenetre et menu, dont l'ombre deborde du cadre plein.
    pub const fn avec_ombre(element: Element, bornes_dessin: Rect, cadre: Rect) -> Self {
        Self { element, bornes_dessin, opaque_sur: Some(cadre) }
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
/// Parcourt de HAUT en BAS et s'arrete au premier calque dont la zone OPAQUE
/// recouvre entierement `zone` : tout ce qui est en dessous est invisible.
///
/// L'index rendu est INCLUSIF, et c'est le point qui compte : le calque qui
/// declare l'opacite est celui qui peint ces pixels, donc il doit etre dessine.
/// Le rendre exclusif -- ou le faire porter par un calque qui ne dessine rien,
/// comme l'ancienne `ZonePleine` -- laisse les pixels intacts, c'est-a-dire
/// laisse a l'ecran ce qui s'y trouvait : une trainee de curseur.
///
/// Rend `0` quand rien ne recouvre la zone : il faut alors repartir du fond.
pub fn premier_calque(calques: &[Calque], zone: &Rect) -> usize {
    for index in (0..calques.len()).rev() {
        if let Some(opaque) = calques[index].opaque_sur {
            if recouvre(&opaque, zone) {
                return index;
            }
        }
    }
    0
}

/// Ce calque doit-il etre dessine pour `zone` ?
///
/// A n'appeler que sur les calques a partir de [`premier_calque`] : cette
/// fonction ne connait pas l'occlusion, seulement l'intersection.
pub fn doit_dessiner(calque: &Calque, zone: &Rect) -> bool {
    se_touchent(&calque.bornes_dessin, zone)
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
