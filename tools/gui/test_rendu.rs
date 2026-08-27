//! Oracle d'equivalence de rendu : le culling ne doit RIEN changer a l'image.
//!
//! # La propriete
//!
//! Pour chaque rectangle de degat :
//!
//!     TEST      = tampon contenant encore la scene A,
//!                 puis pipeline reel (occlusion + intersection) de la scene B
//!                 applique UNIQUEMENT sur le degat
//!
//!     REFERENCE = meme tampon de depart,
//!                 puis TOUS les calques de la scene B dessines avec le meme
//!                 clip, sans aucun culling
//!
//!     ASSERT    : tout pixel DANS le degat est identique.
//!
//! Le tampon hors degat n'est pas compare : le compositeur ne le presente pas.
//!
//! # Pourquoi cet oracle et pas des tests de culling
//!
//! Les tests de `scene.rs` verifient des DECISIONS -- quel calque part, quel
//! calque reste. Ils ne peuvent pas voir qu'une decision juste sur le papier
//! laisse des pixels perimes a l'ecran, parce qu'ils ne dessinent rien.
//!
//! Les trainees observees au runtime -- rectangles sombres sur le bureau,
//! fragments de chrome, artefacts autour du menu Demarrer -- sont exactement
//! cela : un calque ecarte a tort, donc des pixels de la scene PRECEDENTE
//! laisses intacts sous un degat qui sera pourtant presente.
//!
//! Cet oracle les attrape toutes d'un coup, sans cas particulier « curseur ».
//!
//! # Le rasteriseur
//!
//! Volontairement minimal : des rectangles et des couleurs. On teste la
//! SEMANTIQUE du plan, du culling et du tampon persistant -- pas fontdue, pas
//! les degrades, pas les polices.
//!
//! Ce qu'il respecte scrupuleusement, en revanche, ce sont les deux contrats
//! que `Calque` declare :
//!
//!   * un calque ne peint JAMAIS hors de `bornes_dessin` ;
//!   * un calque peint TOUS les pixels de `opaque_sur`.
//!
//! Un plan qui ment sur l'un ou l'autre produit une divergence, et c'est
//! precisement ce qu'on veut detecter.
//!
//! Lance par `tools/gui/test-rendu.sh`.

extern crate alloc;

#[path = "../../src/gui/protocole.rs"]
mod protocole;

#[path = "../../src/gui/scene.rs"]
mod scene;

use protocole::Rect;
use scene::{doit_dessiner, premier_calque, Calque, Element};

const L: usize = 200;
const H: usize = 120;
const BARRE_H: u32 = 8;

const VIDE: u32 = 0x00_0000;

// ---------------------------------------------------------------- tampon

#[derive(Clone, PartialEq, Eq)]
struct Tampon {
    px: Vec<u32>,
}

impl Tampon {
    fn neuf() -> Self {
        Self { px: vec![VIDE; L * H] }
    }

    fn pose(&mut self, x: i32, y: i32, couleur: u32, clip: &Rect) {
        if x < clip.x || y < clip.y || (x as i64) >= clip.droite() || (y as i64) >= clip.bas() {
            return;
        }
        if x < 0 || y < 0 || x as usize >= L || y as usize >= H {
            return;
        }
        self.px[y as usize * L + x as usize] = couleur;
    }

    fn remplit(&mut self, zone: &Rect, couleur: u32, clip: &Rect) {
        for y in zone.y..(zone.bas() as i32) {
            for x in zone.x..(zone.droite() as i32) {
                self.pose(x, y, couleur, clip);
            }
        }
    }

    /// Motif clairseme : un pixel sur deux. Represente du texte ou une icone --
    /// ce qui NE remplit pas sa boite, donc ne peut pas etre declare opaque.
    fn tramage(&mut self, zone: &Rect, couleur: u32, clip: &Rect) {
        for y in zone.y..(zone.bas() as i32) {
            for x in zone.x..(zone.droite() as i32) {
                if (x + y) % 2 == 0 {
                    self.pose(x, y, couleur, clip);
                }
            }
        }
    }
}

// ----------------------------------------------------------------- scene

/// Une scene de test : les calques, plus de quoi les peindre.
struct Scene {
    calques: Vec<Calque>,
    /// Cadre plein de chaque calque a ombre, dans le meme ordre.
    fenetres: Vec<(Element, Rect)>,
}

fn ecran() -> Rect {
    Rect::neuf(0, 0, L as u32, H as u32)
}

/// Debordement de l'ombre portee, comme `widgets::DEBORD_OMBRE`.
const DEBORD_OMBRE: u32 = 4;

/// Ce qu'une fenetre ou un menu OCCUPE a l'ecran, ombre comprise.
///
/// C'est la meme fonction que `window_manager::empreinte_fenetre`, et c'est
/// tout l'enjeu : les bornes du calque ET le rectangle invalide doivent en
/// venir. Quand ils divergeaient, la bande d'ombre de l'ancienne position
/// n'etait jamais invalidee, et le rectangle sombre restait a l'ecran.
fn avec_ombre(cadre: Rect) -> Rect {
    Rect::neuf(
        cadre.x,
        cadre.y,
        cadre.largeur + DEBORD_OMBRE,
        cadre.hauteur + DEBORD_OMBRE,
    )
}

/// Le degat qu'un deplacement de fenetre doit produire : l'union des deux
/// EMPREINTES, pas des deux cadres.
fn degats_deplacement(avant: Rect, apres: Rect) -> Vec<Rect> {
    vec![avec_ombre(avant), avec_ombre(apres)]
}

/// Couleur deterministe d'un element : elle doit differer d'un element a
/// l'autre, sinon une confusion de calques passerait inapercue.
fn couleur(element: Element) -> u32 {
    match element {
        Element::Fond => 0x10_2030,
        Element::Filigrane => 0x33_4466,
        Element::Icone(i) => 0x40_5000 + (i as u32 + 1) * 0x11,
        Element::BarreHaute => 0x0d_1a30,
        Element::Fenetre(i) => 0x11_1827 + (i as u32 + 1) * 0x10_0000,
        Element::Menu => 0x13_1c2e,
        Element::BarreTaches => 0x18_2440,
        Element::Curseur => 0xff_ffff,
    }
}

const OMBRE: u32 = 0x04_080f;

impl Scene {
    /// Peint UN calque, en respectant ses deux contrats.
    fn peins_calque(&self, tampon: &mut Tampon, calque: &Calque, clip: &Rect) {
        let c = couleur(calque.element);
        match calque.element {
            // Opaques sur toutes leurs bornes : ils remplissent.
            Element::Fond | Element::BarreHaute | Element::BarreTaches => {
                tampon.remplit(&calque.bornes_dessin, c, clip);
            }
            // Ombre debordante PUIS cadre plein. L'ombre ne couvre pas tout :
            // c'est bien pour cela que `opaque_sur` est plus petit.
            Element::Fenetre(_) | Element::Menu => {
                let cadre = self
                    .fenetres
                    .iter()
                    .find(|(e, _)| *e == calque.element)
                    .map(|(_, r)| *r)
                    .expect("cadre plein manquant pour un calque a ombre");
                let ombre = Rect::neuf(cadre.x + 4, cadre.y + 4, cadre.largeur, cadre.hauteur);
                tampon.remplit(&ombre, OMBRE, clip);
                tampon.remplit(&cadre, c, clip);
            }
            // Ne remplissent pas leur boite : jamais opaques.
            Element::Filigrane | Element::Icone(_) | Element::Curseur => {
                tampon.tramage(&calque.bornes_dessin, c, clip);
            }
        }
    }

    /// REFERENCE : tous les calques, aucun culling.
    fn peins_tout(&self, tampon: &mut Tampon, clip: &Rect) {
        for calque in &self.calques {
            self.peins_calque(tampon, calque, clip);
        }
    }

    /// TEST : le pipeline reel -- occlusion puis intersection.
    fn peins_avec_culling(&self, tampon: &mut Tampon, clip: &Rect) {
        let debut = premier_calque(&self.calques, clip);
        for calque in &self.calques[debut..] {
            if !doit_dessiner(calque, clip) {
                continue;
            }
            self.peins_calque(tampon, calque, clip);
        }
    }

    fn rendu_complet(&self) -> Tampon {
        let mut tampon = Tampon::neuf();
        self.peins_tout(&mut tampon, &ecran());
        tampon
    }
}

/// Construit une scene comme `plan_de_scene` le fait.
struct Constructeur {
    calques: Vec<Calque>,
    fenetres: Vec<(Element, Rect)>,
}

impl Constructeur {
    fn neuf() -> Self {
        let mut c = Self { calques: Vec::new(), fenetres: Vec::new() };
        c.calques.push(Calque::plein(Element::Fond, ecran()));
        c.calques.push(Calque::transparent(
            Element::Filigrane,
            Rect::neuf(70, (H - 20) as i32, 40, 10),
        ));
        c.calques
            .push(Calque::transparent(Element::Icone(0), Rect::neuf(4, 12, 16, 18)));
        c.calques
            .push(Calque::transparent(Element::Icone(1), Rect::neuf(4, 36, 16, 18)));
        c.calques
            .push(Calque::plein(Element::BarreHaute, Rect::neuf(0, 0, L as u32, BARRE_H)));
        c
    }

    fn fenetre(mut self, index: usize, cadre: Rect) -> Self {
        let element = Element::Fenetre(index);
        self.calques
            .push(Calque::avec_ombre(element, avec_ombre(cadre), cadre));
        self.fenetres.push((element, cadre));
        self
    }

    fn menu(mut self, cadre: Rect) -> Self {
        self.calques
            .push(Calque::avec_ombre(Element::Menu, avec_ombre(cadre), cadre));
        self.fenetres.push((Element::Menu, cadre));
        self
    }

    fn finit(mut self, curseur: (i32, i32)) -> Scene {
        self.calques.push(Calque::plein(
            Element::BarreTaches,
            Rect::neuf(0, (H as u32 - BARRE_H) as i32, L as u32, BARRE_H),
        ));
        self.calques.push(Calque::transparent(
            Element::Curseur,
            empreinte_curseur(curseur),
        ));
        Scene { calques: self.calques, fenetres: self.fenetres }
    }
}

fn empreinte_curseur((x, y): (i32, i32)) -> Rect {
    Rect::neuf(x, y, 8, 12)
}

// ------------------------------------------------------------- l'oracle

/// Le coeur du test. Rend le nombre de pixels divergents.
/// Chaque rectangle est verifie INDEPENDAMMENT, depuis le meme tampon de
/// depart.
///
/// C'est plus fort que de les appliquer cumulativement, et c'est aussi plus
/// fidele : le compositeur traite un rectangle a la fois, et la propriete doit
/// tenir pour chacun. Cumuler laissait un rectangle correctement traite
/// reparer les pixels qu'un autre avait laisses perimes -- une fausse
/// reussite, constatee en essayant.
fn divergences(scene_a: &Scene, scene_b: &Scene, degats: &[Rect]) -> usize {
    // Etat de depart : le tampon contient encore la scene A. C'est le point
    // cle -- le compositeur ne repart jamais d'un tampon vierge.
    let depart = scene_a.rendu_complet();
    let mut divergents = 0usize;

    for zone in degats {
        // REFERENCE : tous les calques de B, meme clip, aucun culling.
        let mut reference = depart.clone();
        scene_b.peins_tout(&mut reference, zone);

        // TEST : le pipeline reel -- occlusion puis intersection.
        let mut test = depart.clone();
        scene_b.peins_avec_culling(&mut test, zone);

        for y in zone.y.max(0)..(zone.bas().min(H as i64) as i32) {
            for x in zone.x.max(0)..(zone.droite().min(L as i64) as i32) {
                let i = y as usize * L + x as usize;
                if test.px[i] != reference.px[i] {
                    divergents += 1;
                }
            }
        }
    }
    divergents
}

fn verifie(nom: &str, scene_a: &Scene, scene_b: &Scene, degats: &[Rect]) {
    let d = divergences(scene_a, scene_b, degats);
    assert_eq!(
        d, 0,
        "{nom} : {d} pixel(s) perime(s) dans le degat -- le culling a ecarte \
         un calque qui peignait ces pixels"
    );
}

fn bureau(curseur: (i32, i32)) -> Scene {
    Constructeur::neuf().finit(curseur)
}

fn bureau_fenetre(cadre: Rect, curseur: (i32, i32)) -> Scene {
    Constructeur::neuf().fenetre(0, cadre).finit(curseur)
}

// =========================================================== les dix cas

#[test]
fn cas_1_curseur_sur_le_fond() {
    let a = bureau((30, 60));
    let b = bureau((90, 60));
    verifie(
        "curseur A -> B sur le fond",
        &a,
        &b,
        &[empreinte_curseur((30, 60)), empreinte_curseur((90, 60))],
    );
}

#[test]
fn cas_2_curseur_dans_une_fenetre_opaque() {
    let cadre = Rect::neuf(20, 20, 140, 70);
    let a = bureau_fenetre(cadre, (40, 40));
    let b = bureau_fenetre(cadre, (100, 60));
    verifie(
        "curseur A -> B dans une fenetre opaque",
        &a,
        &b,
        &[empreinte_curseur((40, 40)), empreinte_curseur((100, 60))],
    );
}

#[test]
fn cas_3_curseur_quitte_la_fenetre_vers_le_bureau() {
    let cadre = Rect::neuf(20, 20, 80, 50);
    let a = bureau_fenetre(cadre, (50, 40));
    let b = bureau_fenetre(cadre, (140, 90));
    verifie(
        "curseur sortant d'une fenetre",
        &a,
        &b,
        &[empreinte_curseur((50, 40)), empreinte_curseur((140, 90))],
    );
}

#[test]
fn cas_4_curseur_traverse_bordure_et_ombre() {
    let cadre = Rect::neuf(20, 20, 80, 50);
    // B place le curseur a cheval sur le bord droit ET sur l'ombre.
    let a = bureau_fenetre(cadre, (50, 40));
    let b = bureau_fenetre(cadre, (96, 66));
    verifie(
        "curseur a cheval sur bordure et ombre",
        &a,
        &b,
        &[empreinte_curseur((50, 40)), empreinte_curseur((96, 66))],
    );
}

#[test]
fn cas_5_ouverture_du_menu() {
    let menu = Rect::neuf(2, 60, 60, 44);
    let a = bureau((120, 40));
    let b = Constructeur::neuf().menu(menu).finit((120, 40));
    verifie("ouverture du menu", &a, &b, &[avec_ombre(menu)]);
}

#[test]
fn cas_6_fermeture_du_menu() {
    let menu = Rect::neuf(2, 60, 60, 44);
    let a = Constructeur::neuf().menu(menu).finit((120, 40));
    let b = bureau((120, 40));
    verifie("fermeture du menu", &a, &b, &[avec_ombre(menu)]);
}

#[test]
fn cas_7_curseur_dans_le_menu_et_son_ombre() {
    let menu = Rect::neuf(2, 60, 60, 44);
    let a = Constructeur::neuf().menu(menu).finit((20, 70));
    // B : le curseur passe sur l'ombre du menu, qui n'est PAS opaque.
    let b = Constructeur::neuf().menu(menu).finit((60, 100));
    verifie(
        "curseur dans le menu puis sur son ombre",
        &a,
        &b,
        &[empreinte_curseur((20, 70)), empreinte_curseur((60, 100))],
    );
}

#[test]
fn cas_8_deux_fenetres_superposees() {
    let bas = Rect::neuf(10, 15, 100, 60);
    let haut = Rect::neuf(50, 30, 100, 60);
    let a = Constructeur::neuf().fenetre(0, bas).fenetre(1, haut).finit((60, 40));
    let b = Constructeur::neuf().fenetre(0, bas).fenetre(1, haut).finit((120, 80));
    verifie(
        "deux fenetres superposees",
        &a,
        &b,
        &[empreinte_curseur((60, 40)), empreinte_curseur((120, 80))],
    );
}

#[test]
fn cas_9_fenetre_deplacee_ancienne_et_nouvelle_empreinte() {
    let avant = Rect::neuf(20, 20, 70, 40);
    let apres = Rect::neuf(90, 50, 70, 40);
    let a = bureau_fenetre(avant, (150, 100));
    let b = bureau_fenetre(apres, (150, 100));
    verifie(
        "fenetre deplacee",
        &a,
        &b,
        &degats_deplacement(avant, apres),
    );
}

#[test]
fn cas_10_degats_sparse_eloignes() {
    let cadre = Rect::neuf(60, 30, 60, 40);
    let a = bureau_fenetre(cadre, (10, 100));
    let b = bureau_fenetre(cadre, (180, 15));
    verifie(
        "degats sparse eloignes",
        &a,
        &b,
        &[
            empreinte_curseur((10, 100)),
            empreinte_curseur((180, 15)),
            Rect::neuf(0, 0, L as u32, BARRE_H),
        ],
    );
}

/// LES COINS DE L'OMBRE PORTEE.
///
/// L'ombre est dessinee decalee de 4 pixels : l'union du cadre et de l'ombre
/// n'est PAS leur boite englobante. Deux bandes de coin -- en bas a gauche et
/// en haut a droite -- ne sont peintes par AUCUN des deux, et laissent donc
/// voir le fond.
///
/// C'est precisement la ou une opacite declaree trop large fait des degats :
/// `premier_calque` ecarte le fond, la fenetre ne peint pas ces pixels, et
/// l'image precedente y reste. Sans ce cas, l'oracle laissait passer une
/// fenetre declarant `opaque_sur == bornes_dessin` -- verifie.
#[test]
fn cas_11_les_coins_de_l_ombre_portee_montrent_le_fond() {
    let cadre = Rect::neuf(20, 20, 80, 50);
    // Les deux bandes que ni le cadre ni l'ombre ne couvrent.
    let coin_bas_gauche = Rect::neuf(cadre.x, cadre.bas() as i32, 4, 4);
    let coin_haut_droite = Rect::neuf(cadre.droite() as i32, cadre.y, 4, 4);

    // A et B different SOUS la fenetre : le curseur bouge sur le fond, donc les
    // pixels perimes seraient visibles.
    let a = bureau_fenetre(cadre, (18, 68));
    let b = bureau_fenetre(cadre, (150, 100));

    verifie(
        "coins de l'ombre portee",
        &a,
        &b,
        &[coin_bas_gauche, coin_haut_droite, empreinte_curseur((18, 68))],
    );
}

/// La contrepartie mesurable : ces bandes de coin ne doivent PAS etre
/// considerees comme recouvertes par la fenetre. Si elles l'etaient, le fond
/// serait ecarte et les pixels resteraient perimes.
#[test]
fn les_coins_de_l_ombre_ne_sont_pas_recouverts_par_la_fenetre() {
    let cadre = Rect::neuf(20, 20, 80, 50);
    let scene = bureau_fenetre(cadre, (150, 100));
    let coin = Rect::neuf(cadre.x, cadre.bas() as i32, 4, 4);

    let debut = premier_calque(&scene.calques, &coin);
    assert_eq!(
        scene.calques[debut].element,
        Element::Fond,
        "le coin de l'ombre laisse voir le fond : on doit repartir de lui"
    );
}

/// LE DEGAT DOIT COUVRIR CE QUI EST PEINT, PAS SEULEMENT LE CADRE.
///
/// Le culling ne peut rien redessiner en dehors du degat qu'on lui donne :
/// meme parfait, il ne repare pas un degat trop petit. Ce test verifie donc
/// l'autre moitie de la chaine.
///
/// Une fenetre qui se deplace laisse derriere elle sa bande d'ombre. Si le
/// degat ne couvre que l'ancien CADRE, cette bande n'est jamais repeinte et le
/// rectangle sombre reste a l'ecran -- ce qui etait observe au runtime.
#[test]
fn le_degat_d_un_deplacement_couvre_l_ombre_laissee_derriere() {
    let avant = Rect::neuf(20, 20, 70, 40);
    let apres = Rect::neuf(110, 60, 70, 40);
    let a = bureau_fenetre(avant, (150, 15));
    let b = bureau_fenetre(apres, (150, 15));

    // La bande d'ombre que l'ancienne position laisse derriere elle.
    let bande_basse = Rect::neuf(
        avant.x + DEBORD_OMBRE as i32,
        avant.bas() as i32,
        avant.largeur,
        DEBORD_OMBRE,
    );
    let degats = degats_deplacement(avant, apres);
    assert!(
        degats.iter().any(|d| scene::recouvre(d, &bande_basse)),
        "le degat doit inclure la bande d'ombre de l'ancienne position, \
         sinon personne ne la repeint"
    );

    verifie("deplacement, ombre comprise", &a, &b, &degats);
}

/// Meme propriete pour le menu : ouverture et fermeture.
#[test]
fn le_degat_du_menu_couvre_son_ombre() {
    let menu = Rect::neuf(2, 50, 60, 44);
    let ferme = bureau((150, 20));
    let ouvert = Constructeur::neuf().menu(menu).finit((150, 20));

    let bande = Rect::neuf(
        menu.x + DEBORD_OMBRE as i32,
        menu.bas() as i32,
        menu.largeur,
        DEBORD_OMBRE,
    );
    let degat = avec_ombre(menu);
    assert!(
        scene::recouvre(&degat, &bande),
        "le degat du menu doit inclure son ombre"
    );

    verifie("ouverture du menu, ombre comprise", &ferme, &ouvert, &[degat]);
    verifie("fermeture du menu, ombre comprise", &ouvert, &ferme, &[degat]);
}

/// LES DEUX CONTRATS DE `Calque`, verifies pixel par pixel.
///
/// L'oracle d'equivalence attrape un calque ECARTE a tort. Il n'attrape pas un
/// calque qui MENT sur sa geometrie, parce qu'un mensonge coherent -- bornes et
/// dessin retrecis ensemble -- reste coherent. Verifie en essayant.
///
/// Ces deux invariants ferment cette porte :
///
///   * un calque ne peint jamais hors de `bornes_dessin` -- sinon il laisse une
///     trainee la ou personne ne l'a invalide ;
///   * un calque peint TOUS les pixels de `opaque_sur` -- sinon l'occlusion
///     ecarte ce qu'il y a dessous pour des pixels qu'il ne remplit pas.
///
/// Chaque calque est peint SEUL sur un tampon vierge, ce qui rend son empreinte
/// reelle directement observable.
#[test]
fn chaque_calque_respecte_ses_bornes_et_son_opacite() {
    let cadre = Rect::neuf(20, 20, 80, 50);
    let menu = Rect::neuf(2, 60, 60, 44);
    let scene = Constructeur::neuf()
        .fenetre(0, cadre)
        .menu(menu)
        .finit((90, 40));

    for calque in &scene.calques {
        let mut seul = Tampon::neuf();
        scene.peins_calque(&mut seul, calque, &ecran());

        // 1. rien hors des bornes de dessin
        for y in 0..H {
            for x in 0..L {
                if seul.px[y * L + x] == VIDE {
                    continue;
                }
                let dedans = (x as i32) >= calque.bornes_dessin.x
                    && (y as i32) >= calque.bornes_dessin.y
                    && (x as i64) < calque.bornes_dessin.droite()
                    && (y as i64) < calque.bornes_dessin.bas();
                assert!(
                    dedans,
                    "{:?} peint en ({x},{y}), hors de ses bornes {:?} : \
                     ce pixel ne sera jamais invalide, donc il laissera une trainee",
                    calque.element, calque.bornes_dessin,
                );
            }
        }

        // 2. tout ce qui est declare opaque est reellement peint
        if let Some(opaque) = calque.opaque_sur {
            for y in opaque.y.max(0)..(opaque.bas().min(H as i64) as i32) {
                for x in opaque.x.max(0)..(opaque.droite().min(L as i64) as i32) {
                    assert_ne!(
                        seul.px[y as usize * L + x as usize],
                        VIDE,
                        "{:?} declare {:?} opaque mais ne peint pas ({x},{y}) : \
                         l'occlusion ecarterait ce qu'il y a dessous pour rien",
                        calque.element, opaque,
                    );
                }
            }
        }
    }
}

/// Un degat plein ecran doit evidemment converger : c'est le cas le plus
/// simple, et s'il echouait le rasteriseur lui-meme serait faux.
#[test]
fn un_degat_plein_ecran_converge() {
    let cadre = Rect::neuf(30, 25, 90, 50);
    let a = bureau((5, 5));
    let b = bureau_fenetre(cadre, (150, 90));
    verifie("degat plein ecran", &a, &b, &[ecran()]);
}

// ===========================================================================
// Garde sur le code reel
// ===========================================================================

/// `window_manager.rs` ne se compile pas sur l'hote. Ce garde lit sa source et
/// verifie qu'aucune invalidation ne repart du CADRE seul.
///
/// C'est exactement la divergence qui a produit les rectangles sombres : les
/// bornes du calque incluaient l'ombre, l'invalidation non. Les deux viennent
/// desormais de `empreinte_fenetre` / `empreinte_menu`, et ce test empeche de
/// les redissocier.
#[test]
fn aucune_invalidation_ne_repart_du_cadre_seul() {
    const SOURCE: &str = include_str!("../../src/gui/window_manager.rs");

    for motif in [
        "degats.ajoute(Origine::Fenetre, cadre_fenetre(",
        "degats.ajoute(Origine::Menu, depuis_widget(menu_rect())",
    ] {
        assert!(
            !SOURCE.contains(motif),
            "invalidation sur le cadre seul : `{motif}` laisse la bande d'ombre \
             a l'ecran. Passer par empreinte_fenetre / empreinte_menu."
        );
    }

    assert!(
        SOURCE.contains("fn empreinte_fenetre(")
            && SOURCE.contains("fn empreinte_menu("),
        "les deux empreintes doivent rester la source unique de ce qu'un calque \
         occupe a l'ecran"
    );
}

/// Le debordement de l'ombre n'a qu'UNE definition, dans `widgets`, parce que
/// trois endroits doivent s'accorder : ce qui est peint, ce qui est declare, ce
/// qui est invalide.
#[test]
fn le_debordement_de_l_ombre_n_a_qu_une_definition() {
    const WIDGETS: &str = include_str!("../../src/gui/widgets.rs");
    assert!(
        WIDGETS.contains("pub(crate) const DEBORD_OMBRE: i32 = 4;"),
        "DEBORD_OMBRE doit rester declare une seule fois dans widgets"
    );
    assert!(
        !WIDGETS.contains("fill_rect_rgb(x + 4, y + 4, ww, wh"),
        "l'ombre de fenetre doit passer par DEBORD_OMBRE, pas par un 4 en dur"
    );
    assert!(
        !WIDGETS.contains("fill_rect_rgb(mxi + 4, myi + 4, mw, mh"),
        "l'ombre de menu doit passer par DEBORD_OMBRE, pas par un 4 en dur"
    );
}
