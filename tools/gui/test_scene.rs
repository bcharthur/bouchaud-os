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

/// Bornes de dessin d'un cadre plus son ombre portee de 4 pixels.
fn avec_ombre(cadre: Rect) -> Rect {
    Rect::neuf(cadre.x, cadre.y, cadre.largeur + 4, cadre.hauteur + 4)
}

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
    calques.push(Calque::plein(
        Element::BarreTaches,
        Rect::neuf(0, (HAUTEUR - BARRE_H) as i32, LARGEUR, BARRE_H),
    ));
    calques.push(Calque::transparent(
        Element::Curseur,
        Rect::neuf(640, 400, 14, 22),
    ));
    calques
}

fn alloc_vec(fenetre: Option<Rect>) -> Vec<Calque> {
    let mut calques = Vec::new();
    calques.push(Calque::plein(Element::Fond, ecran()));
    calques.push(Calque::transparent(
        Element::Filigrane,
        Rect::neuf(540, (HAUTEUR - 64) as i32, 200, 24),
    ));
    calques.push(Calque::transparent(Element::Icone(0), Rect::neuf(20, 40, 56, 60)));
    calques.push(Calque::transparent(Element::Icone(1), Rect::neuf(20, 120, 56, 60)));
    calques.push(Calque::plein(
        Element::BarreHaute,
        Rect::neuf(0, 0, LARGEUR, BARRE_H),
    ));
    if let Some(cadre) = fenetre {
        calques.push(Calque::avec_ombre(
            Element::Fenetre(0),
            avec_ombre(cadre),
            cadre,
        ));
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
        Element::Fenetre(0),
        "on repart de la fenetre elle-meme, pas d'un calque qui ne dessine rien"
    );
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

// ------------------------------------------- regression : trainees de curseur

/// LE TEST QUI AURAIT ATTRAPE LE BUG.
///
/// Scene minimale `[Fond, Fenetre]`, degat = petit rectangle strictement dans
/// le cadre opaque de la fenetre. Le calque retenu doit etre la FENETRE --
/// celle qui peint reellement ces pixels -- et elle doit figurer parmi les
/// calques dessines.
///
/// La version precedente rendait ici l'index d'un calque `ZonePleine` place
/// APRES la fenetre et qui ne dessinait rien. La boucle
/// `for calque in &calques[debut..]` excluait donc la fenetre, les pixels
/// restaient tels quels, et l'ancien curseur demeurait a l'ecran.
#[test]
fn un_degat_dans_une_fenetre_opaque_redessine_la_fenetre() {
    let cadre = Rect::neuf(100, 100, 800, 500);
    let calques = alloc::vec![
        Calque::plein(Element::Fond, ecran()),
        Calque::avec_ombre(Element::Fenetre(0), avec_ombre(cadre), cadre),
    ];
    let zone = Rect::neuf(300, 250, 14, 22);

    let debut = premier_calque(&calques, &zone);
    assert_eq!(
        calques[debut].element,
        Element::Fenetre(0),
        "le calque retenu doit etre celui qui PEINT ces pixels"
    );

    let dessines: Vec<Element> = calques[debut..]
        .iter()
        .filter(|c| doit_dessiner(c, &zone))
        .map(|c| c.element)
        .collect();
    assert!(
        dessines.contains(&Element::Fenetre(0)),
        "sans la fenetre dans les calques dessines, rien ne recouvre l'ancien \
         curseur : c'est exactement la trainee observee, obtenu {dessines:?}"
    );
}

/// Le cas observe a l'ecran : l'empreinte de l'ANCIEN curseur, dans la fenetre
/// Ladybird, doit etre repeinte par le contenu de cette fenetre.
#[test]
fn l_empreinte_de_l_ancien_curseur_dans_une_fenetre_est_repeinte() {
    // Une fenetre de navigateur qui occupe presque tout l'ecran.
    let cadre = Rect::neuf(40, BARRE_H as i32 + 8, LARGEUR - 80, HAUTEUR - BARRE_H * 3);
    let calques = bureau(Some(cadre));

    // Le curseur s'est deplace : on invalide son ancienne empreinte.
    let ancienne = Rect::neuf(600, 380, 14, 22);
    assert!(
        recouvre(&cadre, &ancienne),
        "prealable du test : l'ancienne empreinte est bien dans la fenetre"
    );

    let debut = premier_calque(&calques, &ancienne);
    let dessines: Vec<Element> = calques[debut..]
        .iter()
        .filter(|c| doit_dessiner(c, &ancienne))
        .map(|c| c.element)
        .collect();

    assert!(
        dessines.contains(&Element::Fenetre(0)),
        "l'ancienne empreinte doit etre recouverte par la fenetre, obtenu {dessines:?}"
    );
    assert!(
        !dessines.contains(&Element::Fond),
        "mais pas par le fond : il est integralement cache par la fenetre"
    );
}

/// Meme propriete pour le menu, qui porte lui aussi une ombre debordante.
#[test]
fn un_degat_dans_le_menu_redessine_le_menu() {
    let cadre_menu = Rect::neuf(2, 400, 220, 300);
    let mut calques = bureau(None);
    // Le menu s'insere juste avant la barre des taches et le curseur.
    let position = calques.len() - 2;
    calques.insert(
        position,
        Calque::avec_ombre(Element::Menu, avec_ombre(cadre_menu), cadre_menu),
    );

    let zone = Rect::neuf(60, 500, 14, 22);
    let debut = premier_calque(&calques, &zone);
    assert_eq!(calques[debut].element, Element::Menu);

    let dessines: Vec<Element> = calques[debut..]
        .iter()
        .filter(|c| doit_dessiner(c, &zone))
        .map(|c| c.element)
        .collect();
    assert!(dessines.contains(&Element::Menu));
}

/// La contrepartie : l'ombre du menu n'est PAS opaque, donc le fond doit y
/// rester dessine. C'est la moitie du contrat que la separation en deux
/// rectangles doit preserver.
#[test]
fn l_ombre_du_menu_laisse_voir_le_fond() {
    let cadre_menu = Rect::neuf(2, 400, 220, 300);
    let calques = alloc::vec![
        Calque::plein(Element::Fond, ecran()),
        Calque::avec_ombre(Element::Menu, avec_ombre(cadre_menu), cadre_menu),
    ];
    // Dans l'ombre (4 px hors du cadre), pas dans le cadre.
    let zone = Rect::neuf(223, 701, 2, 2);

    let debut = premier_calque(&calques, &zone);
    assert_eq!(
        calques[debut].element,
        Element::Fond,
        "l'ombre ne cache rien : on repart du fond"
    );
}

/// Aucun calque ne peut declarer une zone opaque plus large que ce qu'il
/// dessine. `bornes_dessin` MAJORE, `opaque_sur` MINORE : les deux exigences
/// sont opposees, et les confondre est precisement l'erreur d'origine.
#[test]
fn la_zone_opaque_ne_deborde_jamais_les_bornes_de_dessin() {
    let cadre = Rect::neuf(100, 100, 200, 200);
    let mut calques = bureau(Some(cadre));
    calques.push(Calque::avec_ombre(
        Element::Menu,
        avec_ombre(cadre),
        cadre,
    ));

    for calque in &calques {
        if let Some(opaque) = calque.opaque_sur {
            assert!(
                recouvre(&calque.bornes_dessin, &opaque),
                "{:?} declare une zone opaque hors de ses bornes de dessin",
                calque.element,
            );
        }
    }
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
    assert_eq!(
        retenus, 1,
        "dans une fenetre opaque, seule la fenetre reste"
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
        position(Element::Fenetre(0)) < position(Element::BarreTaches),
        "la barre des taches passe au-dessus des fenetres"
    );
}
