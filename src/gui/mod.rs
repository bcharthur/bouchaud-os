//! Bureau graphique (mode VGA 13h) : fond, barre des taches avec lanceur,
//! horloge RTC, fenetre d'infos deplacable, curseur, et un **terminal
//! interactif** qui reutilise tout le shell.
//!
//! Lance par la commande `desktop`. Echap quitte (retour au shell texte).
//! Tout est gate ici : un probleme graphique n'affecte pas l'OS texte.

use crate::drivers::gfx::{
    self, C_BLACK, C_BLUE, C_DESKTOP, C_DKGRAY, C_GRAY, C_GREEN, C_TITLE, C_WHITE, C_YELLOW,
    HEIGHT, WIDTH,
};
use crate::drivers::keyboard::{self, Key};
use crate::drivers::mouse;
use crate::arch::x86_64::rtc;
use crate::fs::ramfs;
use crate::kernel::timer;
use crate::{fs::ramfs, shell, users};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const WW: i32 = 200;
const WH: i32 = 110;
const BAR_H: usize = 11;
const STATUS_H: usize = 11;
const MAX_LINES: usize = 96;

#[derive(Clone, Copy, PartialEq)]
enum App {
    Terminal,
    Systeme,
    Notes,
    Navigateur,
}

struct DesktopState {
    app: App,
    wx: i32,
    wy: i32,
    dragging: bool,
    offx: i32,
    offy: i32,
    prev_left: bool,
    cwd: usize,
    input: String,
    lines: Vec<String>,
}

struct Rect { x: i32, y: i32, w: i32, h: i32 }
impl Rect {
    fn hit(&self, mx: i32, my: i32) -> bool {
        mx >= self.x && mx < self.x + self.w && my >= self.y && my < self.y + self.h
    }
}

fn term_btn() -> Rect { Rect { x: 2, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 72, h: 9 } }
fn quit_btn() -> Rect { Rect { x: 78, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 64, h: 9 } }

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

        if click && term_btn().hit(mx, my) {
            terminal(&mut cwd);
            prev_left = false;
            continue;
        }
        if click && quit_btn().hit(mx, my) {
            break;
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
}

fn handle_mouse(state: &mut DesktopState) {
    let (mxu, myu) = mouse::pos();
    let mx = mxu as i32;
    let my = myu as i32;
    let left = mouse::left_down();

    if left && !state.prev_left && my < BAR_H as i32 {
        if mx < 76 {
            switch_app(state, App::Terminal);
        } else if mx < 146 {
            switch_app(state, App::Systeme);
        } else if mx < 208 {
            switch_app(state, App::Notes);
        } else if mx < 310 {
            switch_app(state, App::Navigateur);
        }
    }
    state.prev_left = left;

    let (ww, wh) = app_size(state.app);
    if left {
        if !state.dragging
            && my >= state.wy
            && my < state.wy + BAR_H as i32
            && mx >= state.wx
            && mx < state.wx + ww
        {
            state.dragging = true;
            state.offx = mx - state.wx;
            state.offy = my - state.wy;
        }
        if state.dragging {
            state.wx = mx - state.offx;
            state.wy = my - state.offy;
        }
    } else {
        state.dragging = false;
    }
    clamp_window(&mut state.wx, &mut state.wy, ww, wh);
}

fn switch_app(state: &mut DesktopState, app: App) {
    state.app = app;
    match app {
        App::Terminal => { state.wx = 14; state.wy = 28; }
        App::Systeme => { state.wx = 92; state.wy = 36; }
        App::Notes => { state.wx = 86; state.wy = 58; }
        App::Navigateur => { state.wx = 24; state.wy = 42; }
    }
}

fn close_gui() {
    gfx::leave();
    crate::serial_println!("[gui] bureau ferme");
}

fn draw_desktop(wx: usize, wy: usize) {
    gfx::clear(C_DESKTOP);
    draw_topbar(state.app);
    match state.app {
        App::Terminal => draw_terminal(state),
        App::Systeme => draw_system(state.wx as usize, state.wy as usize),
        App::Notes => draw_notes(state.wx as usize, state.wy as usize),
        App::Navigateur => draw_browser(state.wx as usize, state.wy as usize),
    }
    gfx::fill_rect(0, HEIGHT - STATUS_H, WIDTH, STATUS_H, C_TITLE);
    gfx::draw_text(2, HEIGHT - STATUS_H + 2, "Echap=quitter  clic titre=deplacer", C_WHITE);
}

fn draw_topbar(active: App) {
    gfx::fill_rect(0, 0, WIDTH, BAR_H, C_TITLE);
    launcher(2, "Terminal", active == App::Terminal);
    launcher(78, "Systeme", active == App::Systeme);
    launcher(150, "Notes", active == App::Notes);
    launcher(212, "Web", active == App::Navigateur);
    let dt = rtc::now();
    let clk = format!("{:02}:{:02}", dt.hour, dt.minute);
    gfx::draw_text(WIDTH - clk.len() * 8 - 2, 2, &clk, C_YELLOW);
}

fn launcher(x: usize, label: &str, active: bool) {
    gfx::fill_rect(x, 1, label.len() * 8 + 6, 9, if active { C_BLUE } else { C_DKGRAY });
    gfx::draw_text(x + 3, 2, label, C_WHITE);
}

fn draw_terminal(state: &DesktopState) {
    let (w, h) = app_size(App::Terminal);
    let wx = state.wx as usize;
    let wy = state.wy as usize;
    window(wx, wy, w as usize, h as usize, "Terminal");
    gfx::fill_rect(wx + 3, wy + BAR_H + 2, w as usize - 6, h as usize - BAR_H - 5, C_BLACK);

    let max = ((h as usize - BAR_H - 8) / 9).min(state.lines.len());
    let start = state.lines.len().saturating_sub(max);
    let mut y = wy + BAR_H + 5;
    for l in &state.lines[start..] {
        draw_clipped_text(wx + 6, y, l, 35, if l.starts_with('$') { C_GREEN } else { C_CYAN });
        y += 9;
    }
}

fn draw_system(wx: usize, wy: usize) {
    let (w, h) = app_size(App::Systeme);
    window(wx, wy, w as usize, h as usize, "Systeme");
    let tx = wx + 5;
    let mut ty = wy + BAR_H + 4;
    let dt = rtc::now();
    gfx::draw_text(tx, ty, &format!("Version {}", crate::VERSION), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("User {}", users::session().username()), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Date {:04}-{:02}-{:02}", dt.year, dt.month, dt.day), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Uptime {} s", timer::seconds()), C_WHITE); ty += 12;
    gfx::draw_text(tx, ty, "Souverain FR", C_YELLOW);
}

fn draw_notes(wx: usize, wy: usize) {
    let (w, h) = app_size(App::Notes);
    window(wx, wy, w as usize, h as usize, "Apps maison");
    gfx::draw_text(wx + 5, wy + BAR_H + 5, "Terminal natif OK", C_YELLOW);
    gfx::draw_text(wx + 5, wy + BAR_H + 17, "Systeme + Notes OK", C_WHITE);
    gfx::draw_text(wx + 5, wy + BAR_H + 29, "Web: apres e1000", C_WHITE);
    gfx::draw_text(wx + 5, wy + BAR_H + 41, "HiDPI: bootloader 0.11", C_WHITE);
}

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
    let tb = term_btn();
    gfx::fill_rect(tb.x as usize, tb.y as usize, tb.w as usize, tb.h as usize, C_BLUE);
    gfx::draw_text(tb.x as usize + 3, tb.y as usize + 1, "Terminal", C_WHITE);
    let qb = quit_btn();
    gfx::fill_rect(qb.x as usize, qb.y as usize, qb.w as usize, qb.h as usize, C_BLUE);
    gfx::draw_text(qb.x as usize + 3, qb.y as usize + 1, "Quitter", C_WHITE);
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
