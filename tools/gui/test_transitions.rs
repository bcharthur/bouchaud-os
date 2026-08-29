//! Oracle de transition d'etat : le degat annonce suffit-il a rendre l'image ?
//!
//! # La propriete
//!
//!     tampon      = rendu_complet(etat_A)
//!     mutation      A -> B
//!     degats      = ce que la mutation ANNONCE reellement (gui::transition)
//!     rendu_partiel(tampon, etat_B, degats)
//!     reference   = rendu_complet(etat_B)
//!     ASSERT        tampon == reference, bit pour bit, sur TOUT l'ecran
//!
//! Tout l'ecran, y compris hors degat. Un pixel qui differe hors degat est
//! exactement le defaut recherche : l'etat a change la, et personne ne l'a dit.
//!
//! # Pourquoi l'oracle de rendu ne suffisait pas
//!
//! `test_rendu.rs` compare, DANS un rectangle de degat, le rendu avec culling
//! au rendu sans culling. Il repond a « le culling change-t-il l'image ? ».
//!
//! Il ne peut pas repondre a « le degat couvre-t-il ce qui a change ? », parce
//! qu'il ne connait qu'un seul etat. Il etait vert pendant que l'horloge
//! invalidait la mauvaise barre, que le survol du menu oubliait l'ancienne
//! ligne et que le focus oubliait la fenetre qui le perdait — trois defauts
//! qui vivent tous dans le passage d'un etat a l'autre, et nulle part ailleurs.
//!
//! # Le rasteriseur
//!
//! Volontairement minimal : des rectangles et des couleurs. Ce qu'il respecte
//! scrupuleusement, ce sont les trois choses qui font l'objet du test :
//!
//!   * chaque calque peint UNIQUEMENT dans `bornes_dessin` ;
//!   * chaque calque peint TOUS les pixels de `opaque_sur` ;
//!   * les pixels de chaque calque sont une fonction PURE de l'etat — c'est ce
//!     qui rend `rendu_complet(B)` atteignable depuis un rendu partiel.
//!
//! Chaque dependance d'etat du vrai bureau y est representee : l'heure et les
//! statistiques dans la barre du haut, la couleur de barre de titre et de
//! bordure selon le focus, la ligne du menu survolee, le bouton Demarrer selon
//! l'ouverture du menu, les boutons de la barre des taches selon le focus,
//! l'ombre portee de 4 pixels, la surface d'un client.
//!
//! Lance par `tools/gui/test-transitions.sh`.

extern crate alloc;

#[path = "../../src/gui/protocole.rs"]
mod protocole;

#[path = "../../src/gui/disposition.rs"]
mod disposition;

#[path = "../../src/gui/scene.rs"]
mod scene;

#[path = "../../src/gui/transition.rs"]
mod transition;

use disposition::{BANDE_ACCENT, ENTETE_MENU, HAUTEUR_BARRE, HAUTEUR_LIGNE_MENU};
use protocole::Rect;
use scene::{Calque, Element};
use transition::Rects;

// ─── Ecran simule ──────────────────────────────────────────────────────────

const L: usize = 400;
const H: usize = 260;
const LIGNES_MENU: usize = 7;

fn ecran() -> Rect {
    Rect::neuf(0, 0, L as u32, H as u32)
}

/// Le menu, calcule comme `window::menu_rect`.
fn menu() -> Rect {
    let h = LIGNES_MENU as i32 * HAUTEUR_LIGNE_MENU + ENTETE_MENU + 8;
    Rect::neuf(2, H as i32 - HAUTEUR_BARRE as i32 - h, 178, h as u32)
}

fn barre_haute() -> Rect {
    disposition::barre_haute(L as u32)
}

fn barre_taches() -> Rect {
    disposition::barre_taches(L as u32, H as u32)
}

fn filigrane() -> Rect {
    Rect::neuf(L as i32 / 2 - 60, H as i32 - 64, 120, 24)
}

const ICONES: usize = 3;

/// Largeur du libelle de l'icone `index`. La deuxieme deborde de son carre,
/// comme « Calculatrice » deborde du sien a l'ecran.
fn largeur_libelle(index: usize) -> i32 {
    [40, 72, 30][index]
}

fn icone(position: (i32, i32)) -> Rect {
    Rect::neuf(position.0, position.1, 56, 60)
}

/// Ce que l'icone peint reellement : le carre, son ombre, et le libelle —
/// qui passe A GAUCHE du carre des qu'il est plus large que lui.
fn empreinte_icone(index: usize, position: (i32, i32)) -> Rect {
    let r = icone(position);
    let carre = 40i32;
    let vx = r.x + (r.largeur as i32 - carre) / 2;
    let lw = largeur_libelle(index);
    let lx = (r.x + (r.largeur as i32 - lw) / 2).max(0);
    let ly = r.y + carre + 3;

    let gauche = r.x.min(lx);
    let haut = r.y;
    let droite = (r.droite() as i32).max(vx + 3 + carre).max(lx + lw + 2);
    let bas = (r.bas() as i32).max(r.y + 3 + carre).max(ly + 16);
    Rect::neuf(gauche, haut, (droite - gauche) as u32, (bas - haut) as u32)
}

fn positions_initiales() -> [(i32, i32); ICONES] {
    [(8, 20), (8, 84), (8, 148)]
}

// Les boutons de la coquille, tels que `window::start_btn` et
// `window::taskbar_btn` les posent. Le modele suit la vraie geometrie : c'est
// elle que les degats de `transition` doivent couvrir.
const MARGE_BARRE: i32 = 4;
const BOUTON_H: u32 = HAUTEUR_BARRE - MARGE_BARRE as u32 * 2;
const DEMARRER_W: u32 = 74;
const TACHE_W: u32 = 148;
const TACHE_PAS: i32 = TACHE_W as i32 + 6;
const TACHE_X0: i32 = MARGE_BARRE + DEMARRER_W as i32 + 10;

fn sommet_barre_basse() -> i32 {
    H as i32 - HAUTEUR_BARRE as i32 + MARGE_BARRE
}

fn bouton_demarrer() -> Rect {
    Rect::neuf(MARGE_BARRE, sommet_barre_basse(), DEMARRER_W, BOUTON_H)
}

fn bouton_taches(index: usize) -> Rect {
    Rect::neuf(TACHE_X0 + index as i32 * TACHE_PAS, sommet_barre_basse(),
        TACHE_W, BOUTON_H)
}

/// Champ de l'heure : coin haut droit de la barre du haut.
fn champ_horloge() -> Rect {
    Rect::neuf(L as i32 - 90, MARGE_BARRE, 82, BOUTON_H)
}

/// Champ des statistiques CPU/RAM/Disque : centre de la barre du haut.
fn champ_stats() -> Rect {
    Rect::neuf(L as i32 / 2 - 140, MARGE_BARRE, 280, BOUTON_H)
}

// ─── Etat du bureau ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fen {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    min: bool,
    teinte: u32,
    /// Surface du client : la tache qu'il a peinte, en coordonnees locales.
    tache: Rect,
    couleur_tache: u32,
}

impl Fen {
    fn neuve(x: i32, y: i32, w: i32, h: i32, teinte: u32) -> Self {
        Self { x, y, w, h, min: false, teinte, tache: Rect::default(), couleur_tache: 0 }
    }

    fn cadre(&self) -> Rect {
        Rect::neuf(self.x, self.y, self.w.max(0) as u32, self.h.max(0) as u32)
    }

    fn empreinte(&self) -> Rect {
        disposition::empreinte_fenetre_peinte(self.cadre())
    }

    /// Interieur : sous la barre de titre, dans les bordures.
    fn interieur(&self) -> Rect {
        let cadre = self.cadre();
        if cadre.largeur < 4 || cadre.hauteur < 14 {
            return Rect::default();
        }
        Rect::neuf(cadre.x + 1, cadre.y + 11, cadre.largeur - 2, cadre.hauteur - 12)
    }

    /// La tache du client, ramenee a l'ecran.
    fn tache_ecran(&self) -> Rect {
        if self.tache.vide() {
            return Rect::default();
        }
        let interieur = self.interieur();
        Rect::neuf(
            interieur.x + self.tache.x,
            interieur.y + self.tache.y,
            self.tache.largeur,
            self.tache.hauteur,
        )
        .intersecte(&interieur)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Etat {
    souris: (i32, i32),
    menu_ouvert: bool,
    survol: Option<usize>,
    /// L'ordre EST le z-order. La derniere non minimisee a le focus.
    fenetres: alloc::vec::Vec<Fen>,
    icones: [(i32, i32); ICONES],
    horloge: u32,
    stats: u32,
}

impl Etat {
    fn focus(&self) -> Option<usize> {
        self.fenetres.iter().rposition(|f| !f.min)
    }

    /// Le survol, recalcule comme le fait le gestionnaire de fenetres.
    fn survol_attendu(&self) -> Option<usize> {
        if !self.menu_ouvert {
            return None;
        }
        disposition::ligne_menu_survolee(menu(), self.souris.0, self.souris.1)
    }

    /// Deplace la souris ET met le survol a jour, comme la boucle reelle.
    fn avec_souris(&self, x: i32, y: i32) -> Etat {
        let mut suivant = self.clone();
        suivant.souris = (x, y);
        suivant.survol = suivant.survol_attendu();
        suivant
    }
}

fn bureau_vide() -> Etat {
    Etat {
        souris: (200, 130),
        menu_ouvert: false,
        survol: None,
        fenetres: alloc::vec::Vec::new(),
        icones: positions_initiales(),
        horloge: 0,
        stats: 0,
    }
}

fn bureau_une_fenetre() -> Etat {
    let mut etat = bureau_vide();
    etat.fenetres.push(Fen::neuve(60, 40, 140, 90, 0x00_10_00));
    etat
}

fn bureau_deux_fenetres() -> Etat {
    let mut etat = bureau_une_fenetre();
    etat.fenetres.push(Fen::neuve(180, 90, 150, 100, 0x10_00_00));
    etat
}

// ─── Toile ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Toile {
    pixels: alloc::vec::Vec<u32>,
    clip: Rect,
}

impl Toile {
    fn neuve() -> Self {
        Self { pixels: alloc::vec![0u32; L * H], clip: ecran() }
    }

    fn remplis(&mut self, rect: Rect, couleur: u32) {
        let zone = rect.intersecte(&self.clip).intersecte(&ecran());
        if zone.vide() {
            return;
        }
        for y in zone.y..(zone.bas() as i32) {
            let base = y as usize * L;
            for x in zone.x..(zone.droite() as i32) {
                self.pixels[base + x as usize] = couleur;
            }
        }
    }

    fn contour(&mut self, rect: Rect, couleur: u32) {
        if rect.vide() {
            return;
        }
        self.remplis(Rect::neuf(rect.x, rect.y, rect.largeur, 1), couleur);
        self.remplis(
            Rect::neuf(rect.x, rect.bas() as i32 - 1, rect.largeur, 1),
            couleur,
        );
        self.remplis(Rect::neuf(rect.x, rect.y, 1, rect.hauteur), couleur);
        self.remplis(
            Rect::neuf(rect.droite() as i32 - 1, rect.y, 1, rect.hauteur),
            couleur,
        );
    }

    /// Premier pixel qui differe, en balayage ligne par ligne.
    fn premiere_difference(&self, autre: &Toile) -> Option<(usize, usize, u32, u32)> {
        for index in 0..(L * H) {
            if self.pixels[index] != autre.pixels[index] {
                return Some((
                    index % L,
                    index / L,
                    self.pixels[index],
                    autre.pixels[index],
                ));
            }
        }
        None
    }
}

// ─── Peintres : pixels = fonction pure de l'etat ───────────────────────────

fn peins_fond(toile: &mut Toile) {
    for y in 0..H {
        let teinte = (y as u32 % 13) * 0x00_01_01;
        toile.remplis(Rect::neuf(0, y as i32, L as u32, 1), 0x08_0e_1c + teinte);
    }
}

fn peins_filigrane(toile: &mut Toile) {
    toile.remplis(filigrane(), 0x33_47_6b);
}

fn peins_icone(toile: &mut Toile, index: usize, etat: &Etat) {
    let position = etat.icones[index];
    let r = icone(position);
    let carre = 40i32;
    let vx = r.x + (r.largeur as i32 - carre) / 2;
    toile.remplis(Rect::neuf(vx + 3, r.y + 3, carre as u32, carre as u32), 0x06_09_0f);
    toile.remplis(
        Rect::neuf(vx, r.y, carre as u32, carre as u32),
        0x20_50_20 + index as u32 * 0x10_10_10,
    );
    // Libelle, centre sur le carre mais pas contraint par lui.
    let lw = largeur_libelle(index);
    let lx = (r.x + (r.largeur as i32 - lw) / 2).max(0);
    let ly = r.y + carre + 3;
    toile.remplis(Rect::neuf(lx + 1, ly + 1, lw as u32, 14), 0x00_00_00);
    toile.remplis(Rect::neuf(lx, ly, lw as u32, 14), 0xe8_f4_fd);
}

fn peins_barre_haute(toile: &mut Toile, etat: &Etat) {
    toile.remplis(barre_haute(), 0x0d_1a_30);
    // Les deux seuls champs du bureau qui changent sans que personne l'annonce.
    toile.remplis(champ_horloge(), 0xf0_c0_60 ^ (etat.horloge * 0x00_01_07));
    toile.remplis(champ_stats(), 0x5a_b4_d6 ^ (etat.stats * 0x01_00_03));
}

fn peins_fenetre(toile: &mut Toile, fen: &Fen, focalisee: bool) {
    let cadre = fen.cadre();
    if cadre.vide() {
        return;
    }
    // Ombre portee : `paint_window_shape` peint des anneaux AUTOUR du cadre,
    // donc elle deborde des quatre cotes. Le modele suit exactement ce que
    // `disposition::empreinte_fenetre_peinte` declare, sinon l'oracle
    // validerait un degat qui ne couvre pas ce que le noyau peint.
    toile.remplis(disposition::empreinte_fenetre_peinte(cadre), 0x05_07_0a);
    toile.remplis(cadre, 0x2a_2a_3a + fen.teinte);
    // Barre de titre et bordures : leur couleur depend du FOCUS.
    let couleur_focus = if focalisee { 0x25_63_eb } else { 0x1f_29_37 };
    toile.remplis(Rect::neuf(cadre.x, cadre.y, cadre.largeur, 10), couleur_focus);
    toile.contour(cadre, couleur_focus);
    // Surface du client : ce qu'il a peint, et lui seul.
    toile.remplis(fen.tache_ecran(), fen.couleur_tache);
}

fn peins_menu(toile: &mut Toile, etat: &Etat) {
    let m = menu();
    // Le menu garde une ombre DECALEE : `draw_menu` peint une copie du cadre
    // translatee vers le bas et la droite.
    let debord = disposition::DEBORD_OMBRE as i32;
    toile.remplis(
        Rect::neuf(m.x + debord, m.y + debord, m.largeur, m.hauteur),
        0x05_07_0a,
    );
    toile.remplis(m, 0x14_20_3a);
    toile.remplis(Rect::neuf(m.x, m.y, BANDE_ACCENT as u32, m.hauteur), 0x25_63_eb);
    for index in 0..disposition::lignes_menu(m) {
        let ligne = disposition::rect_ligne_menu(m, index);
        let survolee = etat.survol == Some(index);
        let corps = Rect::neuf(
            ligne.x + BANDE_ACCENT,
            ligne.y,
            ligne.largeur - BANDE_ACCENT as u32,
            ligne.hauteur,
        );
        toile.remplis(corps, if survolee { 0x1e_3a_6b } else { 0x14_20_3a });
        if survolee {
            toile.remplis(
                Rect::neuf(ligne.x + BANDE_ACCENT, ligne.y, 2, ligne.hauteur),
                0x60_a5_fa,
            );
        }
        // Libelle : couleur et graisse changent avec le survol.
        toile.remplis(
            Rect::neuf(ligne.x + BANDE_ACCENT + 24, ligne.y + 6, 100, 10),
            if survolee { 0xff_ff_ff } else { 0xb8_d0_ee },
        );
    }
}

fn peins_barre_taches(toile: &mut Toile, etat: &Etat) {
    toile.remplis(barre_taches(), 0x0d_1a_30);
    toile.remplis(
        bouton_demarrer(),
        if etat.menu_ouvert { 0x0c_5c_bf } else { 0x1a_3f_6b },
    );
    let focus = etat.focus();
    for index in 0..etat.fenetres.len() {
        let bouton = bouton_taches(index);
        if bouton.droite() > L as i64 {
            break;
        }
        toile.remplis(
            bouton,
            if focus == Some(index) { 0x2a_5a_aa } else { 0x1e_34_62 },
        );
    }
}

fn peins_curseur(toile: &mut Toile, etat: &Etat) {
    let (x, y) = etat.souris;
    // BOUCHAUD_GUI_CURSEUR_ADAPTATIF_V1 : la couleur de la fleche est lue SOUS
    // son point chaud, exactement comme `widgets::draw_cursor`. C'est de l'etat
    // derive du tampon compose, et c'est ce qui rend un degat partiel visible.
    let px = (x.max(0) as usize).min(L - 1);
    let py = (y.max(0) as usize).min(H - 1);
    let fond = toile.pixels[py * L + px];
    let luminance = ((fond >> 16 & 0xff) * 299 + (fond >> 8 & 0xff) * 587 + (fond & 0xff) * 114) / 1000;
    let couleur = if luminance > 140 { 0x00_00_00 } else { 0xff_ff_ff };
    for ligne in 0..19i32 {
        let largeur = if ligne < 7 { ligne + 1 } else { 19 - ligne };
        if largeur <= 0 {
            continue;
        }
        toile.remplis(Rect::neuf(x, y + ligne, largeur as u32, 1), couleur);
    }
}

// ─── Plan de scene : le meme que `window_manager::plan_de_scene` ───────────

fn plan(etat: &Etat) -> alloc::vec::Vec<Calque> {
    let mut calques = alloc::vec::Vec::new();
    calques.push(Calque::plein(Element::Fond, ecran()));
    calques.push(Calque::transparent(Element::Filigrane, filigrane()));
    for index in 0..ICONES {
        calques.push(Calque::transparent(
            Element::Icone(index),
            empreinte_icone(index, etat.icones[index]),
        ));
    }
    calques.push(Calque::plein(Element::BarreHaute, barre_haute()));
    for (index, fen) in etat.fenetres.iter().enumerate() {
        if fen.min {
            continue;
        }
        calques.push(Calque::avec_ombre(
            Element::Fenetre(index),
            fen.empreinte(),
            fen.cadre(),
        ));
    }
    if etat.menu_ouvert {
        calques.push(Calque::avec_ombre(
            Element::Menu,
            disposition::empreinte_avec_ombre(menu()),
            menu(),
        ));
    }
    calques.push(Calque::plein(Element::BarreTaches, barre_taches()));
    calques.push(Calque::transparent(
        Element::Curseur,
        disposition::curseur(etat.souris.0, etat.souris.1),
    ));
    calques
}

fn peins_calque(toile: &mut Toile, calque: &Calque, etat: &Etat) {
    match calque.element {
        Element::Fond => peins_fond(toile),
        Element::Filigrane => peins_filigrane(toile),
        Element::Icone(index) => peins_icone(toile, index, etat),
        Element::BarreHaute => peins_barre_haute(toile, etat),
        Element::Fenetre(index) => {
            if let Some(fen) = etat.fenetres.get(index) {
                peins_fenetre(toile, fen, etat.focus() == Some(index));
            }
        }
        Element::Menu => peins_menu(toile, etat),
        Element::BarreTaches => peins_barre_taches(toile, etat),
        Element::Curseur => peins_curseur(toile, etat),
    }
}

/// Rendu de reference : tous les calques, plein ecran, aucun culling.
fn rendu_complet(etat: &Etat) -> Toile {
    let mut toile = Toile::neuve();
    toile.clip = ecran();
    for calque in plan(etat).iter() {
        peins_calque(&mut toile, calque, etat);
    }
    toile
}

/// Rendu partiel : exactement le pipeline du compositeur, degat par degat.
fn rendu_partiel(toile: &mut Toile, etat: &Etat, degats: &[Rect]) {
    let calques = plan(etat);
    for zone in degats.iter().copied() {
        let zone = zone.intersecte(&ecran());
        if zone.vide() {
            continue;
        }
        toile.clip = zone;
        let debut = scene::premier_calque(&calques, &zone);
        for calque in &calques[debut..] {
            if !scene::doit_dessiner(calque, &zone) {
                continue;
            }
            peins_calque(toile, calque, etat);
        }
    }
    toile.clip = ecran();
}

// ─── L'oracle ──────────────────────────────────────────────────────────────

fn quel_calque(etat: &Etat, x: usize, y: usize) -> alloc::string::String {
    use alloc::string::ToString;
    let point = Rect::neuf(x as i32, y as i32, 1, 1);
    let mut noms = alloc::vec::Vec::new();
    for calque in plan(etat).iter() {
        if !calque.bornes_dessin.intersecte(&point).vide() {
            noms.push(alloc::format!("{:?}", calque.element));
        }
    }
    if noms.is_empty() {
        "aucun".to_string()
    } else {
        noms.join(", ")
    }
}

/// LA propriete. Rendue depuis A, mutee vers B, repeinte sur les seuls degats
/// annonces, comparee au rendu complet de B sur TOUT l'ecran.
#[track_caller]
fn oracle(nom: &str, avant: &Etat, apres: &Etat, degats: &[Rect]) {
    // Le compositeur applique cette regle une fois par trame, sur les degats
    // accumules. L'oracle fait donc de meme, sinon il testerait autre chose.
    let mut degats = degats.to_vec();
    let recoloration = liste(transition::recoloration_curseur(&degats, apres.souris));
    degats.extend(recoloration);
    let degats = &degats[..];

    let mut test = rendu_complet(avant);
    rendu_partiel(&mut test, apres, degats);
    let reference = rendu_complet(apres);

    if let Some((x, y, obtenu, attendu)) = test.premiere_difference(&reference) {
        let couvert = degats
            .iter()
            .any(|d| !d.intersecte(&Rect::neuf(x as i32, y as i32, 1, 1)).vide());
        panic!(
            "\n  TRANSITION   {nom}\
             \n  PIXEL        ({x}, {y})\
             \n  A L'ECRAN    {obtenu:#08x}   (reste de l'etat A)\
             \n  ATTENDU      {attendu:#08x}   (etat B)\
             \n  DANS UN DEGAT ? {couvert}\
             \n  CALQUES ICI  {calques}\
             \n  DEGATS ({nombre}) {liste:?}\
             \n\n  Un pixel change sans degat correspondant : la mutation ne dit\
             \n  pas tout ce qu'elle change.\n",
            nom = nom,
            x = x,
            y = y,
            obtenu = obtenu,
            attendu = attendu,
            couvert = couvert,
            calques = quel_calque(apres, x, y),
            nombre = degats.len(),
            liste = degats
                .iter()
                .map(|d| (d.x, d.y, d.largeur, d.hauteur))
                .collect::<alloc::vec::Vec<_>>(),
        );
    }
}

fn liste(rects: Rects) -> alloc::vec::Vec<Rect> {
    rects.rects().collect()
}

/// Le degat plein ecran, tel que `Degats::tout` le produit.
fn tout() -> alloc::vec::Vec<Rect> {
    alloc::vec![ecran()]
}

// ─── Curseur ───────────────────────────────────────────────────────────────

#[test]
fn curseur_sur_le_fond() {
    let a = bureau_vide().avec_souris(120, 150);
    let b = a.avec_souris(190, 172);
    let degats = liste(transition::curseur_deplace(Some(a.souris), b.souris));
    oracle("curseur A -> B sur le fond", &a, &b, &degats);
}

#[test]
fn curseur_dans_une_fenetre() {
    let a = bureau_une_fenetre().avec_souris(80, 60);
    let b = a.avec_souris(140, 100);
    let degats = liste(transition::curseur_deplace(Some(a.souris), b.souris));
    oracle("curseur A -> B dans une fenetre", &a, &b, &degats);
}

#[test]
fn curseur_d_un_pixel() {
    let a = bureau_deux_fenetres().avec_souris(200, 120);
    let b = a.avec_souris(201, 120);
    let degats = liste(transition::curseur_deplace(Some(a.souris), b.souris));
    oracle("curseur d'un pixel", &a, &b, &degats);
}

#[test]
fn curseur_sur_le_bord_de_l_ecran() {
    let a = bureau_vide().avec_souris(L as i32 - 20, H as i32 - 30);
    let b = a.avec_souris(L as i32 - 2, H as i32 - 4);
    let degats = liste(transition::curseur_deplace(Some(a.souris), b.souris));
    oracle("curseur au bord", &a, &b, &degats);
}

// ─── Menu ──────────────────────────────────────────────────────────────────

/// Position du pointeur au milieu de la ligne `index`.
fn sur_ligne(index: usize) -> (i32, i32) {
    let ligne = disposition::rect_ligne_menu(menu(), index);
    (ligne.x + 40, ligne.y + HAUTEUR_LIGNE_MENU / 2)
}

fn menu_ouvert_sur(index: usize) -> Etat {
    let mut etat = bureau_une_fenetre();
    etat.menu_ouvert = true;
    let (x, y) = sur_ligne(index);
    etat.avec_souris(x, y)
}

/// Le degat complet d'un deplacement DANS le menu : le curseur, et le survol.
fn degats_deplacement(avant: &Etat, apres: &Etat) -> alloc::vec::Vec<Rect> {
    let mut rects = liste(transition::curseur_deplace(Some(avant.souris), apres.souris));
    rects.extend(liste(transition::survol_menu_change(
        menu(),
        avant.survol,
        apres.survol,
    )));
    rects
}

#[test]
fn curseur_entre_deux_lignes_du_menu() {
    let a = menu_ouvert_sur(2);
    let b = {
        let (x, y) = sur_ligne(3);
        a.avec_souris(x, y)
    };
    assert_eq!((a.survol, b.survol), (Some(2), Some(3)));
    oracle(
        "curseur ligne 2 -> ligne 3",
        &a,
        &b,
        &degats_deplacement(&a, &b),
    );
}

#[test]
fn curseur_saute_par_dessus_une_ligne() {
    let a = menu_ouvert_sur(0);
    let b = {
        let (x, y) = sur_ligne(5);
        a.avec_souris(x, y)
    };
    oracle("curseur ligne 0 -> ligne 5", &a, &b, &degats_deplacement(&a, &b));
}

#[test]
fn curseur_entre_dans_le_menu() {
    let mut a = bureau_une_fenetre();
    a.menu_ouvert = true;
    let m = menu();
    let a = a.avec_souris(m.droite() as i32 + 30, m.y + 40);
    let b = {
        let (x, y) = sur_ligne(1);
        a.avec_souris(x, y)
    };
    assert_eq!((a.survol, b.survol), (None, Some(1)));
    oracle("curseur entre dans le menu", &a, &b, &degats_deplacement(&a, &b));
}

#[test]
fn curseur_sort_du_menu() {
    let a = menu_ouvert_sur(4);
    let m = menu();
    let b = a.avec_souris(m.droite() as i32 + 30, m.y + 40);
    assert_eq!((a.survol, b.survol), (Some(4), None));
    oracle("curseur sort du menu", &a, &b, &degats_deplacement(&a, &b));
}

#[test]
fn curseur_sort_du_menu_par_le_bas() {
    let a = menu_ouvert_sur(LIGNES_MENU - 1);
    let m = menu();
    let b = a.avec_souris(m.x + 40, m.bas() as i32 - 2);
    assert_eq!(b.survol, None, "la marge basse du menu ne survole rien");
    oracle("curseur sort par la marge basse", &a, &b, &degats_deplacement(&a, &b));
}

#[test]
fn curseur_sort_du_menu_par_la_bande_d_accent() {
    let a = menu_ouvert_sur(3);
    let ligne = disposition::rect_ligne_menu(menu(), 3);
    let b = a.avec_souris(ligne.x, ligne.y + 5);
    assert_eq!(b.survol, None);
    oracle("curseur sur la bande d'accent", &a, &b, &degats_deplacement(&a, &b));
}

#[test]
fn chaque_paire_de_lignes_voisines() {
    for depart in 0..(LIGNES_MENU - 1) {
        let a = menu_ouvert_sur(depart);
        let b = {
            let (x, y) = sur_ligne(depart + 1);
            a.avec_souris(x, y)
        };
        oracle(
            &alloc::format!("ligne {depart} -> {}", depart + 1),
            &a,
            &b,
            &degats_deplacement(&a, &b),
        );
    }
}

#[test]
fn le_menu_s_ouvre() {
    let mut a = bureau_une_fenetre();
    let (x, y) = sur_ligne(2);
    a = a.avec_souris(x, y);
    let mut b = a.clone();
    b.menu_ouvert = true;
    b.survol = b.survol_attendu();
    assert_eq!(b.survol, Some(2), "le menu s'ouvre sous le pointeur");
    let degats = liste(transition::menu_bascule(menu(), barre_taches()));
    oracle("le menu s'ouvre", &a, &b, &degats);
}

#[test]
fn le_menu_se_ferme() {
    let a = menu_ouvert_sur(2);
    let mut b = a.clone();
    b.menu_ouvert = false;
    b.survol = None;
    let degats = liste(transition::menu_bascule(menu(), barre_taches()));
    oracle("le menu se ferme", &a, &b, &degats);
}

// ─── Fenetres ──────────────────────────────────────────────────────────────

fn bouge(etat: &Etat, index: usize, dx: i32, dy: i32) -> Etat {
    let mut suivant = etat.clone();
    suivant.fenetres[index].x += dx;
    suivant.fenetres[index].y += dy;
    suivant
}

#[test]
fn fenetre_deplacee_d_un_pixel() {
    let a = bureau_deux_fenetres();
    let index = a.fenetres.len() - 1;
    let b = bouge(&a, index, 1, 0);
    let degats = liste(transition::fenetre_bougee(
        a.fenetres[index].cadre(),
        b.fenetres[index].cadre(),
    ));
    oracle("fenetre + 1 px", &a, &b, &degats);
}

#[test]
fn fenetre_deplacee_de_cinquante_pixels() {
    let a = bureau_deux_fenetres();
    let index = a.fenetres.len() - 1;
    let b = bouge(&a, index, 50, 30);
    let degats = liste(transition::fenetre_bougee(
        a.fenetres[index].cadre(),
        b.fenetres[index].cadre(),
    ));
    oracle("fenetre + 50 px", &a, &b, &degats);
}

#[test]
fn fenetre_deplacee_dans_les_quatre_directions() {
    for (dx, dy) in [(-30, 0), (30, 0), (0, -25), (0, 25), (-20, -20), (40, 35)] {
        let a = bureau_deux_fenetres();
        let index = a.fenetres.len() - 1;
        let b = bouge(&a, index, dx, dy);
        let degats = liste(transition::fenetre_bougee(
            a.fenetres[index].cadre(),
            b.fenetres[index].cadre(),
        ));
        oracle(&alloc::format!("fenetre ({dx}, {dy})"), &a, &b, &degats);
    }
}

/// Rafale : plusieurs mutations, AUCUNE composition entre elles.
///
/// C'est ce qui se passe quand la souris produit plus d'evenements que le
/// compositeur ne fait de trames : les degats s'accumulent et une seule trame
/// les presente tous. Une transition dont le degat n'est juste que compose
/// immediatement se voit ici.
#[test]
fn rafale_de_deplacements_sans_composition() {
    let depart = bureau_deux_fenetres();
    let index = depart.fenetres.len() - 1;
    let mut courant = depart.clone();
    let mut degats = alloc::vec::Vec::new();
    for pas in 0..8 {
        let suivant = bouge(&courant, index, 7, if pas % 2 == 0 { 3 } else { -2 });
        degats.extend(liste(transition::fenetre_bougee(
            courant.fenetres[index].cadre(),
            suivant.fenetres[index].cadre(),
        )));
        courant = suivant;
    }
    oracle("rafale de 8 deplacements", &depart, &courant, &degats);
}

#[test]
fn rafale_de_deplacements_du_curseur() {
    let depart = bureau_une_fenetre().avec_souris(50, 50);
    let mut courant = depart.clone();
    let mut degats = alloc::vec::Vec::new();
    for pas in 0..12 {
        let suivant = courant.avec_souris(50 + pas * 11, 50 + pas * 7);
        degats.extend(liste(transition::curseur_deplace(
            Some(courant.souris),
            suivant.souris,
        )));
        courant = suivant;
    }
    oracle("rafale de 12 deplacements du curseur", &depart, &courant, &degats);
}

#[test]
fn fenetre_redimensionnee() {
    for (dw, dh) in [(40, 0), (0, 30), (40, 30), (-25, -20)] {
        let a = bureau_deux_fenetres();
        let index = a.fenetres.len() - 1;
        let mut b = a.clone();
        b.fenetres[index].w += dw;
        b.fenetres[index].h += dh;
        let degats = liste(transition::fenetre_bougee(
            a.fenetres[index].cadre(),
            b.fenetres[index].cadre(),
        ));
        oracle(&alloc::format!("resize ({dw}, {dh})"), &a, &b, &degats);
    }
}

#[test]
fn fenetre_maximisee_puis_restauree() {
    let a = bureau_deux_fenetres();
    let index = a.fenetres.len() - 1;
    let mut b = a.clone();
    b.fenetres[index] = Fen {
        x: 0,
        y: HAUTEUR_BARRE as i32,
        w: L as i32,
        h: H as i32 - 2 * HAUTEUR_BARRE as i32,
        ..a.fenetres[index]
    };
    let maximiser = liste(transition::fenetre_bougee(
        a.fenetres[index].cadre(),
        b.fenetres[index].cadre(),
    ));
    oracle("maximiser", &a, &b, &maximiser);

    let restaurer = liste(transition::fenetre_bougee(
        b.fenetres[index].cadre(),
        a.fenetres[index].cadre(),
    ));
    oracle("restaurer", &b, &a, &restaurer);
}

#[test]
fn fenetre_minimisee_puis_restauree() {
    // Production annonce le plein ecran dans les deux sens : ce qui etait sous
    // la fenetre n'a jamais ete dessine, et personne d'autre que le bureau ne
    // le sait. L'oracle verifie que ce degat suffit, et surtout que l'etat B
    // est bien atteignable.
    let a = bureau_deux_fenetres();
    let index = a.fenetres.len() - 1;
    let mut b = a.clone();
    b.fenetres[index].min = true;
    let m = b.fenetres.pop().unwrap();
    b.fenetres.insert(0, m);
    oracle("minimiser", &a, &b, &tout());

    let mut c = b.clone();
    let mut m = c.fenetres.remove(0);
    m.min = false;
    c.fenetres.push(m);
    oracle("restaurer depuis la barre", &b, &c, &tout());
}

#[test]
fn le_focus_passe_d_une_fenetre_a_l_autre() {
    let a = bureau_deux_fenetres();
    // On remonte la fenetre 0 : elle prend le focus, la 1 le perd.
    let mut b = a.clone();
    let remontee = b.fenetres.remove(0);
    b.fenetres.push(remontee);

    let cadre_perdu = a.fenetres[a.focus().unwrap()].cadre();
    let cadre_gagne = a.fenetres[0].cadre();
    let degats = liste(transition::focus_transfere(
        Some(cadre_perdu),
        cadre_gagne,
        barre_taches(),
    ));
    oracle("focus fenetre 1 -> fenetre 0", &a, &b, &degats);
}

#[test]
fn le_focus_passe_entre_fenetres_disjointes() {
    // Le cas qui compte : les deux fenetres ne se touchent pas, donc le degat
    // de celle qui monte ne peut rien repeindre de celle qui descend.
    let mut a = bureau_vide();
    a.fenetres.push(Fen::neuve(20, 30, 100, 70, 0x00_10_00));
    a.fenetres.push(Fen::neuve(240, 150, 120, 80, 0x10_00_00));
    assert!(
        a.fenetres[0].empreinte().intersecte(&a.fenetres[1].empreinte()).vide(),
        "les deux fenetres doivent etre disjointes pour que le test ait du sens"
    );
    let mut b = a.clone();
    let remontee = b.fenetres.remove(0);
    b.fenetres.push(remontee);

    let degats = liste(transition::focus_transfere(
        Some(a.fenetres[1].cadre()),
        a.fenetres[0].cadre(),
        barre_taches(),
    ));
    oracle("focus entre fenetres disjointes", &a, &b, &degats);
}

#[test]
fn une_fenetre_se_ferme() {
    let a = bureau_deux_fenetres();
    let mut b = a.clone();
    b.fenetres.pop();
    oracle("fermeture de la fenetre du dessus", &a, &b, &tout());

    let mut c = a.clone();
    c.fenetres.remove(0);
    oracle("fermeture d'une fenetre du dessous", &a, &c, &tout());
}

#[test]
fn une_fenetre_apparait() {
    let a = bureau_une_fenetre();
    let mut b = a.clone();
    b.fenetres.push(Fen::neuve(90, 120, 160, 90, 0x00_00_10));
    oracle("nouvelle fenetre", &a, &b, &tout());
}

// ─── Client ring 3 ─────────────────────────────────────────────────────────

#[test]
fn un_client_repeint_une_partie_de_sa_surface() {
    let mut a = bureau_deux_fenetres();
    let index = a.fenetres.len() - 1;
    a.fenetres[index].tache = Rect::neuf(10, 8, 60, 30);
    a.fenetres[index].couleur_tache = 0x22_44_66;

    let mut b = a.clone();
    b.fenetres[index].couleur_tache = 0xaa_33_11;

    // Le client annonce EXACTEMENT ce qu'il a repeint, en coordonnees ecran.
    let degats = alloc::vec![b.fenetres[index].tache_ecran()];
    oracle("trame d'un client", &a, &b, &degats);
}

#[test]
fn un_client_agrandit_sa_tache() {
    let mut a = bureau_une_fenetre();
    a.fenetres[0].tache = Rect::neuf(10, 8, 40, 20);
    a.fenetres[0].couleur_tache = 0x22_44_66;

    let mut b = a.clone();
    b.fenetres[0].tache = Rect::neuf(10, 8, 90, 50);
    b.fenetres[0].couleur_tache = 0x22_44_66;

    // L'union : ce qui etait peint, et ce qui l'est maintenant.
    let degats = alloc::vec![a.fenetres[0].tache_ecran(), b.fenetres[0].tache_ecran()];
    oracle("le client agrandit sa tache", &a, &b, &degats);
}

// ─── Ce que chaque rectangle VISE ──────────────────────────────────────────
//
// Une transition touche parfois deux elements de nature differente. Si tous ses
// rectangles portent la meme etiquette, les degats restent JUSTES -- les bons
// pixels sont presentes -- mais la mesure devient fausse, et un compteur a zero
// fait chercher un bug qui n'existe pas.
//
// C'est arrive : `[GUI-DAMAGE] taskbar=0` sur une session entiere, pendant que
// la barre des taches etait repeinte des dizaines de fois.

use transition::Cible;

fn cibles(rects: Rects) -> alloc::vec::Vec<Cible> {
    rects.iter().map(|(_, cible)| cible).collect()
}

#[test]
fn ouvrir_le_menu_vise_le_menu_ET_la_barre_des_taches() {
    let vues = cibles(transition::menu_bascule(menu(), barre_taches()));
    assert!(vues.contains(&Cible::Menu), "le menu lui-meme");
    assert!(
        vues.contains(&Cible::BarreTaches),
        "le bouton Demarrer change de couleur : c'est la barre des taches, \
         et la trace doit le dire ({vues:?})"
    );
}

#[test]
fn changer_le_focus_vise_les_fenetres_ET_la_barre_des_taches() {
    let vues = cibles(transition::focus_transfere(
        Some(Rect::neuf(10, 20, 100, 80)),
        Rect::neuf(50, 60, 100, 80),
        barre_taches(),
    ));
    assert_eq!(
        vues.iter().filter(|c| **c == Cible::Fenetre).count(),
        2,
        "celle qui gagne le focus et celle qui le perd"
    );
    assert!(vues.contains(&Cible::BarreTaches), "les boutons suivent le focus");
}

#[test]
fn le_tic_d_horloge_vise_la_barre_du_haut() {
    assert_eq!(
        cibles(transition::tic_horloge(L as u32)),
        alloc::vec![Cible::BarreHaute],
    );
}

#[test]
fn un_deplacement_du_curseur_ne_vise_que_le_curseur() {
    let vues = cibles(transition::curseur_deplace(Some((10, 10)), (80, 90)));
    assert_eq!(vues, alloc::vec![Cible::Curseur, Cible::Curseur]);
}

#[test]
fn le_survol_du_menu_ne_vise_que_le_menu() {
    let vues = cibles(transition::survol_menu_change(menu(), Some(1), Some(3)));
    assert_eq!(vues, alloc::vec![Cible::Menu, Cible::Menu]);
}

#[test]
fn un_deplacement_de_fenetre_ne_vise_que_la_fenetre() {
    let vues = cibles(transition::fenetre_bougee(
        Rect::neuf(10, 20, 100, 80),
        Rect::neuf(60, 20, 100, 80),
    ));
    assert_eq!(vues, alloc::vec![Cible::Fenetre, Cible::Fenetre]);
}

/// La recoloration vise le curseur, pas ce qui l'a declenchee.
#[test]
fn la_recoloration_du_curseur_vise_le_curseur() {
    let empreinte = disposition::curseur(100, 100);
    let vues = cibles(transition::recoloration_curseur(&[empreinte], (100, 100)));
    assert_eq!(vues, alloc::vec![Cible::Curseur]);
}

// ─── Icones du bureau ──────────────────────────────────────────────────────

fn deplace_icone(etat: &Etat, index: usize, dx: i32, dy: i32) -> Etat {
    let mut suivant = etat.clone();
    suivant.icones[index].0 += dx;
    suivant.icones[index].1 += dy;
    suivant
}

/// Le libelle deborde du carre de l'icone : c'est le cas qui compte.
#[test]
fn une_icone_au_libelle_large_est_deplacee() {
    let a = bureau_vide();
    assert!(
        largeur_libelle(1) > icone(a.icones[1]).largeur as i32,
        "l'icone 1 doit avoir un libelle plus large que son carre"
    );
    for (dx, dy) in [(1, 0), (0, 1), (37, 24), (-4, 40)] {
        let b = deplace_icone(&a, 1, dx, dy);
        let degats = alloc::vec![
            empreinte_icone(1, a.icones[1]),
            empreinte_icone(1, b.icones[1]),
        ];
        oracle(&alloc::format!("icone large ({dx}, {dy})"), &a, &b, &degats);
    }
}

#[test]
fn une_icone_au_libelle_etroit_est_deplacee() {
    let a = bureau_vide();
    let b = deplace_icone(&a, 2, 30, 20);
    let degats = alloc::vec![
        empreinte_icone(2, a.icones[2]),
        empreinte_icone(2, b.icones[2]),
    ];
    oracle("icone etroite deplacee", &a, &b, &degats);
}

// ─── Curseur adaptatif ─────────────────────────────────────────────────────
//
// `draw_cursor` lit le pixel sous son point chaud pour choisir sa couleur. Un
// degat qui repeint ce pixel change donc TOUTE la fleche, y compris ce qu'il
// ne couvre pas.

/// Un bureau ou le curseur pose son point chaud sur une tache claire, plus
/// petite que son empreinte.
fn bureau_curseur_sur_tache_claire() -> Etat {
    let mut etat = bureau_vide();
    let mut fen = Fen::neuve(60, 40, 160, 110, 0x00_10_00);
    fen.tache = Rect::neuf(10, 8, 24, 6);
    fen.couleur_tache = 0xff_ff_ff;
    etat.fenetres.push(fen);
    let tache = etat.fenetres[0].tache_ecran();
    etat.avec_souris(tache.x + 4, tache.y + 2)
}

#[test]
fn le_fond_sous_le_point_chaud_change_de_luminance() {
    let a = bureau_curseur_sur_tache_claire();
    let mut b = a.clone();
    b.fenetres[0].couleur_tache = 0x08_08_08;

    let tache = b.fenetres[0].tache_ecran();
    let empreinte = disposition::curseur(a.souris.0, a.souris.1);
    assert!(
        !tache.intersecte(&empreinte).vide() && tache.intersecte(&empreinte) != empreinte,
        "la tache doit couvrir le point chaud SANS couvrir toute la fleche"
    );

    // Le client annonce exactement ce qu'il a repeint, et rien d'autre.
    oracle("le fond sous le curseur s'assombrit", &a, &b, &alloc::vec![tache]);
}

#[test]
fn le_fond_sous_le_point_chaud_s_eclaircit() {
    let mut a = bureau_curseur_sur_tache_claire();
    a.fenetres[0].couleur_tache = 0x08_08_08;
    let mut b = a.clone();
    b.fenetres[0].couleur_tache = 0xff_ff_ff;
    let tache = b.fenetres[0].tache_ecran();
    oracle("le fond sous le curseur s'eclaircit", &a, &b, &alloc::vec![tache]);
}

/// Le meme changement, mais loin du curseur : rien ne doit etre ajoute.
#[test]
fn un_fond_qui_change_loin_du_curseur_ne_touche_pas_la_fleche() {
    let mut a = bureau_curseur_sur_tache_claire();
    a = a.avec_souris(300, 200);
    let mut b = a.clone();
    b.fenetres[0].couleur_tache = 0x08_08_08;
    let tache = b.fenetres[0].tache_ecran();
    assert!(
        tache
            .intersecte(&disposition::curseur(a.souris.0, a.souris.1))
            .vide()
    );
    assert!(
        transition::recoloration_curseur(&[tache], a.souris).est_vide(),
        "aucune recoloration ne doit etre demandee"
    );
    oracle("fond loin du curseur", &a, &b, &alloc::vec![tache]);
}

// ─── Barre du haut ─────────────────────────────────────────────────────────

/// LA transition qui etait cassee : le tic d'horloge.
#[test]
fn le_tic_d_horloge() {
    let a = bureau_deux_fenetres();
    let mut b = a.clone();
    b.horloge = a.horloge + 1;
    let degats = liste(transition::tic_horloge(L as u32));
    oracle("tic d'horloge", &a, &b, &degats);
}

#[test]
fn les_statistiques_changent() {
    let a = bureau_deux_fenetres();
    let mut b = a.clone();
    b.stats = a.stats + 7;
    let degats = liste(transition::tic_horloge(L as u32));
    oracle("CPU / RAM changent", &a, &b, &degats);
}

#[test]
fn l_horloge_et_les_statistiques_changent_ensemble() {
    let a = bureau_deux_fenetres();
    let mut b = a.clone();
    b.horloge = a.horloge + 1;
    b.stats = a.stats + 3;
    let degats = liste(transition::tic_horloge(L as u32));
    oracle("horloge + statistiques", &a, &b, &degats);
}

/// Une seconde passe pendant que le menu est ouvert : le menu recouvre-t-il
/// la barre du haut ? Non — mais le culling doit quand meme repeindre.
#[test]
fn le_tic_d_horloge_avec_le_menu_ouvert() {
    let a = menu_ouvert_sur(3);
    let mut b = a.clone();
    b.horloge = a.horloge + 1;
    let degats = liste(transition::tic_horloge(L as u32));
    oracle("tic d'horloge, menu ouvert", &a, &b, &degats);
}

/// Une fenetre maximisee touche la barre du haut par le haut de son cadre.
#[test]
fn le_tic_d_horloge_sous_une_fenetre_maximisee() {
    let mut a = bureau_vide();
    a.fenetres.push(Fen::neuve(
        0,
        HAUTEUR_BARRE as i32,
        L as i32,
        H as i32 - 2 * HAUTEUR_BARRE as i32,
        0x00_10_00,
    ));
    let mut b = a.clone();
    b.horloge = a.horloge + 1;
    let degats = liste(transition::tic_horloge(L as u32));
    oracle("tic d'horloge sous une fenetre maximisee", &a, &b, &degats);
}

// ─── Le rasteriseur lui-meme ───────────────────────────────────────────────
//
// Un oracle dont le rasteriseur ment est vert pour de mauvaises raisons.

#[test]
fn aucun_calque_ne_peint_hors_de_ses_bornes() {
    let etat = menu_ouvert_sur(2);
    for calque in plan(&etat).iter() {
        let mut seul = Toile::neuve();
        seul.clip = ecran();
        for pixel in seul.pixels.iter_mut() {
            *pixel = 0xdead_beef;
        }
        peins_calque(&mut seul, calque, &etat);
        for y in 0..H {
            for x in 0..L {
                let dans = x as i32 >= calque.bornes_dessin.x
                    && (x as i64) < calque.bornes_dessin.droite()
                    && y as i32 >= calque.bornes_dessin.y
                    && (y as i64) < calque.bornes_dessin.bas();
                if !dans {
                    assert_eq!(
                        seul.pixels[y * L + x], 0xdead_beef,
                        "{:?} peint hors de ses bornes en ({x}, {y})",
                        calque.element
                    );
                }
            }
        }
    }
}

#[test]
fn tout_calque_opaque_remplit_sa_zone_opaque() {
    let etat = menu_ouvert_sur(2);
    for calque in plan(&etat).iter() {
        let Some(opaque) = calque.opaque_sur else { continue };
        let mut seul = Toile::neuve();
        seul.clip = ecran();
        for pixel in seul.pixels.iter_mut() {
            *pixel = 0xdead_beef;
        }
        peins_calque(&mut seul, calque, &etat);
        for y in opaque.y..(opaque.bas() as i32) {
            for x in opaque.x..(opaque.droite() as i32) {
                assert_ne!(
                    seul.pixels[y as usize * L + x as usize], 0xdead_beef,
                    "{:?} laisse ({x}, {y}) intact dans sa zone opaque",
                    calque.element
                );
            }
        }
    }
}

/// Un rendu partiel sur l'ecran entier doit rendre exactement le rendu complet.
/// Si ce n'est pas le cas, c'est le culling qui est faux, pas les degats.
#[test]
fn le_rendu_partiel_plein_ecran_egale_le_rendu_complet() {
    for etat in [
        bureau_vide(),
        bureau_une_fenetre(),
        bureau_deux_fenetres(),
        menu_ouvert_sur(0),
        menu_ouvert_sur(6),
    ] {
        let mut test = Toile::neuve();
        rendu_partiel(&mut test, &etat, &tout());
        let reference = rendu_complet(&etat);
        assert!(
            test.premiere_difference(&reference).is_none(),
            "rendu partiel plein ecran != rendu complet"
        );
    }
}

/// Deux etats differents doivent produire deux images differentes, sinon
/// l'oracle serait vert parce qu'il ne voit rien.
#[test]
fn chaque_etat_teste_change_reellement_des_pixels() {
    let base = bureau_deux_fenetres();

    let mut horloge = base.clone();
    horloge.horloge += 1;
    assert!(
        rendu_complet(&base).premiere_difference(&rendu_complet(&horloge)).is_some(),
        "le tic d'horloge doit changer des pixels"
    );

    let mut stats = base.clone();
    stats.stats += 1;
    assert!(
        rendu_complet(&base).premiere_difference(&rendu_complet(&stats)).is_some(),
        "les statistiques doivent changer des pixels"
    );

    let survol_a = menu_ouvert_sur(1);
    let survol_b = menu_ouvert_sur(4);
    assert!(
        rendu_complet(&survol_a)
            .premiere_difference(&rendu_complet(&survol_b))
            .is_some(),
        "le survol doit changer des pixels"
    );

    let mut focus = base.clone();
    let remontee = focus.fenetres.remove(0);
    focus.fenetres.push(remontee);
    assert!(
        rendu_complet(&base).premiere_difference(&rendu_complet(&focus)).is_some(),
        "le focus doit changer des pixels"
    );
}
