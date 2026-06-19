//! Fenetres et types partages du gestionnaire de fenetres.

use crate::gui::apps::chromium_stub;
use crate::gui::framebuffer::{HEIGHT, WIDTH};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub(crate) const BAR_H: usize = 11; // hauteur des barres haut/bas
pub(crate) const TITLE_H: i32 = 10; // hauteur barre de titre fenetre
pub(crate) const MIN_W: i32 = 90;
pub(crate) const MIN_H: i32 = 50;

/// Entrees du menu Demarrer (l'index = `kind` passe a `make_app`).
pub(crate) const MENU: [&str; 6] = ["Terminal", "Fichiers", "Navigateur", "Moniteur", "Calculatrice", "Quitter"];

/// Icones du bureau : (libelle, kind). Cliquables pour lancer l'application.
pub(crate) const ICONS: [(&str, usize); 4] = [
    ("Navigateur", 2), ("Calculatrice", 4), ("Terminal", 0), ("Fichiers", 1),
];

/// Etat applicatif porte par une fenetre.
pub(crate) enum App {
    Terminal { sb: Vec<String>, input: String, cwd: usize },
    Files { cur: usize, view: Option<Vec<String>>, name: String },
    Browser { url: String, input: String, page: crate::gui::web::Page, scroll: i32, session: crate::gui::web::Session },
    Calc { expr: String },
    Monitor,
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

/// Tronque une chaine a `n` caracteres (ASCII) pour l'affichage.
pub(crate) fn clip(s: &str, n: usize) -> &str {
    if s.len() > n { &s[..n] } else { s }
}

pub(crate) fn start_btn() -> Rect {
    Rect { x: 2, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 38, h: 9 }
}

pub(crate) fn menu_rect() -> Rect {
    let h = MENU.len() as i32 * 10 + 2;
    Rect { x: 2, y: HEIGHT as i32 - BAR_H as i32 - h, w: 92, h }
}

pub(crate) fn taskbar_btn(i: usize) -> Rect {
    Rect { x: 44 + i as i32 * 56, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 54, h: 9 }
}

/// Rectangle de l'icone de bureau `i` (colonne verticale en haut a gauche).
/// Inclut l'etiquette sous la vignette (zone cliquable complete).
pub(crate) fn icon_rect(i: usize) -> Rect {
    Rect { x: 10, y: BAR_H as i32 + 14 + i as i32 * 66, w: 56, h: 60 }
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
            title: "Fichiers".to_string(), x, y, w: 320, h: 300, min: false, restore: None,
            app: App::Files { cur: home, view: None, name: String::new() },
        },
        2 => {
            let url = "about:bouchaud".to_string();
            let w = 560; let h = 420;
            let (session, page) = chromium_stub::open(&url, w - 6);
            Win { title: "Bouchaud Browser".to_string(), x, y, w, h, min: false, restore: None,
                  app: App::Browser { url: url.clone(), input: url, page, scroll: 0, session } }
        }
        4 => Win {
            title: "Calculatrice".to_string(), x, y, w: 220, h: 300, min: false, restore: None,
            app: App::Calc { expr: String::new() },
        },
        _ => Win {
            title: "Moniteur".to_string(), x, y, w: 300, h: 200, min: false, restore: None,
            app: App::Monitor,
        },
    }
}
