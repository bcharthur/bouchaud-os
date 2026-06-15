//! Bureau graphique minimal (mode VGA 13h) : fond, barre des taches, horloge,
//! fenetres deplacables, lanceur d'applications et terminal interactif.
//!
//! Lance par la commande `desktop`. Echap quitte immediatement le GUI, restaure
//! le mode texte et rend la main au shell.

use crate::drivers::gfx::{self, C_DESKTOP, C_TITLE, C_GRAY, C_DKGRAY, C_WHITE, C_BLUE, C_YELLOW, C_GREEN, C_CYAN, C_BLACK, WIDTH, HEIGHT};
use crate::drivers::{keyboard::{self, Key}, mouse};
use crate::arch::x86_64::rtc;
use crate::kernel::timer;
use crate::{fs::ramfs, shell, users};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const BAR_H: usize = 11;
const SYS_W: i32 = 128;
const SYS_H: i32 = 72;
const TERM_W: i32 = 286;
const TERM_H: i32 = 112;
const NOTES_W: i32 = 142;
const NOTES_H: i32 = 62;

#[derive(Clone, Copy, PartialEq)]
enum App { Systeme, Terminal, Notes }

/// Lance le bureau graphique (bloquant jusqu'a Echap).
pub fn run() {
    gfx::enter();
    mouse::init();
    crate::serial_println!("[gui] bureau demarre");

    let mut app = App::Terminal;
    let mut wx: i32 = 18;
    let mut wy: i32 = 30;
    let mut dragging = false;
    let mut offx = 0i32;
    let mut offy = 0i32;
    let mut prev_left = false;

    let mut cwd = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut input = String::new();
    let mut lines: Vec<String> = Vec::new();
    push_line(&mut lines, "Bouchaud Terminal GUI - shell reutilise".to_string());
    push_prompt(&mut lines, cwd, &input);

    loop {
        while let Some(key) = keyboard::try_key() {
            match key {
                Key::Escape => { gfx::leave(); crate::serial_println!("[gui] bureau ferme"); return; }
                Key::Enter if app == App::Terminal => {
                    let cmd = input.clone();
                    replace_prompt_with_command(&mut lines, cwd, &cmd);
                    input.clear();
                    let trimmed = shell::trim(&cmd);
                    if trimmed == "exit" || trimmed == "logout" { gfx::leave(); crate::serial_println!("[gui] bureau ferme"); return; }
                    if !trimmed.is_empty() {
                        let out = shell::run_line_capture(trimmed, &mut cwd);
                        for l in out.lines() { push_line(&mut lines, l.to_string()); }
                    }
                    push_prompt(&mut lines, cwd, &input);
                }
                Key::Backspace if app == App::Terminal => { input.pop(); refresh_prompt(&mut lines, cwd, &input); }
                Key::Char(c) if app == App::Terminal => {
                    if input.len() < 96 { input.push(c as char); refresh_prompt(&mut lines, cwd, &input); }
                }
                _ => {}
            }
        }

        let (mxu, myu) = mouse::pos();
        let mx = mxu as i32;
        let my = myu as i32;
        let left = mouse::left_down();
        if left && !prev_left && my < BAR_H as i32 {
            if mx < 76 { app = App::Terminal; wx = 18; wy = 30; }
            else if mx < 146 { app = App::Systeme; wx = 96; wy = 40; }
            else if mx < 216 { app = App::Notes; wx = 90; wy = 60; }
        }
        prev_left = left;

        let (ww, wh) = app_size(app);
        if left {
            if !dragging && my >= wy && my < wy + BAR_H as i32 && mx >= wx && mx < wx + ww {
                dragging = true; offx = mx - wx; offy = my - wy;
            }
            if dragging { wx = mx - offx; wy = my - offy; }
        } else { dragging = false; }
        clamp_window(&mut wx, &mut wy, ww, wh);

        draw(app, wx as usize, wy as usize, &lines);
        draw_cursor(mxu, myu);
        gfx::present();
    }
}

fn app_size(app: App) -> (i32, i32) {
    match app { App::Systeme => (SYS_W, SYS_H), App::Terminal => (TERM_W, TERM_H), App::Notes => (NOTES_W, NOTES_H) }
}

fn clamp_window(wx: &mut i32, wy: &mut i32, ww: i32, wh: i32) {
    if *wx < 0 { *wx = 0; }
    if *wy < BAR_H as i32 { *wy = BAR_H as i32; }
    if *wx + ww > WIDTH as i32 { *wx = WIDTH as i32 - ww; }
    if *wy + wh > HEIGHT as i32 - BAR_H as i32 { *wy = HEIGHT as i32 - BAR_H as i32 - wh; }
}

fn draw(app: App, wx: usize, wy: usize, lines: &[String]) {
    gfx::clear(C_DESKTOP);
    draw_topbar(app);
    match app {
        App::Systeme => draw_system(wx, wy),
        App::Terminal => draw_terminal(wx, wy, lines),
        App::Notes => draw_notes(wx, wy),
    }
    gfx::fill_rect(0, HEIGHT - BAR_H, WIDTH, BAR_H, C_TITLE);
    gfx::draw_text(2, HEIGHT - BAR_H + 2, "Echap=quitter  clic titre=deplacer", C_WHITE);
}

fn draw_topbar(active: App) {
    gfx::fill_rect(0, 0, WIDTH, BAR_H, C_TITLE);
    launcher(2, "Terminal", active == App::Terminal);
    launcher(78, "Systeme", active == App::Systeme);
    launcher(150, "Notes", active == App::Notes);
    let dt = rtc::now();
    let clk = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    gfx::draw_text(WIDTH - clk.len() * 8 - 2, 2, &clk, C_YELLOW);
}

fn launcher(x: usize, label: &str, active: bool) {
    gfx::fill_rect(x, 1, label.len() * 8 + 6, 9, if active { C_BLUE } else { C_DKGRAY });
    gfx::draw_text(x + 3, 2, label, C_WHITE);
}

fn draw_system(wx: usize, wy: usize) {
    window(wx, wy, SYS_W as usize, SYS_H as usize, "Systeme");
    let tx = wx + 4; let mut ty = wy + BAR_H + 3; let dt = rtc::now();
    gfx::draw_text(tx, ty, &format!("Version {}", crate::VERSION), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Session {}", users::session().username()), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Date {:04}-{:02}-{:02}", dt.year, dt.month, dt.day), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Uptime {} s", timer::seconds()), C_WHITE); ty += 12;
    gfx::draw_text(tx, ty, "OS souverain FR", C_YELLOW);
}

fn draw_terminal(wx: usize, wy: usize, lines: &[String]) {
    window(wx, wy, TERM_W as usize, TERM_H as usize, "Terminal");
    gfx::fill_rect(wx + 3, wy + BAR_H + 2, TERM_W as usize - 6, TERM_H as usize - BAR_H - 5, C_BLACK);
    let max = ((TERM_H as usize - BAR_H - 8) / 9).min(lines.len());
    let start = lines.len().saturating_sub(max);
    let mut y = wy + BAR_H + 5;
    for l in &lines[start..] {
        let mut shown = l.as_str();
        if shown.len() > 34 { shown = &shown[..34]; }
        gfx::draw_text(wx + 6, y, shown, if shown.starts_with('$') { C_GREEN } else { C_CYAN });
        y += 9;
    }
}

fn draw_notes(wx: usize, wy: usize) {
    window(wx, wy, NOTES_W as usize, NOTES_H as usize, "Apps maison");
    gfx::draw_text(wx + 5, wy + BAR_H + 5, "Lanceur natif OK", C_YELLOW);
    gfx::draw_text(wx + 5, wy + BAR_H + 17, "Terminal + Systeme", C_WHITE);
    gfx::draw_text(wx + 5, wy + BAR_H + 29, "Prochain: navigateur", C_WHITE);
}

fn window(x: usize, y: usize, w: usize, h: usize, title: &str) {
    gfx::fill_rect(x + 3, y + 3, w, h, C_DKGRAY);
    gfx::fill_rect(x, y, w, h, C_GRAY);
    gfx::rect(x, y, w, h, C_WHITE);
    gfx::fill_rect(x, y, w, BAR_H, C_BLUE);
    gfx::draw_text(x + 3, y + 2, title, C_WHITE);
}

fn push_prompt(lines: &mut Vec<String>, cwd: usize, input: &str) { lines.push(format!("{}", prompt(cwd, input))); }
fn refresh_prompt(lines: &mut Vec<String>, cwd: usize, input: &str) { if let Some(last) = lines.last_mut() { *last = prompt(cwd, input); } }
fn replace_prompt_with_command(lines: &mut Vec<String>, cwd: usize, input: &str) { if let Some(last) = lines.last_mut() { *last = prompt(cwd, input); } }
fn push_line(lines: &mut Vec<String>, s: String) { lines.push(s); if lines.len() > 64 { lines.remove(0); } }
fn prompt(cwd: usize, input: &str) -> String { format!("$ {}{}", path(cwd), input) }
fn path(cwd: usize) -> String { if cwd == 0 { "/ ".to_string() } else { "".to_string() } }

/// Curseur fleche 8x8 (blanc).
fn draw_cursor(mx: usize, my: usize) {
    const CUR: [u8; 8] = [0b00000001,0b00000011,0b00000111,0b00001111,0b00011111,0b00000111,0b00001101,0b00011000];
    for (row, bits) in CUR.iter().enumerate() { for col in 0..8 { if bits & (1 << col) != 0 { gfx::pixel(mx + col, my + row, C_WHITE); } } }
}
