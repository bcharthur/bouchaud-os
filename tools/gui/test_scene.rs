//! Preuve hote du culling de scene (Gate 1C).
//!
//! # Ce qui est teste
//!
//! `src/gui/scene.rs`, inclus tel quel : quels calques dessiner pour un
//! rectangle donne, et dans quel ordre. C'est de la geometrie pure -- aucun
//! framebuffer, aucune police, aucune fenetre.
//!
//! # Pourquoi ces regles meritent des tests
//!
//! Une regle de culling fausse ne se voit pas dans un compteur : elle se voit a
//! l'ecran, sous forme de trainee ou de disparition, et parfois seulement dans
//! une configuration de fenetres particuliere. Les deux erreurs classiques sont
//! ici sous test :
//!
//!   * declarer opaque un calque qui ne l'est pas -- l'ombre portee d'une
//!     fenetre --, ce qui fait disparaitre le fond ;
//!   * ecarter un calque qui deborde ses bornes, ce qui laisse une trainee.
//!
//! Lance par `tools/gui/test-scene.sh`.

extern crate alloc;

#[path = "../../src/gui/protocole.rs"]
mod protocole;

#[path = "../../src/gui/scene.rs"]
mod scene;

use protocole::Rect;
use scene::{
    calques_retenus, doit_dessiner, premier_calque, recouvre, se_touchent, Calque, Element,
};

const LARGEUR: u32 = 1280;
const HAUTEUR: u32 = 800;
const BARRE_H: u32 = 28;

fn ecran() -> Rect {
    Rect::neuf(0, 0, LARGEUR, HAUTEUR)
}

/// Un bureau realiste : fond, filigrane, deux icones, barre haute, une fenetre
/// (dessin + occultation), barre des taches, curseur.
fn bureau(fenetre: Option<Rect>) -> Vec<Calque> {
    let mut calques = alloc_vec(fenetre);
    calques.push(Calque::neuf(
        Element::BarreTaches,
        Rect::neuf(0, (HAUTEUR - BARRE_H) as i32, LARGEUR, BARRE_H),
        true,
    ));
    calques.push(Calque::neuf(
        Element::Curseur,
        Rect::neuf(640, 400, 14, 22),
        false,
    ));
    calques
}

fn alloc_vec(fenetre: Option<Rect>) -> Vec<Calque> {
    let mut calques = Vec::new();
    calques.push(Calque::neuf(Element::Fond, ecran(), true));
    calques.push(Calque::neuf(
        Element::Filigrane,
        Rect::neuf(540, (HAUTEUR - 64) as i32, 200, 24),
        false,
    ));
    calques.push(Calque::neuf(Element::Icone(0), Rect::neuf(20, 40, 56, 60), false));
    calques.push(Calque::neuf(Element::Icone(1), Rect::neuf(20, 120, 56, 60), false));
    calques.push(Calque::neuf(
        Element::BarreHaute,
        Rect::neuf(0, 0, LARGEUR, BARRE_H),
        true,
    ));
    if let Some(cadre) = fenetre {
        let avec_ombre = Rect::neuf(
            cadre.x,
            cadre.y,
            cadre.largeur + 4,
            cadre.hauteur + 4,
        );
        calques.push(Calque::neuf(Element::Fenetre(0), avec_ombre, false));
        calques.push(Calque::neuf(Element::ZonePleine, cadre, true));
    }
    calques
}

// ------------------------------------------------------------- geometrie

#[test]
fn recouvrir_est_plus_fort_que_toucher() {
    let grand = Rect::neuf(0, 0, 100, 100);
    let dedans = Rect::neuf(10, 10, 20, 20);
    let chevauche = Rect::neuf(90, 90, 40, 40);

    assert!(se_touchent(&grand, &dedans));
    assert!(recouvre(&grand, &dedans));

    assert!(se_touchent(&grand, &chevauche));
    assert!(!recouvre(&grand, &chevauche), "chevaucher n'est pas recouvrir");
}

#[test]
fn un_rectangle_vide_ne_touche_rien_et_est_recouvert_par_tout() {
    let vide = Rect::neuf(10, 10, 0, 50);
    let grand = Rect::neuf(0, 0, 100, 100);
    assert!(!se_touchent(&grand, &vide));
    assert!(recouvre(&grand, &vide), "il n'y a rien a couvrir");
    assert!(!recouvre(&vide, &grand));
}

#[test]
fn deux_rectangles_adjacents_ne_se_touchent_pas() {
    let gauche = Rect::neuf(0, 0, 50, 50);
    let droite = Rect::neuf(50, 0, 50, 50);
    assert!(
        !se_touchent(&gauche, &droite),
        "se toucher par le bord n'est pas se recouvrir d'un pixel"
    );
}

// ------------------------------------------------------------ intersection

#[test]
fn un_calque_hors_zone_n_est_pas_dessine() {
    let calques = bureau(None);
    // Un rectangle de curseur en bas a droite ne touche ni la barre haute ni
    // les icones : c'est le cas qui coutait une lecture RTC et trois
    // rasterisations TrueType par rectangle.
    let zone = Rect::neuf(1200, 700, 14, 22);

    let barre_haute = calques
        .iter()
        .find(|c| c.element == Element::BarreHaute)
        .unwrap();
    assert!(
        !doit_dessiner(barre_haute, &zone),
        "la barre du haut ne touche pas un rectangle du bas de l'ecran"
    );

    for index in 0..2 {
        let icone = calques
            .iter()
            .find(|c| c.element == Element::Icone(index))
            .unwrap();
        assert!(!doit_dessiner(icone, &zone));
    }
}

#[test]
fn un_calque_qui_touche_la_zone_est_dessine() {
    let calques = bureau(None);
    let zone = Rect::neuf(30, 50, 10, 10); // dans la premiere icone
    let icone = calques
        .iter()
        .find(|c| c.element == Element::Icone(0))
        .unwrap();
    assert!(doit_dessiner(icone, &zone));
}

// --------------------------------------------------------------- occlusion

#[test]
fn une_fenetre_opaque_ecarte_tout_ce_qui_est_dessous() {
    let cadre = Rect::neuf(100, 100, 900, 600);
    let calques = bureau(Some(cadre));
    // Une zone entierement DANS la fenetre.
    let zone = Rect::neuf(200, 200, 64, 64);

    let debut = premier_calque(&calques, &zone);
    assert_eq!(
        calques[debut].element,
        Element::ZonePleine,
        "on repart de la zone pleine de la fenetre"
    );
    // Le fond d'ecran, le filigrane, les icones et la barre haute sont derriere.
    for calque in &calques[..debut] {
        assert!(
            calque.element != Element::ZonePleine,
            "aucune zone pleine ne doit rester derriere"
        );
    }
    assert!(
        calques[..debut].iter().any(|c| c.element == Element::Fond),
        "le fond d'ecran est bien parmi les calques ecartes"
    );
}

#[test]
fn l_ombre_portee_ne_doit_pas_faire_disparaitre_le_fond() {
    let cadre = Rect::neuf(100, 100, 200, 200);
    let calques = bureau(Some(cadre));
    // Zone situee dans l'ombre (4 px hors du cadre) mais PAS dans le cadre.
    let zone = Rect::neuf(301, 301, 2, 2);

    let debut = premier_calque(&calques, &zone);
    let elements: Vec<Element> = calques[debut..]
        .iter()
        .filter(|c| doit_dessiner(c, &zone))
        .map(|c| c.element)
        .collect();

    assert!(
        elements.contains(&Element::Fond),
        "l'ombre laisse voir le fond : il doit etre redessine, obtenu {elements:?}"
    );
    assert!(
        elements.contains(&Element::Fenetre(0)),
        "et l'ombre elle-meme doit etre dessinee"
    );
}

#[test]
fn la_barre_des_taches_occulte_ce_qui_est_dessous_d_elle() {
    let calques = bureau(Some(Rect::neuf(0, 0, LARGEUR, HAUTEUR)));
    let zone = Rect::neuf(400, (HAUTEUR - 10) as i32, 50, 5);

    let debut = premier_calque(&calques, &zone);
    assert_eq!(calques[debut].element, Element::BarreTaches);
}

#[test]
fn le_curseur_n_occulte_jamais_rien() {
    let calques = bureau(None);
    // Zone strictement dans l'empreinte du curseur.
    let zone = Rect::neuf(642, 402, 4, 4);
    let debut = premier_calque(&calques, &zone);
    assert_eq!(
        calques[debut].element,
        Element::Fond,
        "le curseur a des bords transparents : il ne cache rien"
    );
}

#[test]
fn sans_recouvrement_on_repart_du_fond() {
    let calques = bureau(Some(Rect::neuf(100, 100, 200, 200)));
    // Zone a cheval sur le bord de la fenetre : aucun calque opaque ne la
    // recouvre entierement.
    let zone = Rect::neuf(280, 280, 60, 60);
    assert_eq!(premier_calque(&calques, &zone), 0);
}

// ------------------------------------------------------------------ mesure

#[test]
fn le_culling_reduit_reellement_le_nombre_de_calques() {
    let cadre = Rect::neuf(0, BARRE_H as i32, LARGEUR, HAUTEUR - BARRE_H * 2);
    let calques = bureau(Some(cadre));
    let total = calques.len();

    // Le cas Ladybird : une grande fenetre, un petit degat a l'interieur.
    let zone = Rect::neuf(500, 300, 32, 32);
    let retenus = calques_retenus(&calques, &zone);

    assert!(
        retenus < total,
        "le culling doit retenir moins que tout : {retenus} sur {total}"
    );
    assert!(
        retenus <= 2,
        "dans une fenetre opaque, seuls la fenetre et son occultation restent, \
         obtenu {retenus}"
    );
}

#[test]
fn un_degat_plein_ecran_ne_peut_rien_ecarter_sous_le_fond() {
    let calques = bureau(None);
    let zone = ecran();
    assert_eq!(
        premier_calque(&calques, &zone),
        0,
        "un degat plein ecran repart du fond"
    );
    // ... mais l'intersection ecarte quand meme ce qui est hors zone : ici
    // rien, puisque la zone est l'ecran entier.
    assert_eq!(calques_retenus(&calques, &zone), calques.len());
}

#[test]
fn l_ordre_de_dessin_est_preserve() {
    let calques = bureau(Some(Rect::neuf(100, 100, 200, 200)));
    let zone = ecran();
    let ordre: Vec<Element> = calques
        .iter()
        .filter(|c| doit_dessiner(c, &zone))
        .map(|c| c.element)
        .collect();

    let position = |cherche: Element| ordre.iter().position(|e| *e == cherche).unwrap();
    assert!(position(Element::Fond) < position(Element::BarreHaute));
    assert!(position(Element::BarreHaute) < position(Element::Fenetre(0)));
    assert!(position(Element::Fenetre(0)) < position(Element::BarreTaches));
    assert!(
        position(Element::BarreTaches) < position(Element::Curseur),
        "le curseur est toujours au-dessus de tout"
    );
    assert!(
        position(Element::Fenetre(0)) < position(Element::ZonePleine),
        "l'occultation vient juste apres sa fenetre, sinon elle ne l'occulterait pas"
    );
}
