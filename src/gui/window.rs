//! Fenetres et types partages du gestionnaire de fenetres.

use crate::browser::loader;
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

/// Entrees du menu Demarrer (l'index = `kind` passe a `make_app`).
pub(crate) const MENU: [&str; 7] = ["Terminal", "Fichiers", "Nautile", "Moniteur", "Calculatrice", "Rustpad", "Quitter"];

/// Icones du bureau : (libelle, kind). Cliquables pour lancer l'application.
pub(crate) const ICONS: [(&str, usize); 5] = [
    ("Nautile", 2), ("Calculatrice", 4), ("Terminal", 0), ("Fichiers", 1), ("Rustpad", 5),
];

/// Positions des icones de bureau (x, y). Modifiables par drag-and-drop.
pub(crate) static mut ICON_POSITIONS: [(i32, i32); 5] = [
    (10, 25), (10, 91), (10, 157), (10, 223), (10, 289),
];

/// Etat applicatif porte par une fenetre.
pub(crate) enum App {
    Terminal { sb: Vec<String>, input: String, cwd: usize },
    Files { cur: usize, scroll: i32, selected: Option<usize> },
    Browser { state: crate::browser::BrowserState },
    Calc { expr: String },
    Monitor,
    Rustpad { state: crate::gui::apps::rustpad::RustpadState },
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

/// Bascule maximiser / restaurer une fenetre.
pub(crate) fn toggle_max(w: &mut Win) {
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

/// Cree une fenetre d'application a partir d'un index de menu.
pub(crate) fn make_app(kind: usize, home: usize, spawn_n: &mut i32) -> Win {
    let n = *spawn_n;
    *spawn_n += 1;
    let x = 30 + (n % 6) * 22;
    let y = 30 + (n % 6) * 18;
    match kind {
        0 => Win {
            title: "Terminal".to_string(), x, y, w: 380, h: 280, min: false, restore: None,
            app: App::Terminal { sb: { let mut v = Vec::new(); v.push("Bouchaud OS terminal".to_string()); v }, input: String::new(), cwd: home },
        },
        1 => Win {
            title: "Fichiers".to_string(), x, y, w: 420, h: 320, min: false, restore: None,
            app: App::Files { cur: home, scroll: 0, selected: None },
        },
        2 => {
            let url = "about:bouchaud".to_string();
            let bw = 560; let bh = 420;
            let (session, page) = loader::open(&url, bw - 6);
            let state = crate::browser::BrowserState::new(url, page, session);
            Win { title: "Nautile Navigateur".to_string(), x, y, w: bw, h: bh, min: false, restore: None,
                  app: App::Browser { state } }
        }
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
