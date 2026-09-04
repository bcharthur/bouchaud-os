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
/// `opaque` : rectangle que l'appelant GARANTIT repeindre opaque plus tard
/// dans la trame. Le fond de surface ne l'ecrit pas. Voir
/// `BOUCHAUD_C13_PAS_DEUX_FOIS_LE_MEME_PIXEL_V1`.
///
/// Il ne concerne QUE le fond : les anneaux d'ombre sont hors de la fenetre et
/// le filet de bordure est en dehors de la zone utile (`client_rect` est
/// retrait de `WINDOW_BORDER` de chaque cote), donc aucun des deux ne peut
/// tomber dans l'exclusion.
/// `remplit` recoit `(x, y, largeur, couleur, couverture)`.
///
/// # Ce qui est anti-crenele, et ce qui ne l'est pas
///
/// Le fond de la fenetre et son filet de bordure le sont : ce sont eux qui se
/// detachent du bureau, et leurs quatre coins etaient la derniere marche
/// d'escalier visible du systeme.
///
/// Les huit anneaux d'ombre restent binaires, et c'est un choix mesure. Leurs
/// couleurs vont de `0x080a0e` a `0x0f1115` ; le fond du bureau vaut
/// `COLOR_BACKGROUND` = `0x0e1116`. L'ecart tient dans un pas de quantification
/// par canal : une rampe y serait rigoureusement invisible. Elle couterait en
/// revanche NEUF formes parcourues pixel par pixel dans leurs bandes de coins
/// au lieu de deux -- pour rien.
pub fn paint_window_shape_spans<F: FnMut(i32, i32, u32, u32, u8)>(
    geometry: crate::gui::windowing::WindowRenderGeometry,
    radius: u32, shadow_extent: u32, clip: Rect, surface: u32, border: u32,
    opaque: Option<Rect>,
    mut remplit: F,
) {
    for extent in (1..=shadow_extent).rev() {
        let shadow = geometry.outer.outset(extent);
        let shade = 0x07090d + extent * 0x010101;
        spans_stroke_rounded_rect(shadow, radius + extent, 1, clip,
            |x, y, largeur| remplit(x, y, largeur, shade, 255));
    }
    spans_rounded_rect_aa_sauf(geometry.outer, radius, clip,
        opaque.unwrap_or(Rect::new(0, 0, 0, 0)),
        |x, y, largeur, couverture| remplit(x, y, largeur, surface, couverture));
    spans_stroke_rounded_rect_aa(geometry.outer, radius, 1, clip,
        |x, y, largeur, couverture| remplit(x, y, largeur, border, couverture));
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

// BOUCHAUD_C13_PAS_DEUX_FOIS_LE_MEME_PIXEL_V1
//
// LE PIXEL PEINT DEUX FOIS, ET POURQUOI IL NE SE VOYAIT PAS
// ---------------------------------------------------------
// `[GUI-DAMAGE]` annonce `drawn_pixels` proche du TRIPLE de
// `presented_pixels` : chaque pixel presente a l'ecran a ete ecrit environ
// trois fois. Une des trois est structurelle et evitable.
//
// `paint_window_shape_spans` remplit la forme ENTIERE de la fenetre en couleur
// de surface -- zone de contenu comprise. Puis `compose_client` recopie
// par-dessus la surface du client, integralement et de facon opaque. Le
// remplissage n'a donc jamais ete vu : il a ete recouvert a cent pour cent, a
// chaque trame, pour une fenetre de navigateur de 1100x604, soit 664 400
// pixels ecrits pour rien.
//
// C'est le genre de depense qu'aucune capture d'ecran ne revele -- le resultat
// est rigoureusement identique -- et que seul le rapport entre pixels dessines
// et pixels presentes met en evidence.
//
// La reparation ne change pas la forme peinte : elle lui retire un rectangle
// dont on sait qu'il sera recouvert. Une ligne qui traverse ce rectangle rend
// alors deux segments au lieu d'un, exactement comme le fait deja le contour
// autour de son trou.
//
// L'exclusion est une PROMESSE de l'appelant : « ce rectangle sera repeint
// opaque avant la fin de la trame ». Seul `draw_window` la formule, et
// seulement pour une fenetre de client ring 3, dont la surface recouvre la
// zone utile. Une application native qui ne peint pas tous ses pixels
// continue, elle, de recevoir son fond.

/// Segments de `rect ∩ clip`, PRIVES de `exclusion`.
///
/// Meme contrat que [`spans_rounded_rect`] : le nombre de pixels reellement
/// couverts est rendu, ce qui permet a un test d'hote de prouver le gain sans
/// regarder l'ecran.
pub fn spans_rounded_rect_sauf<F: FnMut(i32, i32, u32)>(rect: Rect, radius: u32, clip: Rect,
    exclusion: Rect, mut segment: F) -> usize {
    let Some(area) = intersection(rect, clip) else { return 0 };
    let mut couverts = 0usize;
    for y in area.y..area.bottom() {
        let Some((debut, fin)) = bornes_ligne(rect.width as i32, rect.height as i32,
            radius as i32, y - rect.y) else { continue };
        let x0 = (rect.x + debut).max(area.x);
        let x1 = (rect.x + fin).min(area.right());
        if x1 <= x0 { continue }

        // Hors des lignes de l'exclusion, rien ne change.
        let coupe = exclusion.width > 0 && exclusion.height > 0
            && y >= exclusion.y && y < exclusion.bottom();
        if !coupe {
            couverts += (x1 - x0) as usize;
            segment(x0, y, (x1 - x0) as u32);
            continue;
        }

        let mut pose = |a: i32, b: i32| {
            if b > a {
                couverts += (b - a) as usize;
                segment(a, y, (b - a) as u32);
            }
        };
        pose(x0, x1.min(exclusion.x));
        pose(x0.max(exclusion.right()), x1);
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

// ---------------------------------------------------------------------------
// BOUCHAUD_C12_ANTICRENELAGE_V1 -- la couverture, et non plus l'appartenance
// ---------------------------------------------------------------------------
//
// `bornes_ligne` rend un segment BINAIRE : un pixel est dedans ou dehors. Un
// coin arrondi devient donc un escalier, et cet escalier se voit sur tout ce
// que le chrome dessine -- barre d'URL, boutons, onglets, bords de fenetre --
// parce que tout passe par ce rasteriseur.
//
// La reparation ne change pas la forme : elle change ce qu'on mesure. Au lieu
// de demander « ce pixel est-il dedans ? », on demande « quelle FRACTION de ce
// pixel la forme couvre-t-elle ? ». Le bord cesse alors d'etre une marche pour
// devenir une rampe, et l'oeil ne voit plus de pixel.
//
// La couverture est analytique, pas echantillonnee : pour un pixel dont le
// centre est a distance `d` du centre de l'arc de rayon `r`, la fraction
// couverte vaut `clamp(r - d + 1/2, 0, 1)`. C'est la surface du demi-plan
// tangent, exacte au premier ordre et sans le cout d'un sur-echantillonnage.
//
// Tout est en entiers. Les centres de pixels tombent sur des demis, donc on
// travaille en DEMI-PIXELS : le centre du pixel `x` est `2x + 1`, et le rayon
// `r` vaut `2r`. La formule devient `(2r - d2 + 1) / 2`, exprimee en 255emes.

/// Couverture de la forme sur le pixel `(x, y)`, de 0 (dehors) a 255 (dedans).
///
/// Les coordonnees sont LOCALES a la forme. `radius` est borne comme dans
/// `bornes_ligne`, pour que les deux decrivent le meme objet.
pub fn couverture_pixel(x: i32, y: i32, largeur: i32, hauteur: i32, radius: i32) -> u8 {
    if largeur <= 0 || hauteur <= 0 { return 0 }
    if x < 0 || y < 0 || x >= largeur || y >= hauteur { return 0 }
    let radius = radius.max(0).min(largeur.min(hauteur) / 2);
    if radius == 0 { return 255 }

    // MEMES centres que `inside_rounded`, et memes indices de pixel. Les deux
    // doivent decrire le meme cercle : un fond anti-crenele et une bordure
    // binaire dessines sur la meme forme ne doivent pas se decaler.
    let cx = if x < radius { radius - 1 }
        else if x >= largeur - radius { largeur - radius }
        else { return 255 };
    let cy = if y < radius { radius - 1 }
        else if y >= hauteur - radius { hauteur - radius }
        else { return 255 };

    let dx = x - cx;
    let dy = y - cy;

    // `racine` tronque, donc on l'appelle sur une distance MISE A L'ECHELLE :
    // en seiziemes de pixel, l'erreur de troncature vaut 1/16 de pixel au lieu
    // d'un pixel entier -- assez fin pour que la rampe n'ait pas de marche.
    let distance = racine((dx * dx + dy * dy).saturating_mul(ECHELLE * ECHELLE));

    // couverture = radius - d + 1/2, borne a [0, 1].
    //
    // Au bord exact de la forme binaire (`d == radius`) elle vaut 1/2 : le
    // pixel que l'ancien rasteriseur allumait entierement est desormais a
    // moitie couvert, ce qui est precisement ce qui efface la marche.
    let numerateur = ECHELLE * radius - distance + ECHELLE / 2;
    if numerateur <= 0 { return 0 }
    let couverture = (numerateur * 255) / ECHELLE;
    if couverture >= 255 { 255 } else { couverture as u8 }
}

/// Seiziemes de pixel : la finesse a laquelle la rampe est calculee.
const ECHELLE: i32 = 16;

/// Segments de `rect ∩ clip` AVEC leur couverture.
///
/// Les lignes hors des bandes de coins sortent en un seul segment pleinement
/// couvert : le cout par ligne reste celui du chemin binaire. Seules les deux
/// bandes de coins sont parcourues pixel par pixel, et les couvertures egales
/// y sont regroupees -- un bord franc y redonne donc un segment unique.
///
/// Rend le nombre de pixels touches, pour que les tests d'hote puissent
/// affirmer que la decoupe borne bien le travail.
pub fn spans_rounded_rect_aa<F: FnMut(i32, i32, u32, u8)>(rect: Rect, radius: u32,
    clip: Rect, segment: F) -> usize {
    spans_rounded_rect_aa_sauf(rect, radius, clip, Rect::new(0, 0, 0, 0), segment)
}

/// [`spans_rounded_rect_aa`], prive de `exclusion`.
///
/// Meme promesse que [`spans_rounded_rect_sauf`] : l'appelant garantit que ce
/// rectangle sera repeint opaque avant la fin de la trame. Voir
/// `BOUCHAUD_C13_PAS_DEUX_FOIS_LE_MEME_PIXEL_V1`.
pub fn spans_rounded_rect_aa_sauf<F: FnMut(i32, i32, u32, u8)>(rect: Rect, radius: u32,
    clip: Rect, exclusion: Rect, mut segment: F) -> usize {
    let Some(area) = intersection(rect, clip) else { return 0 };
    let exclue = |x: i32, y: i32| {
        exclusion.width > 0 && exclusion.height > 0
            && x >= exclusion.x && x < exclusion.right()
            && y >= exclusion.y && y < exclusion.bottom()
    };
    let largeur = rect.width as i32;
    let hauteur = rect.height as i32;
    let radius = (radius as i32).max(0).min(largeur.min(hauteur) / 2);
    let mut touches = 0usize;

    for y in area.y..area.bottom() {
        let ly = y - rect.y;

        // Bande centrale : aucun arc ne la traverse, donc aucune couverture
        // partielle a calculer.
        if radius == 0 || (ly >= radius && ly < hauteur - radius) {
            let x0 = area.x;
            let x1 = area.right();
            if x1 <= x0 { continue }
            let coupe = exclusion.width > 0 && exclusion.height > 0
                && y >= exclusion.y && y < exclusion.bottom();
            if !coupe {
                touches += (x1 - x0) as usize;
                segment(x0, y, (x1 - x0) as u32, 255);
                continue;
            }
            let mut pose = |a: i32, b: i32| {
                if b > a {
                    touches += (b - a) as usize;
                    segment(a, y, (b - a) as u32, 255);
                }
            };
            pose(x0, x1.min(exclusion.x));
            pose(x0.max(exclusion.right()), x1);
            continue;
        }

        // Bande de coin : on part de l'extension binaire elargie d'un pixel de
        // chaque cote -- la rampe deborde d'exactement un pixel -- puis on
        // regroupe les couvertures egales.
        let Some((debut, fin)) = bornes_ligne(largeur, hauteur, radius, ly) else { continue };
        let x0 = (rect.x + debut - 1).max(area.x);
        let x1 = (rect.x + fin + 1).min(area.right());

        let mut course_debut = x0;
        let mut course_valeur = 0u8;
        let mut course_active = false;
        for x in x0..x1 {
            // Un pixel exclu se comporte comme un pixel de couverture nulle :
            // il interrompt la course en cours et n'en ouvre aucune.
            let c = if exclue(x, y) { 0 }
                else { couverture_pixel(x - rect.x, ly, largeur, hauteur, radius) };
            if course_active && c == course_valeur { continue }
            if course_active && course_valeur != 0 {
                touches += (x - course_debut) as usize;
                segment(course_debut, y, (x - course_debut) as u32, course_valeur);
            }
            course_debut = x;
            course_valeur = c;
            course_active = true;
        }
        if course_active && course_valeur != 0 && x1 > course_debut {
            touches += (x1 - course_debut) as usize;
            segment(course_debut, y, (x1 - course_debut) as u32, course_valeur);
        }
    }
    touches
}

// BOUCHAUD_C13_CONTOUR_SANS_MARCHE_V1
//
// LE DERNIER CRENELAGE
// --------------------
// Le chrome (barre d'URL, boutons, onglets, barre de titre) est anti-crenele
// depuis `BOUCHAUD_C12_CHROME_SANS_MARCHE_V1`. Le texte l'a toujours ete. Il
// restait UNE forme binaire, et c'est la plus regardee : la silhouette de la
// fenetre elle-meme -- son fond arrondi et le filet qui l'entoure --, dont les
// quatre coins montraient encore un escalier sur le fond du bureau.
//
// Un contour est une DIFFERENCE de deux formes. Sa couverture aussi : ce que
// couvre la forme exterieure, moins ce que couvre la forme interieure. Un
// pixel au milieu du filet est dans l'une et pas dans l'autre (255 - 0), un
// pixel du trou est dans les deux (255 - 255 = 0), et les deux bords rendent
// leur rampe sans qu'aucun cas particulier ne soit ecrit.

/// Couverture du CONTOUR sur le pixel `(x, y)`, de 0 a 255.
///
/// Coordonnees LOCALES a la forme exterieure. Le rayon interieur suit la meme
/// regle que `spans_stroke_rounded_rect` : `radius - epaisseur`.
pub fn couverture_contour(x: i32, y: i32, largeur: i32, hauteur: i32,
    radius: i32, epaisseur: i32) -> u8 {
    let dehors = couverture_pixel(x, y, largeur, hauteur, radius);
    if dehors == 0 || epaisseur <= 0 { return dehors }
    let dedans = couverture_pixel(
        x - epaisseur, y - epaisseur,
        largeur - epaisseur * 2, hauteur - epaisseur * 2,
        radius - epaisseur,
    );
    dehors.saturating_sub(dedans)
}

/// Segments du CONTOUR avec leur couverture.
///
/// Hors des bandes de coins, les deux bords sont des rectangles a angle droit :
/// aucune couverture partielle a calculer, et la ligne rend ses deux segments
/// pleins comme le chemin binaire. Seules les deux bandes de coins sont
/// parcourues pixel par pixel, avec regroupement des couvertures egales.
pub fn spans_stroke_rounded_rect_aa<F: FnMut(i32, i32, u32, u8)>(rect: Rect, radius: u32,
    thickness: u32, clip: Rect, mut segment: F) -> usize {
    let Some(area) = intersection(rect, clip) else { return 0 };
    let largeur = rect.width as i32;
    let hauteur = rect.height as i32;
    let epaisseur = thickness as i32;
    let radius = (radius as i32).max(0).min(largeur.min(hauteur) / 2);
    let mut touches = 0usize;

    for y in area.y..area.bottom() {
        let ly = y - rect.y;

        // Bande centrale : le bord gauche et le bord droit, francs.
        if radius == 0 || (ly >= radius && ly < hauteur - radius) {
            let mut pose = |a: i32, b: i32| {
                let x0 = (rect.x + a).max(area.x);
                let x1 = (rect.x + b).min(area.right());
                if x1 > x0 {
                    touches += (x1 - x0) as usize;
                    segment(x0, y, (x1 - x0) as u32, 255);
                }
            };
            if epaisseur * 2 >= largeur {
                pose(0, largeur);
            } else {
                pose(0, epaisseur);
                pose(largeur - epaisseur, largeur);
            }
            continue;
        }

        // Bande de coin. Meme extension d'un pixel que le remplissage : la
        // rampe deborde d'exactement un pixel de part et d'autre de la forme
        // binaire.
        let Some((debut, fin)) = bornes_ligne(largeur, hauteur, radius, ly) else { continue };
        let x0 = (rect.x + debut - 1).max(area.x);
        let x1 = (rect.x + fin + 1).min(area.right());

        let mut course_debut = x0;
        let mut course_valeur = 0u8;
        let mut course_active = false;
        for x in x0..x1 {
            let c = couverture_contour(x - rect.x, ly, largeur, hauteur, radius, epaisseur);
            if course_active && c == course_valeur { continue }
            if course_active && course_valeur != 0 {
                touches += (x - course_debut) as usize;
                segment(course_debut, y, (x - course_debut) as u32, course_valeur);
            }
            course_debut = x;
            course_valeur = c;
            course_active = true;
        }
        if course_active && course_valeur != 0 && x1 > course_debut {
            touches += (x1 - course_debut) as usize;
            segment(course_debut, y, (x1 - course_debut) as u32, course_valeur);
        }
    }
    touches
}

/// Melange `dessus` sur `fond` selon `couverture` (0 = fond, 255 = dessus).
///
/// Par canal, sur 8 bits, sans flottant. `255` doit rendre `dessus` EXACTEMENT
/// -- sans quoi une surface pleine deriverait en teinte a chaque recomposition.
pub fn melange(fond: u32, dessus: u32, couverture: u8) -> u32 {
    if couverture == 0 { return fond }
    if couverture == 255 { return dessus }
    let a = couverture as u32;
    let inv = 255 - a;
    let canal = |decalage: u32| {
        let f = (fond >> decalage) & 0xff;
        let d = (dessus >> decalage) & 0xff;
        // +127 : arrondi au plus proche plutot que troncature, sinon un degrade
        // se decale systematiquement vers le fond.
        (((d * a + f * inv) + 127) / 255) & 0xff
    };
    (canal(16) << 16) | (canal(8) << 8) | canal(0)
}

/// Version pixel de `spans_rounded_rect_aa`, pour les tests d'hote.
///
/// Ecrite SUR les segments : elle ne peut donc pas dessiner autre chose que ce
/// que le compositeur dessine.
pub fn fill_rounded_rect_aa<F: FnMut(i32, i32, u8)>(rect: Rect, radius: u32, clip: Rect,
    mut paint: F) -> usize {
    spans_rounded_rect_aa(rect, radius, clip, |x, y, largeur, couverture| {
        for dx in 0..largeur as i32 { paint(x + dx, y, couverture) }
    })
}
