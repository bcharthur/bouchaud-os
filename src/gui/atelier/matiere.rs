//! Matieres Atelier : verre, ombre douce, fenetre (BOUCHAUD_ATELIER_MATIERE_V1).
//!
//! # Pourquoi des peintres a part
//!
//! `graphics::paint_window_shape_spans` peint ses huit anneaux d'ombre en gris
//! OPAQUES, calibres pour le fond nocturne d'avant : sur un fond clair ils
//! dessinent un cadre sale. Ici l'ombre est une teinte melangee a opacite
//! decroissante -- elle assombrit ce qui est dessous au lieu de le remplacer.
//!
//! Les bornes sont les MEMES : `etendue` <= `SHADOW_EXTENT` (8), donc tout ce
//! qui est peint reste dans `painted_bounds`, que le plan de scene et les
//! degats annoncent deja. Rien de nouveau a invalider.
//!
//! # Cout
//!
//! Par segments, comme le reste du rendu : un anneau est O(hauteur) segments,
//! le melange est fait par segment (`blend_rect_rgb`). L'interieur d'une
//! fenetre opaque passe par `fill_rect_rgb`.

use super::jetons as j;
use crate::gui::framebuffer as fb;
use crate::gui::windowing::Rect;

fn decoupe() -> Rect {
    let (cx0, cy0, cx1, cy1) = fb::clip_rect();
    Rect::new(cx0 as i32, cy0 as i32, cx1.saturating_sub(cx0) as u32,
        cy1.saturating_sub(cy0) as u32)
}

/// Ombre douce autour de `cadre` : `etendue` anneaux, du plus dense (au
/// contact) au plus leger. Decroissance quadratique : l'oeil lit une ombre
/// lineaire comme un trait.
pub fn ombre_douce(cadre: Rect, rayon: u32, etendue: u32, opacite_max: u8) {
    let clip = decoupe();
    let opacite = |e: u32| -> u8 {
        let reste = etendue + 1 - e;
        ((opacite_max as u32 * reste * reste) / ((etendue + 1) * (etendue + 1))) as u8
    };
    // BOUCHAUD_GFX_OMBRE_PAR_LIGNE_V1
    //
    // Entre les arcs du haut et du bas, chaque anneau ne pose qu'un pixel a
    // gauche (`cadre.x - e`) et un a droite (`cadre.right() + e - 1`) : les
    // huit anneaux y forment deux rampes de huit pixels contigus. On les pose
    // d'un appel par cote et par ligne au lieu de seize segments d'un pixel :
    // l'ombre d'une fenetre deplacee passe de ~2 ms a ~1 ms par trame sous QEMU
    // TCG. Les anneaux sont disjoints, donc l'ordre des melanges ne change
    // rien : memes pixels, memes opacites.
    //
    // La bande vaut pour TOUS les anneaux a la fois tant que le rayon n'est pas
    // borne par la taille de la forme (`2 * rayon <= min(l, h)`) ; sinon, et
    // pour les lignes des arcs, les anneaux gardent leur chemin.
    const RAMPE_MAX: usize = 32;
    let bande = (etendue as usize <= RAMPE_MAX
        && 2 * rayon <= cadre.width.min(cadre.height))
        .then(|| (cadre.y + rayon as i32, cadre.bottom() - rayon as i32))
        .filter(|(debut, fin)| fin > debut);
    let Some((bande_debut, bande_fin)) = bande else {
        anneaux(cadre, rayon, etendue, &opacite, clip);
        return;
    };
    let lignes = |y0: i32, y1: i32| {
        let y0 = y0.max(clip.y);
        let y1 = y1.min(clip.bottom());
        (y1 > y0).then(|| Rect::new(clip.x, y0, clip.width, (y1 - y0) as u32))
    };
    if let Some(haut) = lignes(i32::MIN / 2, bande_debut) {
        anneaux(cadre, rayon, etendue, &opacite, haut);
    }
    if let Some(bas) = lignes(bande_fin, i32::MAX / 2) {
        anneaux(cadre, rayon, etendue, &opacite, bas);
    }
    let n = etendue as usize;
    let mut gauche = [0u8; RAMPE_MAX];
    let mut droite = [0u8; RAMPE_MAX];
    for e in 1..=etendue {
        gauche[n - e as usize] = opacite(e); // x = cadre.x - e
        droite[e as usize - 1] = opacite(e); // x = cadre.right() + e - 1
    }
    // Une rampe qui commence hors de l'ecran a gauche perd ses premiers pixels.
    let x_gauche = cadre.x - etendue as i32;
    let saut = (-x_gauche).max(0) as usize;
    let x_droite = cadre.right();
    for y in bande_debut.max(clip.y)..bande_fin.min(clip.bottom()) {
        if saut < n {
            fb::melange_rampe((x_gauche + saut as i32) as usize, y as usize, j::OMBRE,
                &gauche[saut..n]);
        }
        if x_droite >= 0 {
            fb::melange_rampe(x_droite as usize, y as usize, j::OMBRE, &droite[..n]);
        }
    }
}

/// Les anneaux de l'ombre, un par un, bornes a `clip`.
fn anneaux(cadre: Rect, rayon: u32, etendue: u32, opacite: &impl Fn(u32) -> u8, clip: Rect) {
    for e in 1..=etendue {
        let a = opacite(e);
        if a == 0 {
            continue;
        }
        let anneau = cadre.outset(e);
        crate::gui::graphics::spans_stroke_rounded_rect(anneau, rayon + e, 1, clip,
            |x, y, largeur| {
                fb::blend_rect_rgb(x.max(0) as usize, y.max(0) as usize, largeur as usize,
                    j::OMBRE, a)
            });
    }
}

/// Remplissage arrondi anticrenele d'une couleur a opacite donnee.
pub fn pave(cadre: Rect, rayon: u32, couleur: u32, opacite: u8) {
    let clip = decoupe();
    crate::gui::graphics::spans_rounded_rect_aa(cadre, rayon, clip,
        |x, y, largeur, couverture| {
            let a = (couverture as u32 * opacite as u32 + 127) / 255;
            fb::blend_rect_rgb(x.max(0) as usize, y.max(0) as usize, largeur as usize,
                couleur, a as u8)
        });
}

/// Contour arrondi anticrenele de 1 px a opacite donnee.
pub fn filet(cadre: Rect, rayon: u32, couleur: u32, opacite: u8) {
    let clip = decoupe();
    crate::gui::graphics::spans_stroke_rounded_rect_aa(cadre, rayon, 1, clip,
        |x, y, largeur, couverture| {
            let a = (couverture as u32 * opacite as u32 + 127) / 255;
            fb::blend_rect_rgb(x.max(0) as usize, y.max(0) as usize, largeur as usize,
                couleur, a as u8)
        });
}

/// Plaque de verre flottante (dock, menu) : ombre, verre, liseré, filet.
pub fn plaque_verre(cadre: Rect, rayon: u32, opacite: u8) {
    ombre_douce(cadre, rayon, 8, j::OMBRE_FLOTTANT);
    pave(cadre, rayon, j::VERRE, opacite);
    // Le liseré : la lumiere accroche le bord superieur. Une ligne, en retrait
    // des coins pour ne pas les deborder.
    let r = rayon as i32;
    let largeur = (cadre.width as i32 - 2 * r).max(0) as usize;
    fb::blend_rect_rgb((cadre.x + r).max(0) as usize, (cadre.y + 1).max(0) as usize,
        largeur, j::VERRE, j::VERRE_LISERE);
    filet(cadre, rayon, j::FILET, j::FILET_OPACITE);
}

/// Bande de verre pleine largeur (barre du haut) : verre + filet bas.
pub fn bande_verre(x: usize, y: usize, largeur: usize, hauteur: usize, opacite: u8) {
    for ligne in 0..hauteur {
        fb::blend_rect_rgb(x, y + ligne, largeur, j::VERRE, opacite);
    }
    fb::blend_rect_rgb(x, y + hauteur.saturating_sub(1), largeur, j::FILET, j::FILET_OPACITE);
}

/// Le corps d'une fenetre Atelier : ombre douce, surface, bordure.
///
/// `surface` est la couleur du CORPS : les applications natives peignent
/// encore leur contenu pour une surface sombre, donc le corps garde
/// `theme::COLOR_SURFACE` tant qu'elles ne sont pas passees a Atelier ; la
/// barre de titre, peinte par-dessus, porte la matiere claire.
///
/// `opaque` : zone que l'appelant repeindra opaque plus tard dans la trame
/// (client ring 3) ; la surface ne l'ecrit pas, comme dans
/// `paint_window_shape_spans` (BOUCHAUD_C13_PAS_DEUX_FOIS_LE_MEME_PIXEL_V1).
pub fn fenetre(cadre: Rect, rayon: u32, etendue: u32, active: bool, surface: u32,
    opaque: Option<Rect>) {
    let clip = decoupe();
    ombre_douce(cadre, rayon, etendue,
        if active { j::OMBRE_FENETRE_ACTIVE } else { j::OMBRE_FENETRE_INACTIVE });
    crate::gui::graphics::spans_rounded_rect_aa_sauf(cadre, rayon, clip,
        opaque.unwrap_or(Rect::new(0, 0, 0, 0)),
        |x, y, largeur, couverture| {
            fb::blend_rect_rgb(x.max(0) as usize, y.max(0) as usize, largeur as usize,
                surface, couverture)
        });
    crate::gui::graphics::spans_stroke_rounded_rect_aa(cadre, rayon, 1, clip,
        |x, y, largeur, couverture| {
            fb::blend_rect_rgb(x.max(0) as usize, y.max(0) as usize, largeur as usize,
                j::BORDURE_FENETRE, couverture)
        });
}
