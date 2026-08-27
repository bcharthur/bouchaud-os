//! Decoupe du texte : quelle partie d'un glyphe la trame va-t-elle presenter ?
//!
//! # Le cout que ce module supprime
//!
//! `blit_glyph` parcourait TOUTE la couverture du glyphe et appelait
//! `fb::blend_rgb` pour chaque pixel non nul. Ce writer relit la decoupe --
//! quatre chargements atomiques -- puis rejette le pixel s'il est dehors.
//!
//! Le compositeur dessine une fois par RECTANGLE DE DEGAT. Un mouvement de
//! souris au-dessus du bureau produit deux petits rectangles ; pour chacun, le
//! titre de chaque fenetre, les trois champs de la barre du haut, l'horloge,
//! les libelles de la barre des taches et ceux des icones etaient rasterises
//! puis jetes pixel par pixel. Rien ne le montrait : `blend_rgb` ne compte pas
//! `PIXELS_DESSINES`, donc ce travail n'apparaissait dans aucune metrique.
//!
//! # Ce qui reste vrai
//!
//! La decoupe demeure appliquee par le writer (`BOUCHAUD_GUI_CLIP_V1` : un seul
//! endroit ou tous les pixels passent). Ce module ne l'affaiblit pas : il
//! calcule la meme intersection EN AMONT, pour ne pas appeler le writer sur des
//! pixels qu'il rejettera. Un bug ici ne peut donc pas faire dessiner hors de
//! la decoupe -- au pire, il fait dessiner moins.
//!
//! C'est pourquoi la fonction est pure et vit ici, incluable par un test
//! d'hote : `tools/gui/test_texte.rs` la compare exhaustivement au balayage
//! naif qu'elle remplace.

/// Bornes `[x0, x1) x [y0, y1)` d'une decoupe, comme `fb::clip_rect`.
pub type Decoupe = (usize, usize, usize, usize);

/// Portion VISIBLE d'un glyphe pose en `(gx0, gy0)`, en coordonnees LOCALES au
/// glyphe : `(colonnes, lignes)` sous la forme `(debut, fin)` exclusive.
///
/// `None` quand rien du glyphe ne tombe dans la decoupe -- le cas de loin le
/// plus frequent des qu'un degat est petit.
///
/// `debord_gras` est le nombre de colonnes que le rendu gras ajoute a droite
/// de chaque pixel couvert (1 en gras, 0 sinon) : sans lui, la derniere colonne
/// d'un glyphe gras serait culled alors qu'elle deborde dans la decoupe.
pub fn portion_visible(
    gx0: i32, gy0: i32, largeur: usize, hauteur: usize,
    debord_gras: usize, ecran: (usize, usize), decoupe: Decoupe,
) -> Option<((usize, usize), (usize, usize))> {
    if largeur == 0 || hauteur == 0 {
        return None;
    }
    let (ecran_l, ecran_h) = ecran;
    let (cx0, cy0, cx1, cy1) = decoupe;
    // La decoupe est deja bornee a l'ecran par le pilote, mais un appelant
    // d'hote peut en fournir une plus large : on garde l'invariant ici.
    let cx0 = cx0.min(ecran_l);
    let cy0 = cy0.min(ecran_h);
    let cx1 = cx1.min(ecran_l);
    let cy1 = cy1.min(ecran_h);
    if cx1 <= cx0 || cy1 <= cy0 {
        return None;
    }

    // Lignes : `py = gy0 + ry` doit tomber dans `[cy0, cy1)`.
    let y0 = borne_locale(gy0, cy0);
    let y1 = fin_locale(gy0, cy1, hauteur);
    if y1 <= y0 {
        return None;
    }

    // Colonnes : `px = gx0 + rx` ecrit en `px` et, en gras, en `px + 1`. Une
    // colonne est donc utile des que `px + debord_gras >= cx0`.
    //
    // Le plancher `px >= 0` est SEPARE, et il ne se deduit pas du precedent :
    // le balayage d'origine ecartait la colonne entiere des que `px` sortait de
    // l'ecran par la gauche, y compris le pixel gras qu'elle aurait pose en
    // `px + 1`. Un glyphe pose en `x = -1` ne peint donc rien, meme en gras.
    // C'est un cas de bord invisible en pratique -- aucun texte ne commence
    // hors ecran -- mais la decoupe rapide doit ecrire EXACTEMENT les memes
    // pixels que le balayage qu'elle remplace, sans quoi ce test d'equivalence
    // ne prouve rien.
    let x0 = borne_locale(gx0.saturating_add(debord_gras as i32), cx0)
        .max(borne_locale(gx0, 0));
    let x1 = fin_locale(gx0, cx1, largeur);
    if x1 <= x0 {
        return None;
    }

    Some(((x0, x1), (y0, y1)))
}

/// Premiere coordonnee locale dont l'image ecran atteint `borne`.
fn borne_locale(origine: i32, borne: usize) -> usize {
    if origine >= borne as i32 {
        0
    } else {
        // `borne - origine` tient dans usize : origine < borne.
        (borne as i64 - origine as i64) as usize
    }
}

/// Premiere coordonnee locale dont l'image ecran a atteint `borne`, plafonnee
/// a `taille`.
fn fin_locale(origine: i32, borne: usize, taille: usize) -> usize {
    if origine >= borne as i32 {
        return 0;
    }
    let reste = borne as i64 - origine as i64;
    if reste >= taille as i64 { taille } else { reste as usize }
}

/// Une ligne de texte peut-elle produire un seul pixel dans la decoupe ?
///
/// Test de BANDE, fait une fois par chaine plutot qu'une fois par glyphe : le
/// titre d'une fenetre n'a aucune raison d'etre parcouru caractere par
/// caractere quand le degat est le curseur, trois cents pixels plus bas.
///
/// `sommet` et `hauteur` decrivent la bande verticale que la chaine occupe au
/// plus. Volontairement genereuse : mieux vaut dessiner une chaine invisible
/// que d'en manquer une visible.
pub fn bande_visible(sommet: i32, hauteur: usize, decoupe: Decoupe) -> bool {
    let (cx0, cy0, cx1, cy1) = decoupe;
    if cx1 <= cx0 || cy1 <= cy0 {
        return false;
    }
    let bas = sommet as i64 + hauteur as i64;
    bas > cy0 as i64 && (sommet as i64) < cy1 as i64
}
