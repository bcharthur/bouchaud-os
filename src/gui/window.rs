//! Fenetres et types partages du gestionnaire de fenetres.

use crate::gui::framebuffer::{HEIGHT, WIDTH};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub(crate) const BAR_H: usize = 11;        // hauteur des barres haut/bas
pub(crate) const TITLE_H: i32 = 10;        // hauteur barre de titre fenetre
pub(crate) const MIN_W: i32 = 90;
pub(crate) const MIN_H: i32 = 50;
pub(crate) const MENU_ITEM_H: i32 = 22;    // hauteur d'un item du menu Démarrer
pub(crate) const MENU_HEADER_H: i32 = 8;   // zone vide en haut du menu
pub(crate) const MENU_W: i32 = 178;        // largeur du menu Démarrer

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
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub min: bool,
    pub restore: Option<(i32, i32, i32, i32)>, // rect avant maximisation
    pub app: App,
}

/// Mode de manipulation de la fenetre du dessus a la souris.
#[derive(Clone, Copy)]
pub(crate) enum Drag {
    Move(i32, i32),
    Resize,
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
    let x = w.x + 1;
    let y = w.y + TITLE_H + 1;
    let largeur = (w.w - 2).max(0) as u32;
    let hauteur = (w.h - TITLE_H - 2).max(0) as u32;
    crate::gui::protocole::Rect::neuf(x, y, largeur, hauteur)
}

/// Geometrie d'une fenetre dont la zone utile doit faire `largeur` x `hauteur`.
pub(crate) fn fenetre_pour_zone(largeur: i32, hauteur: i32) -> (i32, i32) {
    (largeur + 2, hauteur + TITLE_H + 2)
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
    match w.restore.take() {
        Some((x, y, ww, hh)) => { w.x = x; w.y = y; w.w = ww; w.h = hh; }
        None => {
            w.restore = Some((w.x, w.y, w.w, w.h));
            w.x = 0;
            w.y = BAR_H as i32;
            w.w = WIDTH as i32;
            w.h = HEIGHT as i32 - 2 * BAR_H as i32;
        }
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
                return Win {
                    title: TITRE_NAVIGATEUR.to_string(),
                    x,
                    y,
                    w,
                    h,
                    min: false,
                    restore: None,
                    app: App::Navigateur { client: alloc::boxed::Box::new(client) },
                };
            }
            Err(error) => {
                crate::serial_println!("[gui] autostart navigateur impossible: {}", error);
            }
        }
    }

    match kind {
        0 => Win {
            title: "Terminal".to_string(), x, y, w: 380, h: 280, min: false, restore: None,
            app: App::Terminal { sb: { let mut v = Vec::new(); v.push("Bouchaud OS terminal".to_string()); v }, input: String::new(), cwd: home },
        },
        1 => Win {
            title: "Fichiers".to_string(), x, y, w: 420, h: 320, min: false, restore: None,
            app: App::Files { cur: home, scroll: 0, selected: None },
        },
        4 => Win {
            title: "Calculatrice".to_string(), x, y, w: 220, h: 300, min: false, restore: None,
            app: App::Calc { expr: String::new() },
        },
        5 => Win {
            title: "Rustpad — Hello World".to_string(), x, y, w: 560, h: 400, min: false, restore: None,
            app: App::Rustpad { state: crate::gui::apps::rustpad::RustpadState::new() },
        },
        _ => Win {
            title: "Moniteur".to_string(), x, y, w: 300, h: 200, min: false, restore: None,
            app: App::Monitor,
        },
    }
}
