//! Preuve hote de l'anti-crenelage du chantier 12.
//!
//! Inclut le module de PRODUCTION `src/gui/graphics.rs` tel quel : ce qui est
//! verifie ici est ce que le compositeur dessine.
//!
//! # Ce que l'escalier coutait
//!
//! `bornes_ligne` rend un segment BINAIRE : un pixel est dedans ou dehors. Un
//! coin arrondi etait donc une marche d'escalier, et cette marche se voyait sur
//! tout ce que le chrome dessine -- barre d'URL, boutons, onglets, bords de
//! fenetre -- puisque tout passe par ce rasteriseur.
//!
//! L'anti-crenelage ne change pas la FORME, il change ce qu'on MESURE : au lieu
//! de « ce pixel est-il dedans ? », « quelle fraction de ce pixel la forme
//! couvre-t-elle ? ». Ces tests exigent que la rampe reste fidele a la forme
//! binaire -- pleine a l'interieur, nulle dehors, bornee par elle, et de meme
//! aire.

extern crate alloc;
#[path="../../src/gui/windowing/mod.rs"] mod windowing;
mod gui { pub mod windowing { pub use crate::windowing::*; } }
#[path="../../src/gui/graphics.rs"] mod graphics;

use graphics::{couverture_contour, couverture_pixel, fill_rounded_rect_aa, melange,
    spans_rounded_rect_aa, spans_stroke_rounded_rect, spans_stroke_rounded_rect_aa,
    fill_rounded_rect};
use windowing::Rect;

const GRAND: Rect = Rect::new(0, 0, 4096, 4096);

/// Sans rayon, la forme est un rectangle : aucun pixel ne doit etre partiel.
/// Un bord franc anti-crenele serait un flou, pas une amelioration.
#[test]
fn un_rectangle_sans_rayon_n_a_aucun_pixel_partiel() {
    for y in 0..20 {
        for x in 0..30 {
            assert_eq!(couverture_pixel(x, y, 30, 20, 0), 255,
                "({x},{y}) devrait etre plein");
        }
    }
}

/// Le centre d'une forme arrondie est pleinement couvert. Si la rampe mordait
/// a l'interieur, toutes les surfaces du chrome seraient assombries.
#[test]
fn l_interieur_est_pleinement_couvert() {
    let (l, h, r) = (64, 48, 12);
    for y in r..h - r {
        for x in 0..l {
            assert_eq!(couverture_pixel(x, y, l, h, r), 255,
                "la bande centrale ({x},{y}) doit etre pleine");
        }
    }
}

/// Les quatre coins exterieurs sont hors de la forme.
#[test]
fn les_coins_exterieurs_sont_vides() {
    let (l, h, r) = (64, 48, 16);
    for (x, y) in [(0, 0), (l - 1, 0), (0, h - 1), (l - 1, h - 1)] {
        assert_eq!(couverture_pixel(x, y, l, h, r), 0,
            "le coin ({x},{y}) doit etre hors de la forme");
    }
}

/// La couverture doit etre symetrique par les deux axes : un coin plus doux
/// qu'un autre se verrait immediatement sur un bouton.
#[test]
fn la_couverture_est_symetrique_sur_les_quatre_coins() {
    let (l, h, r) = (40, 40, 14);
    for y in 0..h {
        for x in 0..l {
            let a = couverture_pixel(x, y, l, h, r);
            assert_eq!(a, couverture_pixel(l - 1 - x, y, l, h, r), "miroir horizontal");
            assert_eq!(a, couverture_pixel(x, h - 1 - y, l, h, r), "miroir vertical");
            assert_eq!(a, couverture_pixel(l - 1 - x, h - 1 - y, l, h, r), "miroir double");
        }
    }
}

/// En s'enfoncant dans la forme le long de la diagonale d'un coin, la
/// couverture ne doit jamais REDESCENDRE. Une rampe non monotone produirait un
/// lisere clair sur le bord -- pire que l'escalier qu'elle remplace.
#[test]
fn la_rampe_ne_redescend_jamais_vers_l_interieur() {
    let (l, h, r) = (60, 60, 20);
    let mut precedent = 0u8;
    for d in 0..r {
        let c = couverture_pixel(d, d, l, h, r);
        assert!(c >= precedent,
            "couverture non monotone en ({d},{d}) : {c} < {precedent}");
        precedent = c;
    }
    assert_eq!(precedent, 255, "la diagonale doit finir pleine");
}

/// LA contrainte de fidelite : la rampe ne doit pas deborder de la forme
/// binaire de plus d'un pixel, et tout pixel binairement plein doit rester
/// pleinement couvert. La forme ne bouge pas, seul son bord s'adoucit.
#[test]
fn la_rampe_reste_bornee_par_la_forme_binaire() {
    let (l, h, r) = (48, 36, 10);
    let mut binaire = alloc::vec![false; (l * h) as usize];
    fill_rounded_rect(Rect::new(0, 0, l as u32, h as u32), r as u32, GRAND,
        |x, y| binaire[(y * l + x) as usize] = true);

    for y in 0..h {
        for x in 0..l {
            let c = couverture_pixel(x, y, l, h, r);
            if binaire[(y * l + x) as usize] {
                assert!(c > 0, "({x},{y}) est dans la forme mais de couverture nulle");
            } else {
                // Hors de la forme binaire, seule la frange est toolee.
                let voisin = (-1..=1).any(|dy| (-1..=1).any(|dx| {
                    let (nx, ny) = (x + dx, y + dy);
                    nx >= 0 && ny >= 0 && nx < l && ny < h
                        && binaire[(ny * l + nx) as usize]
                }));
                assert!(c == 0 || voisin,
                    "({x},{y}) couvert a {c} loin de la forme");
            }
        }
    }
}

/// L'aire couverte doit approcher celle de la forme binaire. Un ecart large
/// voudrait dire que la rampe rogne ou gonfle le bouton.
#[test]
fn l_aire_couverte_approche_celle_de_la_forme() {
    for (l, h, r) in [(32u32, 32u32, 8u32), (64, 48, 16), (100, 40, 20), (24, 24, 12)] {
        let mut binaire = 0usize;
        fill_rounded_rect(Rect::new(0, 0, l, h), r, GRAND, |_, _| binaire += 1);

        let mut somme = 0usize;
        fill_rounded_rect_aa(Rect::new(0, 0, l, h), r, GRAND,
            |_, _, c| somme += c as usize);
        let aire_aa = somme / 255;

        let ecart = (aire_aa as i64 - binaire as i64).abs();
        let tolerance = (r as i64 * 2).max(8);
        assert!(ecart <= tolerance,
            "{l}x{h} r{r} : aire anti-crenelee {aire_aa} vs binaire {binaire} \
             (ecart {ecart} > {tolerance})");
    }
}

/// La decoupe doit borner le travail : rien en dehors de `clip`.
#[test]
fn aucun_pixel_ne_sort_de_la_decoupe() {
    let forme = Rect::new(10, 10, 80, 60);
    let clip = Rect::new(20, 20, 30, 20);
    fill_rounded_rect_aa(forme, 12, clip, |x, y, _| {
        assert!(x >= clip.x && x < clip.right() && y >= clip.y && y < clip.bottom(),
            "pixel ({x},{y}) hors de la decoupe");
    });
}

/// Une couverture nulle ne doit jamais etre emise : ce serait du travail pur
/// perdu sur le port memoire, a chaque trame.
#[test]
fn aucun_segment_de_couverture_nulle_n_est_emis() {
    spans_rounded_rect_aa(Rect::new(0, 0, 50, 40), 14, GRAND, |_, _, largeur, c| {
        assert_ne!(c, 0, "segment de couverture nulle emis");
        assert_ne!(largeur, 0, "segment de largeur nulle emis");
    });
}

/// `255` doit rendre la couleur du dessus EXACTEMENT. Sans cela, une surface
/// pleine deriverait en teinte a chaque recomposition.
#[test]
fn une_couverture_pleine_ne_derive_pas() {
    for couleur in [0x000000, 0xffffff, 0x2f6fed, 0x1a1a1e, 0xc84b31] {
        assert_eq!(melange(0x808080, couleur, 255), couleur);
        assert_eq!(melange(couleur, 0x123456, 0), couleur);
        // Idempotence : melanger une couleur sur elle-meme ne la change pas,
        // quelle que soit la couverture.
        for c in [1u8, 64, 128, 200, 254] {
            assert_eq!(melange(couleur, couleur, c), couleur,
                "melange de {couleur:06x} sur lui-meme a couverture {c}");
        }
    }
}

/// Le melange doit rester dans l'intervalle des deux couleurs, canal par
/// canal : un depassement produirait un liseré fluo sur les bords.
#[test]
fn le_melange_reste_entre_les_deux_couleurs() {
    let fond = 0x102030u32;
    let dessus = 0xa0b0c0u32;
    for c in 0..=255u8 {
        let m = melange(fond, dessus, c);
        for decalage in [0u32, 8, 16] {
            let f = (fond >> decalage) & 0xff;
            let d = (dessus >> decalage) & 0xff;
            let v = (m >> decalage) & 0xff;
            assert!(v >= f.min(d) && v <= f.max(d),
                "canal {decalage} hors bornes a couverture {c} : {v}");
        }
    }
}

/// Le melange doit etre monotone en couverture : plus de couverture ne peut
/// pas rapprocher du fond. C'est ce qui garantit un degrade sans marche.
#[test]
fn le_melange_est_monotone_en_couverture() {
    let (fond, dessus) = (0x000000u32, 0xffffffu32);
    let mut precedent = 0u32;
    for c in 0..=255u8 {
        let v = melange(fond, dessus, c) & 0xff;
        assert!(v >= precedent, "melange non monotone a {c} : {v} < {precedent}");
        precedent = v;
    }
    assert_eq!(precedent, 0xff);
}

/// Un rayon plus grand que la moitie du plus petit cote est borne, comme dans
/// le chemin binaire : les deux doivent decrire le meme objet.
#[test]
fn un_rayon_excessif_est_borne_comme_dans_le_chemin_binaire() {
    let (l, h) = (20, 10);
    for rayon in [5, 8, 40, 1000] {
        let borne = couverture_pixel(l / 2, h / 2, l, h, rayon);
        assert_eq!(borne, 255, "le centre reste plein quel que soit le rayon {rayon}");
    }
    // Rayon 5 = h/2 : au-dela, la forme ne change plus.
    for y in 0..h {
        for x in 0..l {
            assert_eq!(couverture_pixel(x, y, l, h, 5),
                       couverture_pixel(x, y, l, h, 999),
                       "({x},{y}) : le rayon doit etre borne a h/2");
        }
    }
}

/// Une forme degeneree ne doit rien couvrir plutot que paniquer.
#[test]
fn les_formes_degenerees_ne_couvrent_rien() {
    assert_eq!(couverture_pixel(0, 0, 0, 10, 4), 0);
    assert_eq!(couverture_pixel(0, 0, 10, 0, 4), 0);
    assert_eq!(couverture_pixel(-1, 0, 10, 10, 4), 0);
    assert_eq!(couverture_pixel(0, 99, 10, 10, 4), 0);
    assert_eq!(spans_rounded_rect_aa(Rect::new(0, 0, 0, 0), 4, GRAND, |_, _, _, _| {}), 0);
}


// ===========================================================================
// BOUCHAUD_C13_CONTOUR_SANS_MARCHE_V1 -- la silhouette de la fenetre
// ===========================================================================
//
// Le dernier crenelage du systeme : le fond arrondi de la fenetre et le filet
// qui l'entoure. Un contour est une DIFFERENCE de deux formes, et sa
// couverture aussi. Ces tests exigent la meme fidelite que pour le
// remplissage -- plein au milieu du filet, nul dans le trou et dehors, borne
// par la forme binaire, et de meme aire.

/// Le contour d'une fenetre reelle : 400x300, rayon 10, filet d'un pixel.
const CADRE: (i32, i32, i32, i32) = (400, 300, 10, 1);

/// Pixels du contour binaire, pour comparaison.
fn contour_binaire(l: i32, h: i32, r: i32, e: i32) -> alloc::collections::BTreeSet<(i32, i32)> {
    let mut pixels = alloc::collections::BTreeSet::new();
    spans_stroke_rounded_rect(Rect::new(0, 0, l as u32, h as u32), r as u32, e as u32,
        GRAND, |x, y, largeur| {
            for dx in 0..largeur as i32 { pixels.insert((x + dx, y)); }
        });
    pixels
}

#[test]
fn le_milieu_du_filet_est_plein_et_le_trou_est_vide() {
    // Un filet epais, pour qu'il y ait un « milieu » a tester.
    let (l, h, r, e) = (200, 160, 30, 8);
    // Milieu du bord haut : dans le filet, loin des deux rampes.
    assert_eq!(couverture_contour(100, e / 2, l, h, r, e), 255);
    // Milieu du bord gauche.
    assert_eq!(couverture_contour(e / 2, 80, l, h, r, e), 255);
    // Coeur de la forme : dans le trou.
    assert_eq!(couverture_contour(100, 80, l, h, r, e), 0);
    // Franchement dehors.
    assert_eq!(couverture_contour(-5, 80, l, h, r, e), 0);
    assert_eq!(couverture_contour(l + 5, 80, l, h, r, e), 0);
}

#[test]
fn chaque_pixel_du_filet_binaire_recoit_de_la_matiere() {
    // L'INVARIANT D'ALIGNEMENT, et le bug qu'il attrape.
    //
    // Le test precedent borne la rampe PAR le filet binaire ; celui-ci exige
    // la reciproque. Ensemble, ils enferment la rampe sur le filet.
    //
    // Sans lui, une rampe decalee d'un pixel resterait dans la dilatation du
    // filet binaire -- donc passerait le premier test -- tout en laissant des
    // pixels binaires a zero. Le filet paraitrait DEPLACE, pas adouci, et le
    // fond anti-crenele qu'il borde ne coinciderait plus avec lui. C'est
    // exactement l'erreur commise une premiere fois sur le remplissage :
    // placer le centre d'arc en coordonnees continues alors que
    // `inside_rounded` le place sur un INDICE de pixel.
    for (l, h, r, e) in [CADRE, (200, 160, 30, 8), (64, 64, 20, 3)] {
        for (x, y) in contour_binaire(l, h, r, e) {
            assert!(
                couverture_contour(x, y, l, h, r, e) > 0,
                "{l}x{h} r={r} e={e} : ({x},{y}) est dans le filet binaire et \
                 la rampe ne l'allume pas du tout"
            );
        }
    }
}

#[test]
fn le_fond_et_le_filet_decrivent_le_meme_cercle() {
    // Les deux sont dessines l'un sur l'autre a chaque trame. Leur bord
    // EXTERIEUR est le meme arc : la ou le filet allume un pixel, le fond doit
    // l'allumer au moins autant, sinon la bordure baverait d'un pixel tout
    // autour de la fenetre.
    for (l, h, r, e) in [CADRE, (200, 160, 30, 8), (1280, 698, 10, 1)] {
        for y in -1..h + 1 {
            for x in -1..l + 1 {
                let filet = couverture_contour(x, y, l, h, r, e);
                let fond = couverture_pixel(x, y, l, h, r);
                assert!(
                    filet <= fond,
                    "{l}x{h} r={r} e={e} : en ({x},{y}) le filet ({filet}) \
                     depasse le fond ({fond})"
                );
            }
        }
    }
}

#[test]
fn la_rampe_du_contour_ne_deborde_pas_du_contour_binaire() {
    let (l, h, r, e) = CADRE;
    let binaire = contour_binaire(l, h, r, e);
    let voisin_du_binaire = |x: i32, y: i32| {
        (-1..=1).any(|dy| (-1..=1).any(|dx| binaire.contains(&(x + dx, y + dy))))
    };
    for y in -2..h + 2 {
        for x in -2..l + 2 {
            if couverture_contour(x, y, l, h, r, e) == 0 { continue }
            assert!(
                voisin_du_binaire(x, y),
                "({x},{y}) est allume a plus d'un pixel du filet binaire"
            );
        }
    }
}

#[test]
fn les_quatre_coins_du_contour_sont_symetriques() {
    let (l, h, r, e) = CADRE;
    for y in 0..r + 2 {
        for x in 0..r + 2 {
            let a = couverture_contour(x, y, l, h, r, e);
            let b = couverture_contour(l - 1 - x, y, l, h, r, e);
            let c = couverture_contour(x, h - 1 - y, l, h, r, e);
            let d = couverture_contour(l - 1 - x, h - 1 - y, l, h, r, e);
            assert_eq!(a, b, "coins haut gauche/droit desaccordes en ({x},{y})");
            assert_eq!(a, c, "coins haut/bas desaccordes en ({x},{y})");
            assert_eq!(a, d, "coins diagonaux desaccordes en ({x},{y})");
        }
    }
}

#[test]
fn le_contour_anti_crenele_conserve_l_aire() {
    // Une rampe qui ajouterait ou retirerait de la matiere epaissirait ou
    // amincirait visiblement le filet.
    for (l, h, r, e) in [CADRE, (200, 160, 30, 8), (64, 64, 20, 3), (1280, 698, 10, 1)] {
        let binaire = contour_binaire(l, h, r, e).len() as i64;
        let mut somme = 0i64;
        for y in -1..h + 1 {
            for x in -1..l + 1 {
                somme += couverture_contour(x, y, l, h, r, e) as i64;
            }
        }
        let aire_aa = (somme + 127) / 255;
        let ecart = (aire_aa - binaire).abs();
        assert!(
            ecart * 100 <= binaire * 5,
            "{l}x{h} r={r} e={e} : aire {aire_aa} contre {binaire} binaire, \
             ecart {ecart} au-dela de 5 %"
        );
    }
}

#[test]
fn le_rasteriseur_de_contour_n_emet_aucun_segment_vide() {
    let (l, h, r, e) = CADRE;
    let mut segments = 0usize;
    let touches = spans_stroke_rounded_rect_aa(Rect::new(0, 0, l as u32, h as u32),
        r as u32, e as u32, GRAND, |_, _, largeur, couverture| {
            segments += 1;
            assert!(largeur > 0, "un segment de largeur nulle");
            assert!(couverture > 0, "un segment de couverture nulle coute sans rien peindre");
        });
    assert!(touches > 0);
    assert!(segments > 0);
}

#[test]
fn le_contour_anti_crenele_reste_dans_la_decoupe() {
    let (l, h, r, e) = CADRE;
    let decoupe = Rect::new(30, 40, 25, 18);
    spans_stroke_rounded_rect_aa(Rect::new(0, 0, l as u32, h as u32), r as u32, e as u32,
        decoupe, |x, y, largeur, _| {
            assert!(x >= decoupe.x && x + largeur as i32 <= decoupe.right(),
                "segment hors decoupe en x");
            assert!(y >= decoupe.y && y < decoupe.bottom(), "segment hors decoupe en y");
        });
}

#[test]
fn le_contour_garde_deux_segments_par_ligne_hors_des_coins() {
    // La propriete de cout. Une bande centrale ne doit produire que le bord
    // gauche et le bord droit : anti-creneler ne doit pas transformer un
    // contour en balayage d'aire.
    let (l, h, r, e) = (1280, 698, 10, 1);
    let mut segments = 0usize;
    spans_stroke_rounded_rect_aa(Rect::new(0, 0, l as u32, h as u32), r as u32, e as u32,
        // Une bande strictement au milieu, hors des deux bandes de coins.
        Rect::new(0, 300, l as u32, 100), |_, _, _, _| segments += 1);
    assert_eq!(segments, 200, "deux segments par ligne sur cent lignes");
}

#[test]
fn un_contour_degenere_ne_panique_pas() {
    assert_eq!(spans_stroke_rounded_rect_aa(Rect::new(0, 0, 0, 0), 4, 1, GRAND,
        |_, _, _, _| {}), 0);
    // Une epaisseur plus grande que la moitie de la forme : le filet mange
    // tout, et il n'y a plus de trou.
    let mut vu = 0usize;
    spans_stroke_rounded_rect_aa(Rect::new(0, 0, 10, 10), 3, 9, GRAND,
        |_, _, largeur, _| vu += largeur as usize);
    assert!(vu > 0, "une forme entierement pleine doit tout de meme se peindre");
}
