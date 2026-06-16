//! Bureau graphique (mode VGA 13h) : fond, barre des taches avec lanceur,
//! horloge RTC, fenetre d'infos deplacable, curseur, et un **terminal
//! interactif** qui reutilise tout le shell.
//!
//! Lance par la commande `desktop`. Echap quitte (retour au shell texte).
//! Tout est gate ici : un probleme graphique n'affecte pas l'OS texte.

use crate::drivers::gfx::{
    self, C_BLACK, C_BLUE, C_CYAN, C_DESKTOP, C_DKGRAY, C_GRAY, C_GREEN, C_TITLE, C_WHITE,
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

const WW: i32 = 200;
const WH: i32 = 110;
const BAR_H: usize = 11;

struct Rect { x: i32, y: i32, w: i32, h: i32 }
impl Rect {
    fn hit(&self, mx: i32, my: i32) -> bool {
        mx >= self.x && mx < self.x + self.w && my >= self.y && my < self.y + self.h
    }
}

/// Applications de la barre des taches.
const APPS: [&str; 4] = ["Terminal", "Fichiers", "Moniteur", "Quitter"];

fn app_btn(i: usize) -> Rect {
    Rect { x: 2 + i as i32 * 78, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 76, h: 9 }
}

/// Lance le bureau graphique (bloquant jusqu'a Echap / bouton Quitter).
pub fn run() {
    gfx::enter();
    mouse::init();
    crate::serial_println!("[gui] bureau demarre");

    let mut cwd = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut wx: i32 = 55;
    let mut wy: i32 = 35;
    let mut dragging = false;
    let mut offx = 0i32;
    let mut offy = 0i32;
    let mut prev_left = false;

    loop {
        // Echap : on draine toute la file pour une reponse immediate.
        let mut quit = false;
        while let Some(sc) = keyboard::try_scancode() {
            if sc == 0x01 { quit = true; }
        }
        if quit { break; }

        let (mxu, myu) = mouse::pos();
        let mx = mxu as i32;
        let my = myu as i32;
        let left = mouse::left_down();
        let click = left && !prev_left;

        if click {
            let mut handled = false;
            for i in 0..APPS.len() {
                if app_btn(i).hit(mx, my) {
                    match i {
                        0 => { terminal(&mut cwd); }
                        1 => { files(cwd); }
                        2 => { monitor(); }
                        _ => { gfx::leave(); crate::serial_println!("[gui] bureau ferme"); return; }
                    }
                    prev_left = false;
                    handled = true;
                    break;
                }
            }
            if handled { continue; }
        }

        // Deplacement de la fenetre par sa barre de titre.
        if left {
            if !dragging && my >= wy && my < wy + BAR_H as i32 && mx >= wx && mx < wx + WW {
                dragging = true;
                offx = mx - wx;
                offy = my - wy;
            }
            if dragging { wx = mx - offx; wy = my - offy; }
        } else {
            dragging = false;
        }
        if wx < 0 { wx = 0; }
        if wy < BAR_H as i32 { wy = BAR_H as i32; }
        if wx + WW > WIDTH as i32 { wx = WIDTH as i32 - WW; }
        if wy + WH > HEIGHT as i32 - BAR_H as i32 { wy = HEIGHT as i32 - BAR_H as i32 - WH; }

        prev_left = left;
        draw_desktop(wx as usize, wy as usize);
        draw_cursor(mxu, myu);
        gfx::present();
    }

    gfx::leave();
    crate::serial_println!("[gui] bureau ferme");
}

fn draw_desktop(wx: usize, wy: usize) {
    gfx::clear(C_DESKTOP);

    // Barre du haut + horloge.
    gfx::fill_rect(0, 0, WIDTH, BAR_H, C_TITLE);
    gfx::draw_text(2, 2, "Bouchaud OS", C_WHITE);
    let dt = rtc::now();
    let clk = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    gfx::draw_text(WIDTH - clk.len() * 8 - 2, 2, &clk, C_YELLOW);

    // Fenetre "Systeme".
    window(wx, wy, "Systeme");
    let tx = wx + 4;
    let mut ty = wy + BAR_H + 3;
    gfx::draw_text(tx, ty, &format!("Version : {}", crate::VERSION), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Session : {}", users::session().username()), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Date    : {:04}-{:02}-{:02}", dt.year, dt.month, dt.day), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Uptime  : {} s", timer::seconds()), C_WHITE); ty += 12;
    gfx::draw_text(tx, ty, "OS souverain francais", C_YELLOW);

    // Barre des taches : lanceur d'applications.
    gfx::fill_rect(0, HEIGHT - BAR_H, WIDTH, BAR_H, C_TITLE);
    for (i, label) in APPS.iter().enumerate() {
        let b = app_btn(i);
        gfx::fill_rect(b.x as usize, b.y as usize, b.w as usize, b.h as usize, C_BLUE);
        gfx::draw_text(b.x as usize + 4, b.y as usize + 1, label, C_WHITE);
    }
}

fn window(x: usize, y: usize, title: &str) {
    let w = WW as usize;
    let h = WH as usize;
    gfx::fill_rect(x + 3, y + 3, w, h, C_DKGRAY);
    gfx::fill_rect(x, y, w, h, C_GRAY);
    gfx::rect(x, y, w, h, C_WHITE);
    gfx::fill_rect(x, y, w, BAR_H, C_BLUE);
    gfx::draw_text(x + 3, y + 2, title, C_WHITE);
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

// ---------------------------------------------------------------------------
// App "Fichiers" : navigateur de fichiers a la souris
// ---------------------------------------------------------------------------

const ROW_TOP: usize = 12;
const ROW_H: usize = 9;
const LIST_W: usize = 150;

fn files(start: usize) {
    let mut cur = start;
    let mut prev_left = false;
    let mut view: Option<(usize, Vec<String>)> = None; // (idx, lignes)

    loop {
        while let Some(sc) = keyboard::try_scancode() {
            if sc == 0x01 { return; }
        }
        let (mxu, myu) = mouse::pos();
        let left = mouse::left_down();
        let click = left && !prev_left;
        prev_left = left;

        // Construit la liste : ".." (si pas racine) puis les enfants.
        let fs = ramfs::fs();
        let mut entries: Vec<usize> = Vec::new(); // usize::MAX = ".."
        if cur != 0 { entries.push(usize::MAX); }
        for i in 0..ramfs::MAX_NODES {
            if fs.nodes[i].used && i != cur && fs.nodes[i].parent == cur {
                entries.push(i);
            }
        }

        // Clic dans la liste -> action.
        if click && (mxu as usize) < LIST_W && myu >= ROW_TOP {
            let row = (myu - ROW_TOP) / ROW_H;
            if row < entries.len() {
                let e = entries[row];
                if e == usize::MAX {
                    cur = fs.nodes[cur].parent;
                    view = None;
                } else if fs.nodes[e].kind == ramfs::NodeKind::Dir {
                    if fs.can(e, ramfs::PERM_X) { cur = e; view = None; }
                } else if fs.can(e, ramfs::PERM_R) {
                    let mut lines: Vec<String> = Vec::new();
                    let mut s = String::new();
                    for k in 0..fs.nodes[e].content_len { s.push(fs.nodes[e].content[k] as char); }
                    for l in s.split('\n') { lines.push(l.to_string()); }
                    view = Some((e, lines));
                }
            }
        }

        // Rendu.
        gfx::clear(C_BLACK);
        gfx::fill_rect(0, 0, WIDTH, 9, C_TITLE);
        let path = ramfs::path_string(ramfs::fs(), cur);
        gfx::draw_text(2, 1, &format!("Fichiers  {}", clip(&path, 30)), C_WHITE);

        let fs = ramfs::fs();
        let mut y = ROW_TOP;
        for &e in entries.iter() {
            if y + ROW_H > HEIGHT - 2 { break; }
            if e == usize::MAX {
                gfx::draw_text(2, y, "..", C_YELLOW);
            } else if fs.nodes[e].kind == ramfs::NodeKind::Dir {
                gfx::draw_text(2, y, &format!("{}/", clip(fs.nodes[e].name_str(), 16)), C_CYAN);
            } else {
                gfx::draw_text(2, y, clip(fs.nodes[e].name_str(), 17), C_WHITE);
            }
            y += ROW_H;
        }

        // Panneau de droite : contenu du fichier selectionne.
        gfx::fill_rect(LIST_W, 10, 1, HEIGHT - 12, C_GRAY);
        if let Some((idx, lines)) = &view {
            gfx::draw_text(LIST_W + 4, 11, clip(fs.nodes[*idx].name_str(), 20), C_GREEN);
            let mut vy = 22;
            for l in lines.iter() {
                if vy + 8 > HEIGHT - 2 { break; }
                gfx::draw_text(LIST_W + 4, vy, clip(l, 20), C_WHITE);
                vy += 8;
            }
        } else {
            gfx::draw_text(LIST_W + 4, 12, "Clic sur un", C_GRAY);
            gfx::draw_text(LIST_W + 4, 22, "fichier pour", C_GRAY);
            gfx::draw_text(LIST_W + 4, 32, "l'afficher", C_GRAY);
        }
        gfx::draw_text(2, HEIGHT - 9, "Echap=fermer  clic=ouvrir", C_YELLOW);
        draw_cursor(mxu, myu);
        gfx::present();
    }
}

// ---------------------------------------------------------------------------
// App "Moniteur" : informations systeme en direct
// ---------------------------------------------------------------------------

fn monitor() {
    loop {
        while let Some(sc) = keyboard::try_scancode() {
            if sc == 0x01 { return; }
        }
        let (mxu, myu) = mouse::pos();

        gfx::clear(C_BLACK);
        gfx::fill_rect(0, 0, WIDTH, 9, C_TITLE);
        gfx::draw_text(2, 1, "Moniteur systeme", C_WHITE);

        let dt = rtc::now();
        let (used, free, total) = crate::kernel::heap::stats();
        let vendor = crate::arch::x86_64::cpu::vendor();
        let mut vs = String::new();
        for b in vendor.iter() { vs.push(*b as char); }
        let fs = ramfs::fs();

        let mut y = 14;
        let line = |s: &str, col: u8, yy: &mut usize| { gfx::draw_text(4, *yy, s, col); *yy += 10; };
        line(&format!("OS      : Bouchaud OS {}", crate::VERSION), C_YELLOW, &mut y);
        line(&format!("Heure   : {:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second), C_GREEN, &mut y);
        line(&format!("Date    : {:04}-{:02}-{:02}", dt.year, dt.month, dt.day), C_WHITE, &mut y);
        line(&format!("Uptime  : {} s", timer::seconds()), C_WHITE, &mut y);
        line(&format!("CPU     : {}", vs), C_WHITE, &mut y);
        line(&format!("Heap    : {}/{} o libres {}", used, total, free), C_WHITE, &mut y);
        line(&format!("RAMFS   : {} inodes utilises", fs.used_nodes()), C_WHITE, &mut y);
        line(&format!("PCI     : {} peripheriques", crate::arch::x86_64::pci::count()), C_WHITE, &mut y);
        line(&format!("Session : {}", users::session().username()), C_CYAN, &mut y);

        gfx::draw_text(2, HEIGHT - 9, "Echap=fermer (mise a jour en direct)", C_YELLOW);
        draw_cursor(mxu, myu);
        gfx::present();
    }
}

// ---------------------------------------------------------------------------
// Terminal graphique interactif (REPL reutilisant le shell)
// ---------------------------------------------------------------------------

const TERM_COLS: usize = WIDTH / 8;       // 40
const TERM_TOP: usize = 10;
const TERM_ROWS: usize = (HEIGHT - TERM_TOP) / 8; // lignes affichables

fn term_prompt(cwd: usize) -> String {
    format!("{}:{}$ ", users::session().username(), ramfs::path_string(ramfs::fs(), cwd))
}

fn clip(s: &str, n: usize) -> &str {
    if s.len() > n { &s[..n] } else { s }
}

/// Boucle du terminal : lit une commande, l'execute via le shell, affiche.
fn terminal(cwd: &mut usize) {
    let mut sb: Vec<String> = Vec::new();
    sb.push("Terminal Bouchaud OS  -  'exit' ou Echap pour revenir".to_string());
    sb.push("".to_string());
    let mut input = String::new();

    loop {
        render_term(&sb, &input, *cwd);
        gfx::present();

        match keyboard::read_key() {
            Key::Other => return, // Echap
            Key::Enter => {
                let prompt = term_prompt(*cwd);
                sb.push(format!("{}{}", prompt, input));
                let cmd = input.trim().to_string();
                input.clear();
                if cmd.is_empty() { continue; }
                if cmd == "exit" || cmd == "logout" { return; }
                if cmd == "clear" { sb.clear(); continue; }
                // Evite de relancer le bureau (ou un editeur texte) dans le GUI.
                if cmd.starts_with("desktop") || cmd.starts_with("gui")
                    || cmd.starts_with("edit") || cmd.starts_with("nano") {
                    sb.push(format!("{}: a utiliser depuis le shell texte", cmd));
                    continue;
                }
                // Reutilise tout le pipeline du shell (chainage, pipes, $VAR...).
                let out = crate::shell::run_capture(&cmd, cwd);
                for line in out.lines() { sb.push(line.to_string()); }
                while sb.len() > 300 { sb.remove(0); }
            }
            Key::Backspace => { input.pop(); }
            Key::Char(c) => {
                if input.len() < TERM_COLS * 2 { input.push(c as char); }
            }
            _ => {}
        }
    }
}

fn render_term(sb: &[String], input: &str, cwd: usize) {
    gfx::clear(C_BLACK);
    gfx::fill_rect(0, 0, WIDTH, 9, C_TITLE);
    gfx::draw_text(2, 1, "Terminal Bouchaud OS", C_WHITE);

    // Derniere ligne = invite + saisie ; au-dessus = historique.
    let shown = TERM_ROWS - 1;
    let start = if sb.len() > shown { sb.len() - shown } else { 0 };
    let mut y = TERM_TOP;
    for line in &sb[start..] {
        gfx::draw_text(2, y, clip(line, TERM_COLS - 1), C_GREEN);
        y += 8;
    }
    let prompt = term_prompt(cwd);
    let cur = format!("{}{}_", prompt, input);
    gfx::draw_text(2, y, clip(&cur, TERM_COLS - 1), C_WHITE);
}
