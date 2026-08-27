//! Geometrie du bureau : deux barres, un menu, des lignes.
//!
//! # Le defaut que ces tests attrapent
//!
//! Le bureau a deux barres de 11 pixels, une en haut, une en bas. Elles se
//! ressemblent dans le code -- meme hauteur, meme largeur, meme forme
//! d'appel -- et ne se ressemblent pas du tout a l'ecran.
//!
//! L'horloge, la charge CPU, la memoire et le disque sont peints par
//! `Element::BarreHaute`, donc EN HAUT. Le tic d'horloge invalidait
//! `barre_taches_rect()`, donc EN BAS. Le compositeur presentait
//! consciencieusement, chaque seconde, une bande ou rien n'avait change.
//!
//! Aucun compteur ne pouvait le dire : `frames_clock_only`, `presents` et
//! `presented_pixels` montaient tous. Seule la geometrie le dit.
//!
//! Lance par `tools/gui/test-disposition.sh`.

extern crate alloc;

#[path = "../../src/gui/protocole.rs"]
mod protocole;

#[path = "../../src/gui/disposition.rs"]
mod disposition;

use disposition::*;
use protocole::Rect;

// Les dimensions reelles du framebuffer (`drivers::display::bochs`).
const L: u32 = 1280;
const H: u32 = 720;

/// Le menu tel que `window::menu_rect` le calcule, pour 7 entrees.
fn menu() -> Rect {
    let lignes = 7i32;
    let h = lignes * HAUTEUR_LIGNE_MENU + ENTETE_MENU + 8;
    Rect::neuf(2, H as i32 - HAUTEUR_BARRE as i32 - h, 178, h as u32)
}

fn se_touchent(a: Rect, b: Rect) -> bool {
    !a.intersecte(&b).vide()
}

// ─── Les deux barres ───────────────────────────────────────────────────────

#[test]
fn la_barre_haute_est_en_haut() {
    let haute = barre_haute(L);
    assert_eq!(haute.y, 0, "la barre du haut commence a la ligne 0");
    assert_eq!(haute.largeur, L);
    assert_eq!(haute.hauteur, HAUTEUR_BARRE);
}

#[test]
fn la_barre_des_taches_est_en_bas() {
    let basse = barre_taches(L, H);
    assert_eq!(basse.bas(), H as i64, "la barre du bas finit au dernier pixel");
    assert_eq!(basse.hauteur, HAUTEUR_BARRE);
}

/// LE test qui aurait echoue.
///
/// Le tic d'horloge invalidait la barre des taches. Si les deux barres se
/// touchaient, l'erreur se serait rattrapee toute seule. Elles ne se touchent
/// pas : le degat annonce et les pixels qui changent etaient DISJOINTS.
#[test]
fn les_deux_barres_sont_disjointes() {
    let haute = barre_haute(L);
    let basse = barre_taches(L, H);
    assert!(
        !se_touchent(haute, basse),
        "invalider l'une ne peut donc RIEN repeindre de l'autre : \
         haute={:?} basse={:?}",
        (haute.x, haute.y, haute.largeur, haute.hauteur),
        (basse.x, basse.y, basse.largeur, basse.hauteur),
    );
}

/// L'heure est peinte a `y = 1` (voir `widgets::draw_topbar`). Le rectangle
/// invalide chaque seconde doit la contenir.
#[test]
fn le_rectangle_de_l_horloge_contient_les_pixels_de_l_heure() {
    let haute = barre_haute(L);
    // `draw_text_prop(fb::WIDTH - cw - 4, 1, ...)` : coin haut droit.
    let heure = Rect::neuf(L as i32 - 60, 1, 56, 9);
    assert_eq!(
        heure.intersecte(&haute),
        heure,
        "la barre du haut couvre entierement l'horloge"
    );
    assert!(
        !se_touchent(heure, barre_taches(L, H)),
        "la barre du BAS n'en couvre pas un seul pixel"
    );
}

/// Idem pour les statistiques CPU/RAM/Disque, centrees a `y = 1`.
#[test]
fn le_rectangle_de_l_horloge_contient_aussi_les_statistiques() {
    let haute = barre_haute(L);
    let stats = Rect::neuf(L as i32 / 2 - 200, 1, 400, 9);
    assert_eq!(stats.intersecte(&haute), stats);
    assert!(!se_touchent(stats, barre_taches(L, H)));
}

#[test]
fn une_barre_reste_dans_l_ecran() {
    for largeur in [1u32, 320, 1280] {
        for hauteur in [11u32, 200, 720] {
            let ecran = Rect::neuf(0, 0, largeur, hauteur);
            let haute = barre_haute(largeur);
            let basse = barre_taches(largeur, hauteur);
            assert_eq!(haute.intersecte(&ecran), haute, "{largeur}x{hauteur}");
            assert_eq!(basse.intersecte(&ecran), basse, "{largeur}x{hauteur}");
        }
    }
}

// ─── Ombre portee ──────────────────────────────────────────────────────────

/// L'ombre DECALEE du menu Demarrer : `draw_menu` peint une copie du cadre
/// translatee vers le bas et la droite.
#[test]
fn l_empreinte_du_menu_deborde_en_bas_et_a_droite() {
    let cadre = Rect::neuf(100, 100, 200, 150);
    let empreinte = empreinte_avec_ombre(cadre);
    assert_eq!(empreinte.x, cadre.x, "l'ombre du menu ne deborde pas a gauche");
    assert_eq!(empreinte.y, cadre.y, "ni en haut");
    assert_eq!(empreinte.droite(), cadre.droite() + DEBORD_OMBRE as i64);
    assert_eq!(empreinte.bas(), cadre.bas() + DEBORD_OMBRE as i64);
}

/// L'ombre d'une FENETRE est un anneau : `paint_window_shape` peint
/// `SHADOW_EXTENT` contours AUTOUR du cadre. Elle deborde donc des QUATRE
/// cotes, et un degat qui ne l'ajouterait qu'en bas et a droite laisserait
/// deux bandes sombres derriere chaque fenetre deplacee.
#[test]
fn l_empreinte_d_une_fenetre_deborde_des_quatre_cotes() {
    let cadre = Rect::neuf(100, 100, 200, 150);
    let empreinte = empreinte_fenetre_peinte(cadre);
    assert_eq!(empreinte.x, cadre.x - DEBORD_OMBRE as i32, "a gauche");
    assert_eq!(empreinte.y, cadre.y - DEBORD_OMBRE as i32, "en haut");
    assert_eq!(empreinte.droite(), cadre.droite() + DEBORD_OMBRE as i64, "a droite");
    assert_eq!(empreinte.bas(), cadre.bas() + DEBORD_OMBRE as i64, "en bas");
}

/// L'empreinte d'une fenetre CONTIENT toujours son cadre, meme colle a un bord
/// de l'ecran ou l'ombre sort du framebuffer : c'est au compositeur d'ecreter,
/// pas a la geometrie de mentir.
#[test]
fn l_empreinte_d_une_fenetre_contient_toujours_son_cadre() {
    for cadre in [
        Rect::neuf(0, 0, 40, 30),
        Rect::neuf(2, 2, 1, 1),
        Rect::neuf(L as i32 - 10, H as i32 - 10, 40, 30),
        Rect::neuf(-5, -5, 60, 60),
    ] {
        let empreinte = empreinte_fenetre_peinte(cadre);
        assert_eq!(
            empreinte.intersecte(&cadre),
            cadre,
            "l'empreinte doit contenir le cadre pour {cadre:?}"
        );
    }
}

/// Un cadre vide n'a pas d'ombre non plus dans la version fenetre : sinon une
/// fenetre de taille nulle produirait un degat de 16 pixels de cote.
#[test]
fn un_cadre_vide_n_a_pas_d_ombre_de_fenetre() {
    assert!(empreinte_fenetre_peinte(Rect::default()).vide());
    assert!(empreinte_fenetre_peinte(Rect::neuf(10, 10, 0, 50)).vide());
}

#[test]
fn un_cadre_vide_n_a_pas_d_ombre() {
    assert!(empreinte_avec_ombre(Rect::default()).vide());
    assert!(empreinte_avec_ombre(Rect::neuf(10, 10, 0, 50)).vide());
}

// ─── Survol du menu ────────────────────────────────────────────────────────

#[test]
fn le_menu_a_autant_de_lignes_qu_il_est_haut() {
    assert_eq!(lignes_menu(menu()), 7);
}

#[test]
fn chaque_ligne_est_survolee_sur_toute_sa_hauteur() {
    let m = menu();
    for index in 0..lignes_menu(m) {
        let ligne = rect_ligne_menu(m, index);
        for dy in 0..ligne.hauteur as i32 {
            let y = ligne.y + dy;
            assert_eq!(
                ligne_menu_survolee(m, m.x + BANDE_ACCENT + 1, y),
                Some(index),
                "ligne {index}, y={y}"
            );
        }
    }
}

/// La reciproque, et c'est elle qui compte pour l'invalidation : si le survol
/// rendait une ligne dont le rectangle ne contient pas le pointeur, le degat
/// annonce ne couvrirait pas les pixels repeints.
#[test]
fn le_rectangle_d_une_ligne_contient_tout_ce_qui_la_survole() {
    let m = menu();
    for y in (m.y - 30)..(m.bas() as i32 + 30) {
        for x in (m.x - 10)..(m.droite() as i32 + 10) {
            if let Some(index) = ligne_menu_survolee(m, x, y) {
                let ligne = rect_ligne_menu(m, index);
                assert!(
                    x >= ligne.x
                        && (x as i64) < ligne.droite()
                        && y >= ligne.y
                        && (y as i64) < ligne.bas(),
                    "({x},{y}) survole la ligne {index} mais sort de son rectangle"
                );
            }
        }
    }
}

#[test]
fn les_lignes_ne_se_recouvrent_pas() {
    let m = menu();
    let n = lignes_menu(m);
    for a in 0..n {
        for b in (a + 1)..n {
            assert!(
                !se_touchent(rect_ligne_menu(m, a), rect_ligne_menu(m, b)),
                "lignes {a} et {b}"
            );
        }
    }
}

#[test]
fn une_ligne_reste_dans_le_menu() {
    let m = menu();
    for index in 0..lignes_menu(m) {
        let ligne = rect_ligne_menu(m, index);
        assert_eq!(ligne.intersecte(&m), ligne, "ligne {index} deborde du menu");
    }
}

#[test]
fn la_bande_d_accent_ne_survole_rien() {
    let m = menu();
    let y = m.y + ENTETE_MENU + 5;
    for x in m.x..(m.x + BANDE_ACCENT) {
        assert_eq!(
            ligne_menu_survolee(m, x, y),
            None,
            "x={x} est dans la bande d'accent, pas sur une ligne"
        );
    }
    assert_eq!(ligne_menu_survolee(m, m.x + BANDE_ACCENT, y), Some(0));
}

#[test]
fn hors_du_menu_rien_n_est_survole() {
    let m = menu();
    let dedans_y = m.y + ENTETE_MENU + 5;
    assert_eq!(ligne_menu_survolee(m, m.x - 1, dedans_y), None);
    assert_eq!(ligne_menu_survolee(m, m.droite() as i32, dedans_y), None);
    assert_eq!(ligne_menu_survolee(m, m.x + 50, m.y - 1), None);
    assert_eq!(ligne_menu_survolee(m, m.x + 50, m.bas() as i32), None);
    // L'entete du menu ne porte aucune ligne.
    assert_eq!(ligne_menu_survolee(m, m.x + 50, m.y + ENTETE_MENU - 1), None);
}

/// Les 8 pixels du bas du menu sont une marge, pas une huitieme ligne.
#[test]
fn la_marge_basse_ne_survole_rien() {
    let m = menu();
    let derniere = rect_ligne_menu(m, lignes_menu(m) - 1);
    for y in (derniere.bas() as i32)..(m.bas() as i32) {
        assert_eq!(ligne_menu_survolee(m, m.x + 50, y), None, "y={y}");
    }
}

#[test]
fn un_index_hors_menu_rend_un_rectangle_vide() {
    let m = menu();
    assert!(rect_ligne_menu(m, lignes_menu(m)).vide());
    assert!(rect_ligne_menu(m, 999).vide());
}

// ─── Curseur ───────────────────────────────────────────────────────────────

#[test]
fn l_empreinte_du_curseur_couvre_la_fleche() {
    // `widgets::draw_cursor` peint une fleche de 12 colonnes sur 19 lignes,
    // plus son contour d'un pixel : 14x22 la majore.
    let c = curseur(400, 300);
    assert!(c.largeur >= 12 + 2 && c.hauteur >= 19 + 2);
    assert_eq!((c.x, c.y), (400, 300), "le point chaud est en haut a gauche");
}
