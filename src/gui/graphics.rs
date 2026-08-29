//! Primitives visuelles CPU sans allocation, rasterisees par SEGMENTS.
//!
//! # Pourquoi des segments et non des pixels
//!
//! La premiere version parcourait `rect ∩ clip` pixel par pixel, appelait une
//! fermeture pour chacun et testait l'appartenance a la forme arrondie a chaque
//! fois. Pour une fenetre maximisee, `rect` fait 1280 x 698, soit 893 440
//! iterations -- et `stroke_rounded_rect` faisait le meme balayage complet pour
//! ne peindre qu'un contour de mille pixels.
//!
//! `paint_window_shape` appelle ce contour `SHADOW_EXTENT + 1` fois : huit
//! anneaux d'ombre plus la bordure. Une seule fenetre maximisee coutait donc
//! environ **huit millions d'iterations par trame**, chacune avec une fermeture
//! indirecte, deux tests d'appartenance et -- via `fb::pixel_rgb` --
//! **un `fetch_add` atomique par pixel**. Sous TCG, un RMW atomique est un
//! appel d'assistance : c'est des dizaines de millisecondes par fenetre.
//!
//! Un rectangle arrondi est pourtant descriptible ligne par ligne : hors des
//! deux bandes de coins, chaque ligne est un segment plein ; dans les bandes,
//! ses bornes se calculent par une racine carree entiere. Le contour est la
//! difference de deux segments. Le cout passe de `O(largeur x hauteur)` tests
//! de pixel a `O(hauteur)` segments -- pour une fenetre maximisee, 698 appels
//! au lieu de 893 440, et un seul atomique par segment au lieu d'un par pixel.
//!
//! La forme rendue est identique au pixel pres : `inside_rounded` reste la
//! definition, et `bornes_ligne` en est la resolution analytique. Un test
//! d'equivalence les compare exhaustivement.
//!
//! # L'API
//!
//! Les fonctions `spans_*` sont le chemin rapide : le noyau leur donne un
//! remplissage de segment (`fb::fill_rect_rgb`). Les fonctions `fill_*` et
//! `stroke_*` gardent l'interface pixel dont les tests d'hote ont besoin pour
//! affirmer qu'aucun pixel ne sort de la decoupe ; elles sont ecrites SUR les
//! segments, donc elles ne peuvent pas diverger de ce que le noyau dessine.

use crate::gui::windowing::Rect;

/// Exact bounded painter used by runtime chrome and host contract tests.
pub fn paint_window_shape<F: FnMut(i32, i32, u32)>(
    geometry: crate::gui::windowing::WindowRenderGeometry,
    radius: u32, shadow_extent: u32, clip: Rect, surface: u32, border: u32,
    mut paint: F,
) {
    for extent in (1..=shadow_extent).rev() {
        let shadow = geometry.outer.outset(extent);
        let shade = 0x07090d + extent * 0x010101;
        stroke_rounded_rect(shadow, radius + extent, 1, clip,
            |x, y| paint(x, y, shade));
    }
    fill_rounded_rect(geometry.outer, radius, clip,
        |x, y| paint(x, y, surface));
    stroke_rounded_rect(geometry.outer, radius, 1, clip,
        |x, y| paint(x, y, border));
}

/// Meme forme que [`paint_window_shape`], mais rendue par segments.
///
/// C'est ce chemin qu'emprunte le compositeur : `remplit(x, y, largeur, couleur)`
/// recoit des segments horizontaux, jamais des pixels isoles.
pub fn paint_window_shape_spans<F: FnMut(i32, i32, u32, u32)>(
    geometry: crate::gui::windowing::WindowRenderGeometry,
    radius: u32, shadow_extent: u32, clip: Rect, surface: u32, border: u32,
    mut remplit: F,
) {
    for extent in (1..=shadow_extent).rev() {
        let shadow = geometry.outer.outset(extent);
        let shade = 0x07090d + extent * 0x010101;
        spans_stroke_rounded_rect(shadow, radius + extent, 1, clip,
            |x, y, largeur| remplit(x, y, largeur, shade));
    }
    spans_rounded_rect(geometry.outer, radius, clip,
        |x, y, largeur| remplit(x, y, largeur, surface));
    spans_stroke_rounded_rect(geometry.outer, radius, 1, clip,
        |x, y, largeur| remplit(x, y, largeur, border));
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    if right <= x || bottom <= y { None }
    else { Some(Rect::new(x, y, (right - x) as u32, (bottom - y) as u32)) }
}

/// Racine carree entiere, par recherche dichotomique. Aucun flottant : ce code
/// tourne dans le noyau, ou la FPU appartient a la tache interrompue.
fn racine(valeur: i32) -> i32 {
    if valeur <= 0 { return 0 }
    let mut bas = 0i32;
    let mut haut = 46_341i32.min(valeur + 1);
    while bas < haut {
        let milieu = bas + (haut - bas + 1) / 2;
        if milieu.saturating_mul(milieu) <= valeur { bas = milieu } else { haut = milieu - 1 }
    }
    bas
}

/// LA definition de la forme. `bornes_ligne` en est la resolution analytique,
/// et un test d'hote les compare exhaustivement.
fn inside_rounded(x: i32, y: i32, width: i32, height: i32, radius: i32) -> bool {
    let radius = radius.max(0).min(width.min(height) / 2);
    if radius == 0 { return true }
    let cx = if x < radius { radius - 1 }
        else if x >= width - radius { width - radius } else { return true };
    let cy = if y < radius { radius - 1 }
        else if y >= height - radius { height - radius } else { return true };
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}

/// Bornes `[debut, fin)` de la forme arrondie sur la ligne locale `y`.
///
/// `None` si la ligne ne contient aucun pixel. C'est l'inverse analytique de
/// [`inside_rounded`] : au lieu de tester chaque abscisse, on calcule les deux
/// seules qui comptent.
fn bornes_ligne(largeur: i32, hauteur: i32, radius: i32, y: i32) -> Option<(i32, i32)> {
    if largeur <= 0 || hauteur <= 0 || y < 0 || y >= hauteur { return None }
    let radius = radius.max(0).min(largeur.min(hauteur) / 2);
    if radius == 0 { return Some((0, largeur)) }

    // Hors des bandes de coins, `inside_rounded` sort par `return true`.
    let cy = if y < radius { radius - 1 }
        else if y >= hauteur - radius { hauteur - radius }
        else { return Some((0, largeur)) };

    let dy = y - cy;
    let reste = radius * radius - dy * dy;
    if reste < 0 { return None }
    let s = racine(reste);

    // Gauche : x >= (radius - 1) - s. Droite : x <= (largeur - radius) + s.
    let debut = (radius - 1 - s).max(0);
    let fin = (largeur - radius + s + 1).min(largeur);
    if fin <= debut { None } else { Some((debut, fin)) }
}

/// Segments horizontaux de `rect ∩ clip`, un par ligne.
///
/// Rend le nombre de pixels couverts : c'est la mesure que les tests d'hote
/// utilisent pour affirmer que la decoupe borne bien le travail.
pub fn spans_rounded_rect<F: FnMut(i32, i32, u32)>(rect: Rect, radius: u32, clip: Rect,
    mut segment: F) -> usize {
    let Some(area) = intersection(rect, clip) else { return 0 };
    let mut couverts = 0usize;
    for y in area.y..area.bottom() {
        let Some((debut, fin)) = bornes_ligne(rect.width as i32, rect.height as i32,
            radius as i32, y - rect.y) else { continue };
        let x0 = (rect.x + debut).max(area.x);
        let x1 = (rect.x + fin).min(area.right());
        if x1 <= x0 { continue }
        couverts += (x1 - x0) as usize;
        segment(x0, y, (x1 - x0) as u32);
    }
    couverts
}

/// Segments du CONTOUR : la difference entre la forme et sa version retrecie.
///
/// Une ligne donne au plus deux segments -- le bord gauche et le bord droit --
/// ou un seul quand la ligne est entierement dans l'epaisseur (haut et bas du
/// cadre). Jamais un balayage de toute la largeur.
pub fn spans_stroke_rounded_rect<F: FnMut(i32, i32, u32)>(rect: Rect, radius: u32,
    thickness: u32, clip: Rect, mut segment: F) -> usize {
    let Some(area) = intersection(rect, clip) else { return 0 };
    let inset = thickness as i32;
    let largeur = rect.width as i32;
    let hauteur = rect.height as i32;
    let largeur_interne = largeur - inset * 2;
    let hauteur_interne = hauteur - inset * 2;
    let radius_interne = radius.saturating_sub(thickness) as i32;

    let mut couverts = 0usize;
    for y in area.y..area.bottom() {
        let ly = y - rect.y;
        let Some((debut, fin)) = bornes_ligne(largeur, hauteur, radius as i32, ly)
        else { continue };

        // Le trou de cette ligne, s'il y en a un.
        let trou = if largeur_interne > 0 && hauteur_interne > 0 && ly >= inset {
            bornes_ligne(largeur_interne, hauteur_interne, radius_interne, ly - inset)
                .map(|(a, b)| (a + inset, b + inset))
        } else { None };

        let pose = |a: i32, b: i32, couverts: &mut usize, segment: &mut F| {
            let x0 = (rect.x + a).max(area.x);
            let x1 = (rect.x + b).min(area.right());
            if x1 > x0 {
                *couverts += (x1 - x0) as usize;
                segment(x0, y, (x1 - x0) as u32);
            }
        };

        match trou {
            Some((ti, tf)) if tf > ti => {
                pose(debut, fin.min(ti), &mut couverts, &mut segment);
                pose(debut.max(tf), fin, &mut couverts, &mut segment);
            }
            _ => pose(debut, fin, &mut couverts, &mut segment),
        }
    }
    couverts
}

/// Visits only pixels in `rect ∩ clip`; returns the number considered so host
/// tests can prove that sparse damage bounds CPU work.
///
/// Ecrite SUR les segments : elle ne peut donc pas dessiner autre chose que ce
/// que le compositeur dessine.
pub fn fill_rounded_rect<F: FnMut(i32, i32)>(rect: Rect, radius: u32, clip: Rect,
    mut paint: F) -> usize {
    spans_rounded_rect(rect, radius, clip, |x, y, largeur| {
        for dx in 0..largeur as i32 { paint(x + dx, y) }
    })
}

pub fn stroke_rounded_rect<F: FnMut(i32, i32)>(rect: Rect, radius: u32,
    thickness: u32, clip: Rect, mut paint: F) -> usize {
    spans_stroke_rounded_rect(rect, radius, thickness, clip, |x, y, largeur| {
        for dx in 0..largeur as i32 { paint(x + dx, y) }
    })
}

/// La forme analytique et la definition par pixel decrivent-elles la meme
/// chose ? Exposee pour que le test d'hote puisse le verifier exhaustivement.
pub fn ligne_conforme(largeur: i32, hauteur: i32, radius: i32, y: i32) -> bool {
    let attendu: alloc::vec::Vec<i32> = (0..largeur)
        .filter(|&x| inside_rounded(x, y, largeur, hauteur, radius))
        .collect();
    match bornes_ligne(largeur, hauteur, radius, y) {
        None => attendu.is_empty(),
        Some((debut, fin)) => {
            if attendu.is_empty() { return false }
            // La forme doit etre un segment CONTIGU, sinon la rasterisation par
            // segments ne peut pas etre exacte.
            let contigu = attendu.windows(2).all(|paire| paire[1] == paire[0] + 1);
            contigu && attendu[0] == debut && *attendu.last().unwrap() == fin - 1
        }
    }
}
