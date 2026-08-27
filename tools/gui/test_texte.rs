//! Decoupe du texte : la version rapide voit-elle exactement les memes pixels ?
//!
//! # Ce qui est en jeu
//!
//! `blit_glyph` parcourait toute la couverture d'un glyphe et laissait
//! `fb::blend_rgb` rejeter, pixel par pixel, ce qui tombait hors de la decoupe.
//! Elle demande maintenant a `texte::portion_visible` quelles lignes et quelles
//! colonnes valent la peine, et ne soumet que celles-la.
//!
//! Le gain n'a d'interet que si l'ensemble des pixels ecrits est INCHANGE. Un
//! ecart en moins efface du texte a l'ecran sans rien casser ailleurs -- le
//! genre de defaut qu'aucune assertion de geometrie n'attrape et qu'on ne voit
//! qu'en regardant un titre de fenetre tronque.
//!
//! Ce test compare donc, exhaustivement, l'ensemble calcule a l'ensemble
//! obtenu par le balayage naif qu'il remplace.
//!
//! Lance par `tools/gui/test-texte.sh`.

extern crate alloc;

#[path = "../../src/gui/texte.rs"]
mod texte;

use texte::{bande_visible, portion_visible, Decoupe};

const L: usize = 64;
const H: usize = 48;

/// Le balayage NAIF : exactement ce que faisait `blit_glyph` avant.
///
/// Rend la liste des pixels ecran qu'il aurait melanges, dans l'ordre.
fn balayage_naif(
    gx0: i32, gy0: i32, largeur: usize, hauteur: usize, gras: bool, decoupe: Decoupe,
) -> alloc::vec::Vec<(usize, usize)> {
    let (cx0, cy0, cx1, cy1) = decoupe;
    let mut vus = alloc::vec::Vec::new();
    for ry in 0..hauteur {
        let py = gy0 + ry as i32;
        if py < 0 || py as usize >= H { continue }
        for rx in 0..largeur {
            let px = gx0 + rx as i32;
            if px < 0 || px as usize >= L { continue }
            let (px, py) = (px as usize, py as usize);
            // `blend_rgb` : le pixel n'est ecrit que dans la decoupe.
            if px >= cx0 && px < cx1 && py >= cy0 && py < cy1 {
                vus.push((px, py));
            }
            if gras && px + 1 < L && px + 1 >= cx0 && px + 1 < cx1
                && py >= cy0 && py < cy1 {
                vus.push((px + 1, py));
            }
        }
    }
    vus
}

/// Le balayage RAPIDE : ce que `blit_glyph` fait maintenant.
fn balayage_rapide(
    gx0: i32, gy0: i32, largeur: usize, hauteur: usize, gras: bool, decoupe: Decoupe,
) -> alloc::vec::Vec<(usize, usize)> {
    let debord = if gras { 1 } else { 0 };
    let mut vus = alloc::vec::Vec::new();
    let Some(((x0, x1), (y0, y1))) =
        portion_visible(gx0, gy0, largeur, hauteur, debord, (L, H), decoupe)
    else { return vus };
    let (cx0, cy0, cx1, cy1) = decoupe;

    for ry in y0..y1 {
        let py = (gy0 + ry as i32) as usize;
        for rx in x0..x1 {
            let px = (gx0 + rx as i32) as usize;
            // Le writer applique toujours la decoupe : on la rejoue ici, comme
            // `blend_span` le fait, pour comparer ce qui est REELLEMENT ecrit.
            for candidat in [px, px + 1] {
                if candidat >= cx0 && candidat < cx1 && py >= cy0 && py < cy1 {
                    vus.push((candidat, py));
                }
                if !gras { break }
            }
        }
    }
    vus
}

fn trie(mut v: alloc::vec::Vec<(usize, usize)>) -> alloc::vec::Vec<(usize, usize)> {
    v.sort_unstable();
    v.dedup();
    v
}

// ─── L'equivalence ─────────────────────────────────────────────────────────

/// LA propriete. Exhaustive sur des positions qui couvrent tous les cas de
/// bord : glyphe entierement dedans, a cheval sur chaque cote, entierement
/// dehors, et a cheval sur deux cotes a la fois.
#[test]
fn la_decoupe_rapide_ecrit_exactement_les_memes_pixels() {
    let decoupes: [Decoupe; 5] = [
        (0, 0, L, H),          // pleine
        (20, 15, 40, 30),      // au centre
        (0, 0, 1, 1),          // un pixel
        (30, 0, 64, 48),       // collee a droite
        (0, 40, 64, 48),       // collee en bas
    ];
    for decoupe in decoupes {
        for gras in [false, true] {
            for (largeur, hauteur) in [(1usize, 1usize), (3, 5), (9, 12), (17, 3)] {
                for gy0 in -14i32..(H as i32 + 2) {
                    for gx0 in -18i32..(L as i32 + 2) {
                        let naif = trie(balayage_naif(gx0, gy0, largeur, hauteur, gras, decoupe));
                        let rapide = trie(balayage_rapide(gx0, gy0, largeur, hauteur, gras, decoupe));
                        assert_eq!(
                            naif, rapide,
                            "divergence en ({gx0},{gy0}) {largeur}x{hauteur} \
                             gras={gras} decoupe={decoupe:?}"
                        );
                    }
                }
            }
        }
    }
}

/// Un glyphe vide ne produit rien -- et surtout, pas une plage inversee.
#[test]
fn un_glyphe_vide_ne_produit_rien() {
    assert!(portion_visible(0, 0, 0, 10, 0, (L, H), (0, 0, L, H)).is_none());
    assert!(portion_visible(0, 0, 10, 0, 0, (L, H), (0, 0, L, H)).is_none());
}

/// Une decoupe vide -- ce que pose le compositeur quand un degat sort de
/// l'ecran -- ne doit rien laisser passer.
#[test]
fn une_decoupe_vide_ne_laisse_rien_passer() {
    for decoupe in [(10, 10, 10, 20), (10, 10, 20, 10), (30, 30, 5, 5)] {
        assert!(portion_visible(0, 0, 20, 20, 0, (L, H), decoupe).is_none(),
            "decoupe vide {decoupe:?}");
        assert!(!bande_visible(0, 20, decoupe), "bande, decoupe vide {decoupe:?}");
    }
}

/// Les bornes rendues sont toujours DANS le glyphe : `blit_glyph` s'en sert
/// pour indexer `g.cov`, un depassement serait une panique noyau.
#[test]
fn les_bornes_restent_dans_le_glyphe() {
    for gx0 in -30i32..90 {
        for gy0 in -30i32..70 {
            let Some(((x0, x1), (y0, y1))) =
                portion_visible(gx0, gy0, 11, 13, 1, (L, H), (5, 6, 50, 40))
            else { continue };
            assert!(x0 < x1 && x1 <= 11, "colonnes {x0}..{x1} hors du glyphe");
            assert!(y0 < y1 && y1 <= 13, "lignes {y0}..{y1} hors du glyphe");
            // Et l'origine ecran doit etre positive : `blit_glyph` la convertit
            // en `usize` sans verification.
            assert!(gx0 + x0 as i32 >= 0, "abscisse ecran negative");
            assert!(gy0 + y0 as i32 >= 0, "ordonnee ecran negative");
        }
    }
}

// ─── Le test de bande ──────────────────────────────────────────────────────

/// La bande doit etre CONSERVATRICE : elle peut laisser passer une chaine
/// invisible, jamais rejeter une chaine visible.
#[test]
fn la_bande_ne_rejette_jamais_une_ligne_visible() {
    let decoupe: Decoupe = (0, 20, L, 30);
    for sommet in -20i32..60 {
        for hauteur in [1usize, 9, 22] {
            let visible = (sommet..(sommet + hauteur as i32))
                .any(|y| y >= 20 && y < 30);
            if visible {
                assert!(bande_visible(sommet, hauteur, decoupe),
                    "bande {sommet}+{hauteur} rejetee alors qu'elle est visible");
            }
        }
    }
}

/// Et elle doit quand meme rejeter : une bande franchement ailleurs ne doit
/// pas passer, sinon le test ne sert a rien.
#[test]
fn la_bande_rejette_ce_qui_est_ailleurs() {
    let decoupe: Decoupe = (0, 20, L, 30);
    assert!(!bande_visible(0, 10, decoupe), "au-dessus");
    assert!(!bande_visible(30, 10, decoupe), "en dessous");
    assert!(!bande_visible(-40, 10, decoupe), "loin au-dessus");
}

/// Le cas qui motive tout : le curseur en bas de l'ecran ne doit pas faire
/// rasteriser le titre d'une fenetre situee en haut.
#[test]
fn un_degat_de_curseur_ne_reveille_pas_le_texte_du_haut() {
    // Le curseur, 14x22, tout en bas.
    let curseur: Decoupe = (300, 690, 314, 712);
    // Le titre d'une fenetre, hauteur 11, pose a y = 40.
    assert!(!bande_visible(40, 33, curseur));
    // Et un glyphe de ce titre ne produit aucune ligne.
    assert!(portion_visible(120, 44, 9, 12, 0, (1280, 720), curseur).is_none());
}

/// LES BORNES DOIVENT ETRE SERREES.
///
/// L'equivalence seule ne suffit pas : des bornes trop LARGES ecrivent les
/// memes pixels -- le writer rejette le surplus -- tout en ne culling rien.
/// Le test passerait au vert pendant que le cout reste entier.
///
/// Une borne est serree quand sa premiere et sa derniere ligne, sa premiere et
/// sa derniere colonne, produisent chacune au moins un pixel ecrit.
#[test]
fn les_bornes_sont_serrees() {
    let decoupes: [Decoupe; 4] = [
        (0, 0, L, H), (20, 15, 40, 30), (0, 40, 64, 48), (30, 0, 64, 48),
    ];
    for decoupe in decoupes {
        let (cx0, cy0, cx1, cy1) = decoupe;
        for gras in [false, true] {
            let debord = if gras { 1 } else { 0 };
            for (largeur, hauteur) in [(1usize, 1usize), (9, 12), (17, 3)] {
                for gy0 in -14i32..(H as i32 + 2) {
                    for gx0 in -18i32..(L as i32 + 2) {
                        let Some(((x0, x1), (y0, y1))) = portion_visible(
                            gx0, gy0, largeur, hauteur, debord, (L, H), decoupe)
                        else { continue };

                        let haut = gy0 + y0 as i32;
                        let bas = gy0 + y1 as i32 - 1;
                        assert!(haut >= cy0 as i32 && (haut as usize) < cy1,
                            "premiere ligne {haut} hors decoupe ({gx0},{gy0})");
                        assert!(bas >= cy0 as i32 && (bas as usize) < cy1,
                            "derniere ligne {bas} hors decoupe ({gx0},{gy0})");

                        // La premiere colonne doit ecrire quelque part : son
                        // pixel propre, ou -- en gras -- son voisin de droite.
                        let gauche = gx0 + x0 as i32;
                        assert!(gauche >= 0, "colonne {gauche} hors ecran");
                        assert!(
                            (gauche as usize + debord) >= cx0 && (gauche as usize) < cx1,
                            "premiere colonne {gauche} n'ecrit rien ({gx0},{gy0}) \
                             gras={gras} decoupe={decoupe:?}"
                        );
                        let droite = gx0 + x1 as i32 - 1;
                        assert!(
                            (droite as usize) < cx1 && (droite as usize + debord) >= cx0,
                            "derniere colonne {droite} n'ecrit rien ({gx0},{gy0})"
                        );
                    }
                }
            }
        }
    }
}
