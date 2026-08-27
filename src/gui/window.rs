//! Fenetres et types partages du gestionnaire de fenetres.

use crate::gui::framebuffer::{HEIGHT, WIDTH};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub(crate) const BAR_H: usize = 11;        // hauteur des barres haut/bas
pub(crate) const TITLE_H: i32 = crate::gui::windowing::TITLEBAR_HEIGHT as i32;
pub(crate) const MIN_W: i32 = 90;
pub(crate) const MIN_H: i32 = 50;
pub(crate) const MENU_ITEM_H: i32 = 22;    // hauteur d'un item du menu Démarrer
pub(crate) const MENU_HEADER_H: i32 = 8;   // zone vide en haut du menu
pub(crate) const MENU_W: i32 = 178;        // largeur du menu Démarrer

// Les constantes ci-dessus et celles de `gui::disposition` decrivent le MEME
// bureau. Elles sont declarees deux fois parce que l'une doit rester pure et
// testable sur l'hote ; ces assertions garantissent qu'elles ne peuvent pas
// diverger sans casser la compilation.
const _: () = {
    assert!(BAR_H as u32 == crate::gui::disposition::HAUTEUR_BARRE);
    assert!(MENU_ITEM_H == crate::gui::disposition::HAUTEUR_LIGNE_MENU);
    assert!(MENU_HEADER_H == crate::gui::disposition::ENTETE_MENU);
};

/// `kind` du navigateur.
///
/// Il fabrique desormais une fenetre comme les autres. Le programme vit toujours
/// en ring 3 (`tools/userland/navigateur/`), mais il ne prend plus l'ecran : il
/// peint dans une surface partagee que le gestionnaire de fenetres compose dans
/// sa fenetre. Voir `gui::client`.
pub(crate) const KIND_NAVIGATEUR: usize = 6;

/// Taille de la zone utile du navigateur, en pixels.
///
/// Elle est **fixe** pour ce jalon, et c'est assume : la surface partagee est
/// allouee une fois et Qt dimensionne son ecran dessus au demarrage. Redimensionner
/// la fenetre demanderait de reallouer la surface, de la reprojeter dans le client
/// et de faire admettre a `linuxfb` un changement de resolution a chaud — trois
/// chantiers qui n'ont rien a voir entre eux, et aucun qui doive retarder le
/// moment ou le navigateur devient une fenetre.
/// Nom du navigateur, tel qu'il apparait pour qui utilise la machine.
///
/// Il est ecrit **une fois**. Le bureau, le menu Demarrer, la barre de titre et
/// la barre des taches le lisent ici : un navigateur qui s'appelle « Ladybird »
/// sur son icone et « Bouchaud Browser » sur sa fenetre n'est pas un produit,
/// c'est deux moities de produit. C'est aussi le nom du moteur qu'on execute
/// reellement, ce qui evite de faire croire a un moteur maison.
pub(crate) const TITRE_NAVIGATEUR: &str = "Ladybird";

pub(crate) const NAV_LARGEUR: i32 = 1100;
pub(crate) const NAV_HAUTEUR: i32 = 604;

/// Entrees du menu Demarrer : (libelle, `kind` passe a `make_app`).
///
/// Le `kind` est explicite et non deduit de la position : retirer une entree ne
/// doit pas decaler silencieusement les autres vers la mauvaise application.
pub(crate) const MENU: [(&str, usize); 7] = [
    ("Ladybird", KIND_NAVIGATEUR),
    ("Terminal", 0), ("Fichiers", 1), ("Moniteur", 3),
    ("Calculatrice", 4), ("Rustpad", 5), ("Quitter", usize::MAX),
];

/// Icones du bureau : (libelle, kind). Cliquables pour lancer l'application.
pub(crate) const ICONS: [(&str, usize); 5] = [
    ("Ladybird", KIND_NAVIGATEUR),
    ("Calculatrice", 4), ("Terminal", 0), ("Fichiers", 1), ("Rustpad", 5),
];

/// Positions des icones de bureau (x, y). Modifiables par drag-and-drop.
pub(crate) static mut ICON_POSITIONS: [(i32, i32); 5] = [
    (10, 25), (10, 91), (10, 157), (10, 223), (10, 289),
];

/// Etat applicatif porte par une fenetre.
pub(crate) enum App {
    Terminal { sb: Vec<String>, input: String, cwd: usize },
    Files { cur: usize, scroll: i32, selected: Option<usize> },
    Calc { expr: String },
    Monitor,
    Rustpad { state: crate::gui::apps::rustpad::RustpadState },
    /// Fenetre d'un client ring 3 : le contenu vient de sa surface partagee.
    Navigateur { client: alloc::boxed::Box<crate::gui::client::Client> },
}

pub(crate) struct Win {
    pub window: crate::gui::windowing::Window,
    pub app: App,
}

impl Win {
    pub(crate) fn new(title: String, x: i32, y: i32, w: i32, h: i32,
        flags: crate::gui::windowing::WindowFlags, app: App) -> Self {
        let id = crate::gui::windowing::WindowId::allocate();
        let rect = crate::gui::windowing::Rect::new(x, y, w.max(0) as u32, h.max(0) as u32);
        Self { window: crate::gui::windowing::Window::new(id, title, rect, flags), app }
    }
}

impl core::ops::Deref for Win {
    type Target = crate::gui::windowing::Window;
    fn deref(&self) -> &Self::Target { &self.window }
}
impl core::ops::DerefMut for Win {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.window }
}

/// Mode de manipulation de la fenetre du dessus a la souris.
#[derive(Clone, Copy)]
pub(crate) enum Drag {
    Move(i32, i32),
    Resize(crate::gui::windowing::ResizeEdge),
}

pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
impl Rect {
    pub fn hit(&self, mx: i32, my: i32) -> bool {
        mx >= self.x && mx < self.x + self.w && my >= self.y && my < self.y + self.h
    }
}

/// Tronque une chaine a `n` octets max en respectant les frontieres UTF-8.
pub(crate) fn clip(s: &str, n: usize) -> &str {
    if s.len() <= n { return s; }
    let mut end = 0;
    for (i, _) in s.char_indices() {
        if i >= n { break; }
        end = i;
    }
    &s[..end]
}

pub(crate) fn start_btn() -> Rect {
    Rect { x: 2, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 38, h: 9 }
}

pub(crate) fn menu_rect() -> Rect {
    let h = MENU.len() as i32 * MENU_ITEM_H + MENU_HEADER_H + 8;
    Rect { x: 2, y: HEIGHT as i32 - BAR_H as i32 - h, w: MENU_W, h }
}

/// Le menu, dans le systeme de coordonnees du compositeur.
pub(crate) fn menu_proto() -> crate::gui::protocole::Rect {
    let r = menu_rect();
    crate::gui::protocole::Rect::neuf(r.x, r.y, r.w.max(0) as u32, r.h.max(0) as u32)
}

// BOUCHAUD_GUI_HOVER_CONTRAT_V1
//
// LA definition du survol du menu, pour tout le systeme.
//
// `draw_menu` la calculait chez lui, et personne d'autre ne la connaissait. Le
// survol repeint pourtant TOUTE une ligne -- fond, bordure de selection,
// couleur et graisse du texte -- alors qu'un deplacement de souris n'invalide
// que les deux empreintes 14x22 du curseur. Passer d'une ligne a l'autre
// laissait donc l'ancienne surbrillance a l'ecran.
//
// Le peintre et l'invalidation appellent maintenant la meme fonction. Elles ne
// peuvent plus repondre differemment.

/// Ligne du menu sous le pointeur, ou `None` si le menu n'est pas survole.
pub(crate) fn ligne_menu_survolee(mx: i32, my: i32) -> Option<usize> {
    crate::gui::disposition::ligne_menu_survolee(menu_proto(), mx, my)
}

/// Rectangle repeint quand la ligne `index` prend ou perd le survol.
pub(crate) fn rect_ligne_menu(index: usize) -> crate::gui::protocole::Rect {
    crate::gui::disposition::rect_ligne_menu(menu_proto(), index)
}

pub(crate) fn taskbar_btn(i: usize) -> Rect {
    Rect { x: 44 + i as i32 * 56, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 54, h: 9 }
}

/// Rectangle de l'icone de bureau `i`. Position pilotee par ICON_POSITIONS (drag).
pub(crate) fn icon_rect(i: usize) -> Rect {
    let (x, y) = unsafe { ICON_POSITIONS[i] };
    Rect { x, y, w: 56, h: 60 }
}

/// Zone utile d'une fenetre : l'interieur des bordures, sous la barre de titre.
///
/// C'est le repere dans lequel un client exprime ses degats et recoit ses
/// evenements. Une seule definition, ici : deux calculs concurrents de cette
/// zone donneraient un decalage entre l'endroit ou l'on peint et celui ou l'on
/// clique — un pixel, invisible a la lecture, evident a l'usage.
pub(crate) fn zone_utile(w: &Win) -> crate::gui::protocole::Rect {
    let rect = crate::gui::windowing::client_rect(w.rect(), TITLE_H as u32);
    crate::gui::protocole::Rect::neuf(rect.x, rect.y, rect.width, rect.height)
}

/// Geometrie d'une fenetre dont la zone utile doit faire `largeur` x `hauteur`.
pub(crate) fn fenetre_pour_zone(largeur: i32, hauteur: i32) -> (i32, i32) {
    let rect = crate::gui::windowing::outer_rect_for_client_size(
        crate::gui::windowing::Point { x: 0, y: 0 }, largeur.max(0) as u32,
        hauteur.max(0) as u32, TITLE_H as u32);
    (rect.width as i32, rect.height as i32)
}

/// La fenetre porte-t-elle un client ring 3 ?
pub(crate) fn est_client(w: &Win) -> bool {
    matches!(w.app, App::Navigateur { .. })
}

/// Bascule maximiser / restaurer une fenetre.
pub(crate) fn toggle_max(w: &mut Win) {
    // Un client ring 3 a une surface de taille fixe : l'agrandir n'agrandirait
    // que le cadre, et le contenu flotterait dans un coin. Tant que la surface
    // n'est pas reallouable, ne rien faire est la seule reponse honnete.
    if est_client(w) {
        return;
    }
    if w.placement == crate::gui::windowing::WindowPlacement::Maximized {
        if let Some(rect) = w.restore_rect.take() { w.set_rect(rect); }
        w.placement = crate::gui::windowing::WindowPlacement::Normal;
    } else if w.flags.maximizable {
        if w.placement == crate::gui::windowing::WindowPlacement::Normal {
            w.restore_rect = Some(w.rect());
        }
        w.set_rect(crate::gui::windowing::Rect::new(0, BAR_H as i32,
            WIDTH as u32, (HEIGHT - 2 * BAR_H) as u32));
        w.placement = crate::gui::windowing::WindowPlacement::Maximized;
    }
}

pub(crate) fn clamp_win(w: &mut Win) {
    if w.x < 0 { w.x = 0; }
    if w.y < BAR_H as i32 { w.y = BAR_H as i32; }
    if w.x + w.w > WIDTH as i32 { w.x = WIDTH as i32 - w.w; }
    if w.y + w.h > HEIGHT as i32 - BAR_H as i32 { w.y = HEIGHT as i32 - BAR_H as i32 - w.h; }
}

/// Le scenario automatise M8 demande au bureau d'ouvrir directement le
/// navigateur. La variable vient du shell/autorun et ne change donc rien au
/// demarrage interactif normal. On la limite a la toute premiere fenetre : un
/// clic ulterieur sur « Terminal » doit rester un terminal, meme pendant le test.
fn autostart_browser_requested(first_window: bool) -> bool {
    first_window
        && crate::shell::exported()
            .iter()
            .any(|entry| entry == "BO_AUTOSTART_BROWSER=1")
}

/// Cree une fenetre d'application a partir d'un index de menu.
pub(crate) fn make_app(kind: usize, home: usize, spawn_n: &mut i32) -> Win {
    let n = *spawn_n;
    *spawn_n += 1;
    let x = 30 + (n % 6) * 22;
    let y = 30 + (n % 6) * 18;

    // Le WM cree historiquement un terminal comme premiere fenetre. Pour M8 on
    // reutilise exactement ce point de creation afin de tester un vrai client
    // graphique, avec sa Surface partagee et son canal GUI, sans simuler de clic
    // souris en CI. En cas d'echec on retombe sur le terminal pour garder un
    // bureau diagnostic visible.
    if kind == 0 && autostart_browser_requested(n == 0) {
        crate::kernel::perf::browser_click();
        match crate::gui::client::Client::lance(
            crate::gui::client::CHEMIN_NAVIGATEUR,
            home,
            NAV_LARGEUR as usize,
            NAV_HAUTEUR as usize,
        ) {
            Ok(client) => {
                let (w, h) = fenetre_pour_zone(NAV_LARGEUR, NAV_HAUTEUR);
                crate::serial_println!("[gui] BO_AUTOSTART_BROWSER=1 -> /bo-navigateur");
                return Win::new(TITRE_NAVIGATEUR.to_string(), x, y, w, h,
                    crate::gui::windowing::WindowFlags::FIXED_SURFACE,
                    App::Navigateur { client: alloc::boxed::Box::new(client) });
            }
            Err(error) => {
                crate::serial_println!("[gui] autostart navigateur impossible: {}", error);
            }
        }
    }

    match kind {
        0 => Win::new("Terminal".to_string(), x, y, 380, 280,
            crate::gui::windowing::WindowFlags::STANDARD,
            App::Terminal { sb: { let mut v = Vec::new(); v.push("Bouchaud OS terminal".to_string()); v }, input: String::new(), cwd: home }),
        1 => Win::new("Fichiers".to_string(), x, y, 420, 320,
            crate::gui::windowing::WindowFlags::STANDARD,
            App::Files { cur: home, scroll: 0, selected: None }),
        4 => Win::new("Calculatrice".to_string(), x, y, 220, 300,
            crate::gui::windowing::WindowFlags::STANDARD, App::Calc { expr: String::new() }),
        5 => Win::new("Rustpad — Hello World".to_string(), x, y, 560, 400,
            crate::gui::windowing::WindowFlags::STANDARD,
            App::Rustpad { state: crate::gui::apps::rustpad::RustpadState::new() }),
        _ => Win::new("Moniteur".to_string(), x, y, 300, 200,
            crate::gui::windowing::WindowFlags::STANDARD, App::Monitor),
    }
}
