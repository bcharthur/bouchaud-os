//! Bureau graphique VGA : lanceur, fenetres deplacables, terminal interactif
//! reutilisant le shell existant, apps maison et jalon haute resolution.
//!
//! Le rendu actuel reste base sur le mode VGA 13h (320x200) tant que la chaine
//! bootloader 0.11/framebuffer n'est pas activee. L'UI est volontairement
//! factorisee pour etre migree vers un framebuffer haute resolution sans toucher
//! a la logique applicative.

use crate::arch::x86_64::rtc;
use crate::drivers::gfx::{
    self, C_BLACK, C_BLUE, C_CYAN, C_DESKTOP, C_DKGRAY, C_GRAY, C_GREEN, C_TITLE, C_WHITE,
    C_YELLOW, HEIGHT, WIDTH,
};
use crate::drivers::{
    keyboard::{self, Key},
    mouse,
};
use crate::kernel::timer;
use crate::{fs::ramfs, shell, users};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

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

/// Lance le bureau graphique (bloquant jusqu'a Echap).
pub fn run() {
    gfx::enter();
    mouse::init();
    crate::serial_println!("[gui] bureau demarre");

    let mut state = DesktopState::new();

    loop {
        while let Some(key) = keyboard::try_key() {
            if handle_key(&mut state, key) {
                close_gui();
                return;
            }
        }

        handle_mouse(&mut state);
        draw(&state);
        let (mx, my) = mouse::pos();
        draw_cursor(mx, my);
        gfx::present();
    }
}

impl DesktopState {
    fn new() -> Self {
        let cwd = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
        let mut lines = Vec::new();
        push_line(&mut lines, "Bouchaud Terminal GUI - shell natif".to_string());
        push_line(&mut lines, "Tapez help, ls, cat, ping, dmesg...".to_string());
        lines.push(prompt(cwd, ""));
        Self {
            app: App::Terminal,
            wx: 14,
            wy: 28,
            dragging: false,
            offx: 0,
            offy: 0,
            prev_left: false,
            cwd,
            input: String::new(),
            lines,
        }
    }
}

fn handle_key(state: &mut DesktopState, key: Key) -> bool {
    match key {
        Key::Escape => true,
        Key::Enter if state.app == App::Terminal => {
            let cmd = state.input.clone();
            refresh_prompt(&mut state.lines, state.cwd, &cmd);
            state.input.clear();
            let trimmed = shell::trim(&cmd);
            if trimmed == "exit" || trimmed == "logout" {
                return true;
            }
            if !trimmed.is_empty() {
                let out = shell::run_line_capture(trimmed, &mut state.cwd);
                if out.is_empty() {
                    push_line(&mut state.lines, String::new());
                } else {
                    for l in out.lines() {
                        push_line(&mut state.lines, l.to_string());
                    }
                }
            }
            state.lines.push(prompt(state.cwd, ""));
            trim_lines(&mut state.lines);
            false
        }
        Key::Backspace if state.app == App::Terminal => {
            state.input.pop();
            refresh_prompt(&mut state.lines, state.cwd, &state.input);
            false
        }
        Key::Char(c) if state.app == App::Terminal => {
            if state.input.len() < 120 {
                state.input.push(c as char);
                refresh_prompt(&mut state.lines, state.cwd, &state.input);
            }
            false
        }
        _ => false,
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

fn app_size(app: App) -> (i32, i32) {
    match app {
        App::Terminal => (292, 124),
        App::Systeme => (142, 82),
        App::Notes => (166, 74),
        App::Navigateur => (250, 94),
    }
}

fn clamp_window(wx: &mut i32, wy: &mut i32, ww: i32, wh: i32) {
    if *wx < 0 { *wx = 0; }
    if *wy < BAR_H as i32 { *wy = BAR_H as i32; }
    if *wx + ww > WIDTH as i32 { *wx = WIDTH as i32 - ww; }
    if *wy + wh > HEIGHT as i32 - STATUS_H as i32 { *wy = HEIGHT as i32 - STATUS_H as i32 - wh; }
}

fn draw(state: &DesktopState) {
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

fn draw_browser(wx: usize, wy: usize) {
    let (w, h) = app_size(App::Navigateur);
    window(wx, wy, w as usize, h as usize, "Navigateur HTTP");
    gfx::fill_rect(wx + 5, wy + BAR_H + 5, w as usize - 10, 10, C_BLACK);
    gfx::draw_text(wx + 8, wy + BAR_H + 7, "http://127.0.0.1/", C_GREEN);
    gfx::draw_text(wx + 5, wy + BAR_H + 22, "Mini navigateur texte prepare.", C_YELLOW);
    gfx::draw_text(wx + 5, wy + BAR_H + 34, "Activation apres driver e1000", C_WHITE);
    gfx::draw_text(wx + 5, wy + BAR_H + 46, "+ TCP/HTTP client.", C_WHITE);
}

fn window(x: usize, y: usize, w: usize, h: usize, title: &str) {
    gfx::fill_rect(x + 3, y + 3, w, h, C_DKGRAY);
    gfx::fill_rect(x, y, w, h, C_GRAY);
    gfx::rect(x, y, w, h, C_WHITE);
    gfx::fill_rect(x, y, w, BAR_H, C_BLUE);
    gfx::draw_text(x + 3, y + 2, title, C_WHITE);
}

fn push_line(lines: &mut Vec<String>, s: String) {
    lines.push(s);
    trim_lines(lines);
}

fn trim_lines(lines: &mut Vec<String>) {
    while lines.len() > MAX_LINES {
        lines.remove(0);
    }
}

fn refresh_prompt(lines: &mut Vec<String>, cwd: usize, input: &str) {
    if let Some(last) = lines.last_mut() {
        *last = prompt(cwd, input);
    }
}

fn prompt(cwd: usize, input: &str) -> String {
    format!("$ {}{}", shell::path_string(cwd), input)
}

fn draw_clipped_text(x: usize, y: usize, s: &str, max_chars: usize, color: u8) {
    let mut shown = s;
    if shown.len() > max_chars {
        shown = &shown[..max_chars];
    }
    gfx::draw_text(x, y, shown, color);
}

/// Curseur fleche 8x8 (blanc).
fn draw_cursor(mx: usize, my: usize) {
    const CUR: [u8; 8] = [
        0b00000001,
        0b00000011,
        0b00000111,
        0b00001111,
        0b00011111,
        0b00000111,
        0b00001101,
        0b00011000,
    ];
    for (row, bits) in CUR.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                gfx::pixel(mx + col, my + row, C_WHITE);
            }
        }
    }
}
