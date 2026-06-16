//! Gestionnaire de fenetres (Window Manager) de Bouchaud OS — etape Windows-like.
//!
//! Bureau graphique multi-fenetres en mode VGA 13h : fenetres deplacables et
//! fermables, focus / z-order, **menu Demarrer**, barre des taches, et des
//! applications natives (Terminal, Fichiers, Moniteur, Bouchaud Browser).
//!
//! Tout tourne dans une boucle d'evenements unique (entree souris/clavier non
//! bloquante) ; chaque fenetre porte l'etat de son application.

use crate::drivers::gfx::{
    self, C_BLACK, C_BLUE, C_CYAN, C_DESKTOP, C_DKGRAY, C_GRAY, C_GREEN, C_RED, C_TITLE, C_WHITE,
    C_YELLOW, HEIGHT, WIDTH,
};
use crate::drivers::keyboard::{self, Key};
use crate::drivers::mouse;
use crate::arch::x86_64::rtc;
use crate::fs::ramfs;
use crate::kernel::timer;
use crate::users;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const BAR_H: usize = 11; // hauteur des barres haut/bas
const TITLE_H: i32 = 10; // hauteur barre de titre fenetre

/// Etat applicatif porte par une fenetre.
enum App {
    Terminal { sb: Vec<String>, input: String, cwd: usize },
    Files { cur: usize, view: Option<Vec<String>>, name: String },
    Browser { url: String, input: String, content: Vec<String> },
    Monitor,
}

struct Win {
    title: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    app: App,
}

struct Rect { x: i32, y: i32, w: i32, h: i32 }
impl Rect {
    fn hit(&self, mx: i32, my: i32) -> bool {
        mx >= self.x && mx < self.x + self.w && my >= self.y && my < self.y + self.h
    }
}

/// Entrees du menu Demarrer.
const MENU: [&str; 5] = ["Terminal", "Fichiers", "Navigateur", "Moniteur", "Quitter"];

fn start_btn() -> Rect {
    Rect { x: 2, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 38, h: 9 }
}

fn menu_rect() -> Rect {
    let h = MENU.len() as i32 * 10 + 2;
    Rect { x: 2, y: HEIGHT as i32 - BAR_H as i32 - h, w: 92, h }
}

/// Lance le bureau (boucle d'evenements, bloquant jusqu'a Quitter).
pub fn run() {
    gfx::enter();
    mouse::init();
    crate::serial_println!("[gui] window manager demarre");

    let home = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut wins: Vec<Win> = Vec::new();
    let mut menu_open = false;
    let mut prev_left = false;
    let mut drag: Option<(i32, i32)> = None; // offset pour la fenetre du dessus
    let mut spawn_n = 0i32;

    // Fenetre d'accueil : le navigateur sur about:bouchaud.
    wins.push(make_app(2, home, &mut spawn_n));

    let mut quit = false;
    while !quit {
        // ---- Clavier (non bloquant) ----
        while let Some(k) = keyboard::try_key() {
            match k {
                Key::Other => {
                    if menu_open { menu_open = false; }
                    else if wins.pop().is_none() { quit = true; }
                }
                other => {
                    if let Some(w) = wins.last_mut() {
                        if key_to_app(w, other, home) {
                            wins.pop(); // l'app demande sa fermeture (exit)
                        }
                    }
                }
            }
        }

        // ---- Souris ----
        let (mxu, myu) = mouse::pos();
        let mx = mxu as i32;
        let my = myu as i32;
        let left = mouse::left_down();
        let click = left && !prev_left;
        prev_left = left;

        if left {
            if let Some((ox, oy)) = drag {
                if let Some(w) = wins.last_mut() {
                    w.x = mx - ox;
                    w.y = my - oy;
                    clamp_win(w);
                }
            }
        } else {
            drag = None;
        }

        if click {
            handle_click(mx, my, &mut wins, &mut menu_open, &mut drag, &mut quit, home, &mut spawn_n);
        }

        // ---- Rendu ----
        draw_desktop(&wins);
        if menu_open { draw_menu(); }
        draw_taskbar(&wins, menu_open);
        draw_cursor(mxu, myu);
        gfx::present();
    }

    gfx::leave();
    crate::serial_println!("[gui] window manager ferme");
}

fn handle_click(
    mx: i32, my: i32,
    wins: &mut Vec<Win>,
    menu_open: &mut bool,
    drag: &mut Option<(i32, i32)>,
    quit: &mut bool,
    home: usize,
    spawn_n: &mut i32,
) {
    // Menu Demarrer ouvert : clic sur un item ?
    if *menu_open {
        let mr = menu_rect();
        if mr.hit(mx, my) {
            let row = ((my - mr.y - 1) / 10) as usize;
            if row < MENU.len() {
                if row == MENU.len() - 1 { *quit = true; }
                else { wins.push(make_app(row, home, spawn_n)); }
            }
        }
        *menu_open = false;
        return;
    }
    // Bouton Demarrer.
    if start_btn().hit(mx, my) { *menu_open = true; return; }

    // Boutons de la barre des taches (focus fenetre).
    for i in 0..wins.len() {
        if taskbar_btn(i).hit(mx, my) {
            let w = wins.remove(i);
            wins.push(w);
            return;
        }
    }

    // Fenetres, du dessus vers le dessous.
    let mut hit: Option<usize> = None;
    for i in (0..wins.len()).rev() {
        let w = &wins[i];
        if mx >= w.x && mx < w.x + w.w && my >= w.y && my < w.y + w.h {
            hit = Some(i);
            break;
        }
    }
    if let Some(i) = hit {
        let w = wins.remove(i);
        wins.push(w);
        let top = wins.last_mut().unwrap();
        // Bouton fermer (coin haut droit).
        if mx >= top.x + top.w - 10 && mx < top.x + top.w - 1 && my >= top.y + 1 && my < top.y + TITLE_H {
            wins.pop();
        } else if my < top.y + TITLE_H {
            *drag = Some((mx - top.x, my - top.y)); // glisser par la barre de titre
        } else {
            app_click(top, mx, my, home);
        }
    }
}

fn clamp_win(w: &mut Win) {
    if w.x < 0 { w.x = 0; }
    if w.y < BAR_H as i32 { w.y = BAR_H as i32; }
    if w.x + w.w > WIDTH as i32 { w.x = WIDTH as i32 - w.w; }
    if w.y + w.h > HEIGHT as i32 - BAR_H as i32 { w.y = HEIGHT as i32 - BAR_H as i32 - w.h; }
}

/// Cree une fenetre d'application a partir d'un index de menu.
fn make_app(kind: usize, home: usize, spawn_n: &mut i32) -> Win {
    let n = *spawn_n;
    *spawn_n += 1;
    let x = 14 + (n % 5) * 10;
    let y = 16 + (n % 5) * 9;
    match kind {
        0 => Win {
            title: "Terminal".to_string(), x, y, w: 220, h: 150,
            app: App::Terminal { sb: { let mut v = Vec::new(); v.push("Bouchaud OS terminal".to_string()); v }, input: String::new(), cwd: home },
        },
        1 => Win {
            title: "Fichiers".to_string(), x, y, w: 200, h: 150,
            app: App::Files { cur: home, view: None, name: String::new() },
        },
        2 => {
            let url = "about:bouchaud".to_string();
            let content = load_page(&url);
            Win { title: "Bouchaud Browser".to_string(), x, y, w: 250, h: 160,
                  app: App::Browser { url: url.clone(), input: url, content } }
        }
        _ => Win {
            title: "Moniteur".to_string(), x, y, w: 200, h: 130,
            app: App::Monitor,
        },
    }
}

// ---------------------------------------------------------------------------
// Entree clavier -> application focalisee
// ---------------------------------------------------------------------------

/// Transmet une touche a l'app de la fenetre active. Renvoie true si l'app
/// demande sa fermeture.
fn key_to_app(w: &mut Win, k: Key, _home: usize) -> bool {
    match &mut w.app {
        App::Terminal { sb, input, cwd } => match k {
            Key::Enter => {
                let prompt = format!("{}:{}$ ", users::session().username(), ramfs::path_string(ramfs::fs(), *cwd));
                sb.push(format!("{}{}", prompt, input));
                let cmd = input.trim().to_string();
                input.clear();
                if cmd.is_empty() { return false; }
                if cmd == "exit" { return true; }
                if cmd == "clear" { sb.clear(); return false; }
                if is_blocked(&cmd) {
                    sb.push(format!("{}: a lancer depuis le shell texte", first_word(&cmd)));
                } else {
                    let out = crate::shell::run_capture(&cmd, cwd);
                    for l in out.lines() { sb.push(l.to_string()); }
                }
                while sb.len() > 300 { sb.remove(0); }
                false
            }
            Key::Backspace => { input.pop(); false }
            Key::Char(c) => { if input.len() < 120 { input.push(c as char); } false }
            _ => false,
        },
        App::Browser { url, input, content } => match k {
            Key::Enter => {
                *url = input.trim().to_string();
                *content = load_page(url);
                false
            }
            Key::Backspace => { input.pop(); false }
            Key::Char(c) => { if input.len() < 80 { input.push(c as char); } false }
            _ => false,
        },
        _ => false,
    }
}

fn first_word(s: &str) -> &str {
    s.split(' ').next().unwrap_or(s)
}

/// Commandes interactives a ne pas lancer dans le terminal graphique.
fn is_blocked(cmd: &str) -> bool {
    matches!(first_word(cmd),
        "edit" | "nano" | "desktop" | "gui" | "su" | "passwd" | "useradd" | "userdel" | "login")
}

// ---------------------------------------------------------------------------
// Clic dans le corps d'une application
// ---------------------------------------------------------------------------

fn app_click(w: &mut Win, _mx: i32, my: i32, _home: usize) {
    if let App::Files { cur, view, name } = &mut w.app {
        // En mode apercu : un clic revient a la liste.
        if view.is_some() { *view = None; return; }
        let by = w.y + TITLE_H + 2;
        let row = ((my - by) / 9).max(0) as usize;
        let fs = ramfs::fs();
        let mut entries: Vec<usize> = Vec::new();
        if *cur != 0 { entries.push(usize::MAX); }
        for i in 0..ramfs::MAX_NODES {
            if fs.nodes[i].used && i != *cur && fs.nodes[i].parent == *cur { entries.push(i); }
        }
        if row >= entries.len() { return; }
        let e = entries[row];
        if e == usize::MAX {
            *cur = fs.nodes[*cur].parent;
        } else if fs.nodes[e].kind == ramfs::NodeKind::Dir {
            if fs.can(e, ramfs::PERM_X) { *cur = e; }
        } else if fs.can(e, ramfs::PERM_R) {
            let mut lines = Vec::new();
            let mut s = String::new();
            for k in 0..fs.nodes[e].content_len { s.push(fs.nodes[e].content[k] as char); }
            for l in s.split('\n') { lines.push(l.to_string()); }
            *name = fs.nodes[e].name_str().to_string();
            *view = Some(lines);
        }
    }
}

// ---------------------------------------------------------------------------
// Bouchaud Browser : pages internes
// ---------------------------------------------------------------------------

fn load_page(url: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if url == "about:bouchaud" {
        out.push("Bouchaud OS".to_string());
        out.push("OS souverain francais experimental".to_string());
        out.push("".to_string());
        out.push(format!("Version : {}", crate::VERSION));
        out.push("Kernel  : Rust no_std".to_string());
        out.push("GUI     : window manager VGA".to_string());
        out.push("Reseau  : loopback (e1000 a venir)".to_string());
        out.push("".to_string());
        out.push("Pages: about:system  file:/readme.txt".to_string());
    } else if url == "about:system" {
        let dt = rtc::now();
        let (used, free, total) = crate::kernel::heap::stats();
        out.push(format!("Heure  : {:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second));
        out.push(format!("Uptime : {} s", timer::seconds()));
        out.push(format!("Heap   : {}/{} o (libre {})", used, total, free));
        out.push(format!("PCI    : {} peripheriques", crate::arch::x86_64::pci::count()));
        out.push(format!("User   : {}", users::session().username()));
    } else if let Some(path) = url.strip_prefix("file:") {
        // file:/chemin ou file:///chemin
        let p = path.trim_start_matches('/');
        let full = format!("/{}", p);
        let fs = ramfs::fs();
        match fs.resolve_checked(&full, 0) {
            Ok(idx) if fs.nodes[idx].kind == ramfs::NodeKind::File => {
                if fs.can(idx, ramfs::PERM_R) {
                    let mut s = String::new();
                    for k in 0..fs.nodes[idx].content_len { s.push(fs.nodes[idx].content[k] as char); }
                    for l in s.split('\n') { out.push(l.to_string()); }
                } else {
                    out.push("Erreur: permission denied".to_string());
                }
            }
            _ => out.push(format!("Erreur: introuvable {}", full)),
        }
    } else if url.starts_with("http://") || url.starts_with("https://") {
        out.push("Reseau non disponible.".to_string());
        out.push("Le driver e1000 (HTTP) arrive plus tard.".to_string());
    } else {
        out.push(format!("Page inconnue: {}", url));
        out.push("Essaie: about:bouchaud, about:system, file:/readme.txt".to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// Rendu
// ---------------------------------------------------------------------------

fn draw_desktop(wins: &[Win]) {
    gfx::clear(C_DESKTOP);
    gfx::fill_rect(0, 0, WIDTH, BAR_H, C_TITLE);
    gfx::draw_text(2, 2, "Bouchaud OS", C_WHITE);
    let dt = rtc::now();
    let clk = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    gfx::draw_text(WIDTH - clk.len() * 8 - 2, 2, &clk, C_YELLOW);

    let top = wins.len();
    for (i, w) in wins.iter().enumerate() {
        draw_window(w, i + 1 == top);
    }
}

fn draw_window(w: &Win, focused: bool) {
    let x = w.x.max(0) as usize;
    let y = w.y.max(0) as usize;
    let ww = w.w as usize;
    let wh = w.h as usize;
    gfx::fill_rect(x + 2, y + 2, ww, wh, C_DKGRAY); // ombre
    gfx::fill_rect(x, y, ww, wh, C_GRAY);
    gfx::rect(x, y, ww, wh, C_WHITE);
    gfx::fill_rect(x, y, ww, TITLE_H as usize, if focused { C_BLUE } else { C_DKGRAY });
    gfx::draw_text(x + 3, y + 1, clip(&w.title, (ww / 8).saturating_sub(3)), C_WHITE);
    // Bouton fermer.
    gfx::fill_rect(x + ww - 10, y + 1, 8, 8, C_RED);
    gfx::draw_text(x + ww - 9, y + 1, "x", C_WHITE);

    draw_app(w);
}

fn draw_app(w: &Win) {
    let bx = w.x.max(0) as usize + 3;
    let by = w.y.max(0) as usize + TITLE_H as usize + 2;
    let bw = w.w as usize - 6;
    let bh = w.h as usize - TITLE_H as usize - 4;
    let cols = bw / 8;
    let rows = bh / 8;

    match &w.app {
        App::Terminal { sb, input, cwd } => {
            let shown = rows.saturating_sub(1);
            let start = if sb.len() > shown { sb.len() - shown } else { 0 };
            let mut yy = by;
            for line in &sb[start..] {
                gfx::draw_text(bx, yy, clip(line, cols), C_GREEN);
                yy += 8;
            }
            let prompt = format!("{}:{}$ ", users::session().username(), ramfs::path_string(ramfs::fs(), *cwd));
            let cur = format!("{}{}_", prompt, input);
            gfx::draw_text(bx, yy, clip(&cur, cols), C_WHITE);
        }
        App::Files { cur, view, name } => {
            if let Some(lines) = view {
                gfx::draw_text(bx, by, clip(name, cols), C_YELLOW);
                let mut yy = by + 10;
                for l in lines {
                    if yy + 8 > by + bh { break; }
                    gfx::draw_text(bx, yy, clip(l, cols), C_WHITE);
                    yy += 8;
                }
            } else {
                let fs = ramfs::fs();
                let mut yy = by;
                if *cur != 0 { gfx::draw_text(bx, yy, "..", C_YELLOW); yy += 9; }
                for i in 0..ramfs::MAX_NODES {
                    if yy + 9 > by + bh { break; }
                    if fs.nodes[i].used && i != *cur && fs.nodes[i].parent == *cur {
                        if fs.nodes[i].kind == ramfs::NodeKind::Dir {
                            gfx::draw_text(bx, yy, &format!("{}/", clip(fs.nodes[i].name_str(), cols - 1)), C_CYAN);
                        } else {
                            gfx::draw_text(bx, yy, clip(fs.nodes[i].name_str(), cols), C_WHITE);
                        }
                        yy += 9;
                    }
                }
            }
        }
        App::Browser { url, input, content } => {
            // Barre d'adresse.
            gfx::fill_rect(bx, by, bw, 9, C_WHITE);
            let shown = if input == url { format!("{}", input) } else { format!("{}_", input) };
            gfx::draw_text(bx + 1, by + 1, clip(&shown, cols), C_BLACK);
            // Contenu.
            let mut yy = by + 12;
            for l in content {
                if yy + 8 > by + bh { break; }
                gfx::draw_text(bx, yy, clip(l, cols), C_WHITE);
                yy += 8;
            }
        }
        App::Monitor => {
            let dt = rtc::now();
            let (used, free, total) = crate::kernel::heap::stats();
            let fs = ramfs::fs();
            let mut yy = by;
            let mut put = |s: &str, c: u8| { gfx::draw_text(bx, yy, clip(s, cols), c); yy += 10; };
            put(&format!("Bouchaud OS {}", crate::VERSION), C_YELLOW);
            put(&format!("Heure  {:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second), C_GREEN);
            put(&format!("Uptime {} s", timer::seconds()), C_WHITE);
            put(&format!("Heap   {}/{} o", used, total), C_WHITE);
            put(&format!("Libre  {} o", free), C_WHITE);
            put(&format!("PCI    {} dev", crate::arch::x86_64::pci::count()), C_WHITE);
            put(&format!("RAMFS  {} inodes", fs.used_nodes()), C_WHITE);
        }
    }
}

fn taskbar_btn(i: usize) -> Rect {
    Rect { x: 44 + i as i32 * 56, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 54, h: 9 }
}

fn draw_taskbar(wins: &[Win], menu_open: bool) {
    gfx::fill_rect(0, HEIGHT - BAR_H, WIDTH, BAR_H, C_TITLE);
    let sb = start_btn();
    gfx::fill_rect(sb.x as usize, sb.y as usize, sb.w as usize, sb.h as usize, if menu_open { C_GREEN } else { C_BLUE });
    gfx::draw_text(sb.x as usize + 3, sb.y as usize + 1, "Start", C_WHITE);
    for (i, w) in wins.iter().enumerate() {
        let b = taskbar_btn(i);
        if b.x + b.w > WIDTH as i32 { break; }
        gfx::fill_rect(b.x as usize, b.y as usize, b.w as usize, b.h as usize, C_DKGRAY);
        gfx::draw_text(b.x as usize + 2, b.y as usize + 1, clip(&w.title, 6), C_WHITE);
    }
}

fn draw_menu() {
    let mr = menu_rect();
    gfx::fill_rect(mr.x as usize, mr.y as usize, mr.w as usize, mr.h as usize, C_GRAY);
    gfx::rect(mr.x as usize, mr.y as usize, mr.w as usize, mr.h as usize, C_WHITE);
    for (i, item) in MENU.iter().enumerate() {
        let iy = mr.y as usize + 1 + i * 10;
        let col = if i == MENU.len() - 1 { C_RED } else { C_BLUE };
        gfx::draw_text(mr.x as usize + 4, iy + 1, item, col);
    }
}

fn draw_cursor(mx: usize, my: usize) {
    const CUR: [u8; 8] = [
        0b00000001, 0b00000011, 0b00000111, 0b00001111,
        0b00011111, 0b00000111, 0b00001101, 0b00011000,
    ];
    for (row, bits) in CUR.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                gfx::pixel(mx + col, my + row, C_WHITE);
            }
        }
    }
}

fn clip(s: &str, n: usize) -> &str {
    if s.len() > n { &s[..n] } else { s }
}
