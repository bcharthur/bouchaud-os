//! Bureau graphique VGA mode 13h.
//!
//! V0.13.x : bureau simple avec barre des taches, bouton Terminal, bouton
//! Quitter et terminal graphique interactif. Le terminal graphique reutilise
//! le shell existant via `shell::run_capture`, donc il garde les commandes,
//! chainages `; && ||`, pipes `|`, redirections et variables `$VAR`.
//!
//! Important : `gfx::leave()` recharge la police texte VGA avant le retour au
//! shell texte. Cela evite les rayures verticales observees apres le mode 13h.

use crate::arch::x86_64::rtc;
use crate::drivers::gfx::{
    self, C_BLACK, C_BLUE, C_CYAN, C_DESKTOP, C_DKGRAY, C_GRAY, C_GREEN, C_TITLE,
    C_WHITE, C_YELLOW, HEIGHT, WIDTH,
};
use crate::drivers::{keyboard::{self, Key}, mouse};
use crate::kernel::timer;
use crate::{shell, users};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const BAR_H: usize = 11;
const WIN_W: usize = 200;
const WIN_H: usize = 110;
const MAX_SCROLLBACK: usize = 300;
const TERM_COLS: usize = WIDTH / 8;          // 40 colonnes en mode 13h
const TERM_TOP: usize = 10;
const TERM_ROWS: usize = (HEIGHT - TERM_TOP) / 8;

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn hit(&self, mx: i32, my: i32) -> bool {
        mx >= self.x && mx < self.x + self.w && my >= self.y && my < self.y + self.h
    }
}

fn term_btn() -> Rect {
    Rect { x: 2, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 72, h: 9 }
}

fn chromium_btn() -> Rect {
    Rect { x: 78, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 82, h: 9 }
}

fn quit_btn() -> Rect {
    Rect { x: 164, y: HEIGHT as i32 - BAR_H as i32 + 1, w: 64, h: 9 }
}

/// Lance le bureau graphique. Echap ou le bouton Quitter reviennent au shell
/// texte. Le retour appelle `gfx::leave()` pour restaurer le mode texte et la
/// police VGA.
pub fn run() {
    gfx::enter();
    mouse::init();
    crate::serial_println!("[gui] bureau demarre");

    let mut cwd = crate::fs::ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut wx: i32 = ((WIDTH - WIN_W) / 2) as i32;
    let mut wy: i32 = 42;
    let mut dragging = false;
    let mut offx = 0i32;
    let mut offy = 0i32;
    let mut prev_left = false;

    loop {
        // Echap instantane : on draine toute la file clavier a chaque frame.
        let mut quit = false;
        while let Some(key) = keyboard::try_key() {
            if key == Key::Escape {
                quit = true;
            }
        }
        if quit {
            break;
        }

        // ---- Souris ----
        let (mxu, myu) = mouse::pos();
        let mx = mxu as i32;
        let my = myu as i32;
        let left = mouse::left_down();
        let click = left && !prev_left;
        prev_left = left;

        if click && term_btn().hit(mx, my) {
            terminal(&mut cwd);
            // Evite de propager le relachement/clic au bureau.
            prev_left = false;
            continue;
        }
        if click && chromium_btn().hit(mx, my) {
            chromium_launcher();
            prev_left = false;
            continue;
        }
        if click && quit_btn().hit(mx, my) {
            break;
        }

        // Fenetre Systeme deplacable par la barre de titre.
        if left {
            if !dragging
                && mx >= wx
                && mx < wx + WIN_W as i32
                && my >= wy
                && my < wy + BAR_H as i32
            {
                dragging = true;
                offx = mx - wx;
                offy = my - wy;
            }
            if dragging {
                wx = mx - offx;
                wy = my - offy;
            }
        } else {
            dragging = false;
        }
        clamp_window(&mut wx, &mut wy, WIN_W as i32, WIN_H as i32);
        prev_left = left;

        draw_desktop(wx as usize, wy as usize);
        draw_cursor(mxu, myu);
        gfx::present();
    }

    close_gui();
}

fn close_gui() {
    gfx::leave();
    crate::drivers::vga::clear();
    crate::serial_println!("[gui] bureau ferme");
}

fn clamp_window(wx: &mut i32, wy: &mut i32, ww: i32, wh: i32) {
    if *wx < 0 { *wx = 0; }
    if *wy < BAR_H as i32 { *wy = BAR_H as i32; }
    if *wx + ww > WIDTH as i32 { *wx = WIDTH as i32 - ww; }
    if *wy + wh > HEIGHT as i32 - BAR_H as i32 { *wy = HEIGHT as i32 - BAR_H as i32 - wh; }
}

fn draw_desktop(wx: usize, wy: usize) {
    gfx::clear(C_DESKTOP);
    draw_topbar();
    draw_system_window(wx, wy);
    draw_taskbar();
}

fn draw_topbar() {
    gfx::fill_rect(0, 0, WIDTH, BAR_H, C_TITLE);
    gfx::draw_text(2, 2, "Bouchaud OS", C_WHITE);
    let dt = rtc::now();
    let clk = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    gfx::draw_text(WIDTH - clk.len() * 8 - 2, 2, &clk, C_YELLOW);
}

fn draw_taskbar() {
    gfx::fill_rect(0, HEIGHT - BAR_H, WIDTH, BAR_H, C_TITLE);

    let tb = term_btn();
    gfx::fill_rect(tb.x as usize, tb.y as usize, tb.w as usize, tb.h as usize, C_BLUE);
    gfx::draw_text(tb.x as usize + 3, tb.y as usize + 1, "Terminal", C_WHITE);

    let cb = chromium_btn();
    gfx::fill_rect(cb.x as usize, cb.y as usize, cb.w as usize, cb.h as usize, C_BLUE);
    gfx::draw_text(cb.x as usize + 3, cb.y as usize + 1, "Chromium", C_WHITE);

    let qb = quit_btn();
    gfx::fill_rect(qb.x as usize, qb.y as usize, qb.w as usize, qb.h as usize, C_BLUE);
    gfx::draw_text(qb.x as usize + 3, qb.y as usize + 1, "Quitter", C_WHITE);
}

fn draw_system_window(wx: usize, wy: usize) {
    window(wx, wy, WIN_W, WIN_H, "Systeme");
    let tx = wx + 5;
    let mut ty = wy + BAR_H + 4;
    let dt = rtc::now();

    gfx::draw_text(tx, ty, &format!("Version : {}", crate::VERSION), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Session : {}", users::session().username()), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Date    : {:04}-{:02}-{:02}", dt.year, dt.month, dt.day), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Uptime  : {} s", timer::seconds()), C_WHITE); ty += 12;
    gfx::draw_text(tx, ty, "OS souverain francais", C_YELLOW);
}

fn window(x: usize, y: usize, w: usize, h: usize, title: &str) {
    gfx::fill_rect(x + 3, y + 3, w, h, C_DKGRAY);
    gfx::fill_rect(x, y, w, h, C_GRAY);
    gfx::rect(x, y, w, h, C_WHITE);
    gfx::fill_rect(x, y, w, BAR_H, C_BLUE);
    gfx::draw_text(x + 3, y + 2, title, C_WHITE);
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
// Lanceur Chromium experimental
// ---------------------------------------------------------------------------

fn chromium_launcher() {
    crate::serial_println!("[gui] chromium launcher ouvert");
    let mut prev_left = false;

    loop {
        while let Some(key) = keyboard::try_key() {
            match key {
                Key::Escape | Key::Enter | Key::Backspace => {
                    crate::serial_println!("[gui] chromium launcher ferme");
                    return;
                }
                _ => {}
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

        let (mxu, myu) = mouse::pos();
        let mx = mxu as i32;
        let my = myu as i32;
        let left = mouse::left_down();
        let click = left && !prev_left;
        if click && (term_btn().hit(mx, my) || quit_btn().hit(mx, my)) {
            crate::serial_println!("[gui] chromium launcher ferme (bouton)");
            return;
        }
        prev_left = left;

        draw_chromium_launcher();
        draw_cursor(mxu, myu);
        gfx::present();
    }
}

fn draw_chromium_launcher() {
    gfx::clear(C_DESKTOP);
    draw_topbar();
    draw_taskbar();

    let w = 292usize;
    let h = 132usize;
    let x = (WIDTH - w) / 2;
    let y = 32usize;
    window(x, y, w, h, "Chromium");

    let tx = x + 7;
    let mut ty = y + BAR_H + 6;
    gfx::draw_text(tx, ty, "Chromium.exe pret sur /apps", C_YELLOW); ty += 11;
    gfx::draw_text(tx, ty, "Fichier: /apps/chromium.exe", C_WHITE); ty += 11;
    gfx::draw_text(tx, ty, "Etat: lanceur bureau installe", C_GREEN); ty += 13;

    gfx::draw_text(tx, ty, "Execution native: pas encore", C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, "Il manque: processus + loader", C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, "PE/Win32/Chromium + reseau", C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, "e1000 -> TCP/IP -> HTTP/TLS", C_CYAN); ty += 13;

    gfx::draw_text(tx, ty, "Entree/Echap: retour bureau", C_YELLOW);
}

// ---------------------------------------------------------------------------
// Terminal graphique interactif
// ---------------------------------------------------------------------------

fn term_prompt(cwd: usize) -> String {
    format!("{}@bouchaud-os:{}$ ", users::session().username(), shell::path_string(cwd))
}

fn terminal(cwd: &mut usize) {
    crate::serial_println!("[gui] terminal demarre");

    let mut lines: Vec<String> = Vec::new();
    push_wrapped(&mut lines, "Terminal graphique Bouchaud OS", TERM_COLS - 1);
    push_wrapped(&mut lines, "Commandes: help, whoami, ls, sysinfo, date, exit", TERM_COLS - 1);
    push_wrapped(&mut lines, "", TERM_COLS - 1);

    let mut input = String::new();

    loop {
        // Drain clavier a chaque frame : Echap et saisie repondent immediatement.
        while let Some(key) = keyboard::try_key() {
            match key {
                Key::Escape => {
                    crate::serial_println!("[gui] terminal ferme (echap)");
                    return;
                }
                Key::Enter => {
                    let prompt = term_prompt(*cwd);
                    let cmd = input.clone();
                    push_wrapped(&mut lines, &format!("{}{}", prompt, cmd), TERM_COLS - 1);
                    input.clear();

                    let trimmed_owned = shell::trim(&cmd).to_string();
                    let trimmed = trimmed_owned.as_str();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "exit" || trimmed == "logout" {
                        crate::serial_println!("[gui] terminal ferme (exit)");
                        return;
                    }
                    if trimmed == "clear" {
                        lines.clear();
                        continue;
                    }
                    if command_blocked_in_gui(trimmed) {
                        push_wrapped(
                            &mut lines,
                            "Commande bloquee dans le terminal GUI: utiliser le shell texte.",
                            TERM_COLS - 1,
                        );
                        continue;
                    }

                    // Reutilise le shell complet : ;, &&, ||, pipes, redirections, $VAR...
                    let out = shell::run_capture(trimmed, cwd);
                    if out.is_empty() {
                        // Commande valide sans sortie : on ne pollue pas le terminal.
                    } else {
                        for l in out.lines() {
                            push_wrapped(&mut lines, l, TERM_COLS - 1);
                        }
                    }
                    trim_scrollback(&mut lines);
                }
                Key::Backspace => {
                    input.pop();
                }
                Key::Char(c) => {
                    if input.len() < 160 {
                        input.push(c as char);
                    }
                }
                _ => {}
            }
        }

        render_terminal(&lines, &input, *cwd);
        gfx::present();
    }
}

fn command_blocked_in_gui(cmd: &str) -> bool {
    let mut argv = [""; 12];
    let argc = shell::tokenize(cmd, &mut argv);
    if argc == 0 { return false; }
    matches!(argv[0], "desktop" | "gui" | "edit" | "nano" | "panic-test" | "breakpoint")
}

fn render_terminal(lines: &[String], input: &str, cwd: usize) {
    gfx::clear(C_BLACK);
    gfx::fill_rect(0, 0, WIDTH, 9, C_TITLE);
    gfx::draw_text(2, 1, "Bouchaud OS - Terminal", C_WHITE);

    let visible_rows = TERM_ROWS.saturating_sub(1);
    let start = lines.len().saturating_sub(visible_rows);
    let mut y = TERM_TOP;
    for line in &lines[start..] {
        let color = if line.starts_with("root@") || line.starts_with("arthur@") || line.starts_with("guest@") {
            C_GREEN
        } else if line.starts_with("Erreur") || line.starts_with("Commande bloquee") {
            C_YELLOW
        } else {
            C_CYAN
        };
        draw_clipped(2, y, line, TERM_COLS - 1, color);
        y += 8;
    }

    let current = format!("{}{}{}", term_prompt(cwd), input, "_");
    draw_clipped(2, HEIGHT - 8, &current, TERM_COLS - 1, C_WHITE);
}

fn push_wrapped(lines: &mut Vec<String>, text: &str, max: usize) {
    if text.is_empty() {
        lines.push(String::new());
        trim_scrollback(lines);
        return;
    }

    let bytes = text.as_bytes();
    let mut start = 0usize;
    while start < bytes.len() {
        let end = (start + max).min(bytes.len());
        // Les sorties Bouchaud OS sont ASCII pour l'instant. Les accents clavier
        // sont translitteres, donc ce decoupage par octets reste sur.
        lines.push(String::from(&text[start..end]));
        start = end;
    }
    trim_scrollback(lines);
}

fn trim_scrollback(lines: &mut Vec<String>) {
    while lines.len() > MAX_SCROLLBACK {
        lines.remove(0);
    }
}

fn draw_clipped(x: usize, y: usize, s: &str, max_chars: usize, color: u8) {
    let clipped = if s.len() > max_chars { &s[..max_chars] } else { s };
    gfx::draw_text(x, y, clipped, color);
}
