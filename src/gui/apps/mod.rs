//! Applications natives du bureau et aiguillage des événements.
//!
//! Ce module connecte le gestionnaire de fenêtres aux applications :
//! terminal, explorateur de fichiers, Nautile (navigateur), calculatrice,
//! moniteur système.

pub mod calculator;
pub mod file_explorer;
pub mod system_info;
pub mod terminal;
// Nautile vit désormais dans src/browser/ — pas de module local.

use crate::browser;
use crate::browser::ui::chrome::{self, ChromeEvent};
use crate::gui::event::Key;
use crate::gui::framebuffer as fb;
use crate::gui::window::{App, Win, TITLE_H};
use crate::fs::ramfs;
use crate::users;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

fn first_word(s: &str) -> &str {
    s.split(' ').next().unwrap_or(s)
}

fn is_blocked(cmd: &str) -> bool {
    matches!(first_word(cmd),
        "edit" | "nano" | "desktop" | "gui" | "su" | "passwd" | "useradd" | "userdel" | "login")
}

// ── Clavier ───────────────────────────────────────────────────────────────────

/// Transmet une touche à l'application de la fenêtre active.
/// Retourne `true` si l'application demande sa fermeture.
pub(crate) fn key_to_app(w: &mut Win, k: Key, _home: usize) -> bool {
    let win_w = w.w;
    let win_h = w.h;
    let win_x = w.x;
    let win_y = w.y;
    let bx = (win_x + 3).max(0) as usize;
    let by = (win_y + TITLE_H + 2).max(0) as usize;
    let bw = (win_w - 6).max(1) as usize;
    let bh = (win_h - TITLE_H - 4).max(1) as usize;

    let mut new_title: Option<alloc::string::String> = None;

    let close = match &mut w.app {
        App::Terminal { sb, input, cwd } => match k {
            Key::Enter => {
                let prompt = format!("{}:{}$ ", users::session().username(),
                    ramfs::path_string(ramfs::fs(), *cwd));
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
            Key::Char(c)   => { if input.len() < 120 { input.push(c as char); } false }
            _ => false,
        },

        App::Browser { state } => {
            let event = chrome::on_key(state, k, bh);
            handle_browser_event(state, event, bx, by, bw, bh, &mut new_title);
            false
        }

        App::Calc { expr } => match k {
            Key::Enter     => { calculator::apply_key(expr, "="); false }
            Key::Backspace => { calculator::apply_key(expr, "<"); false }
            Key::Char(c)   => {
                if let Some(lbl) = calculator::key_char(c as char) { calculator::apply_key(expr, lbl); }
                false
            }
            _ => false,
        },
        _ => false,
    };

    if let Some(t) = new_title { w.title = t; }
    close
}

// ── Souris : clic ─────────────────────────────────────────────────────────────

pub(crate) fn app_click(w: &mut Win, mx: i32, my: i32, _home: usize) {
    let win_w = w.w;
    let win_h = w.h;
    let bx    = (w.x + 3).max(0) as usize;
    let by    = (w.y + TITLE_H + 2).max(0) as usize;
    let bw    = (win_w - 6).max(1) as usize;
    let bh    = (win_h - TITLE_H - 4).max(1) as usize;

    if let App::Browser { state } = &mut w.app {
        let rel_x = mx - bx as i32;
        let rel_y = my - by as i32;
        let event = chrome::on_click(state, rel_x, rel_y, bw, bh);
        let mut new_title = None;
        handle_browser_event(state, event, bx, by, bw, bh, &mut new_title);
        if let Some(t) = new_title { w.title = t; }
        return;
    }

    if let App::Calc { expr } = &mut w.app {
        let bwi = (win_w - 6).max(1);
        let bhi = (win_h - TITLE_H - 4).max(1);
        if let Some(lbl) = calculator::key_at(bx as i32, by as i32, bwi, bhi, mx, my) {
            calculator::apply_key(expr, lbl);
        }
        return;
    }

    if let App::Files { cur, scroll, selected } = &mut w.app {
        let bx  = (w.x + 3).max(0) as usize;
        let by  = (w.y + TITLE_H + 2).max(0) as usize;
        let bw  = (win_w - 6).max(1) as usize;
        let tbh = file_explorer::TOOLBAR_H;

        // Clic sur la barre d'outils
        let tb_action = file_explorer::toolbar_hit(bx, by, mx, my);
        match tb_action {
            file_explorer::ToolbarAction::Up => {
                let fs = ramfs::fs();
                if *cur != 0 { *cur = fs.nodes[*cur].parent; *scroll = 0; *selected = None; }
                return;
            }
            file_explorer::ToolbarAction::Back | file_explorer::ToolbarAction::Forward => {
                // Navigation historique non implementee (placeholder)
                return;
            }
            file_explorer::ToolbarAction::None => {}
        }

        // Clic dans la grille
        let grid_y = by + tbh + 1;
        if (my as usize) < grid_y { return; }
        let grid_rel_my = my - grid_y as i32;
        if grid_rel_my < 0 { return; }
        let hit_idx = file_explorer::grid_hit(*cur, *scroll, bx, grid_y, bw, mx, my);
        if let Some(idx) = hit_idx {
            let fs = ramfs::fs();
            let mut entries: Vec<(usize, bool)> = Vec::new();
            if *cur != 0 { entries.push((usize::MAX, true)); }
            for i in 0..ramfs::MAX_NODES {
                if fs.nodes[i].used && i != *cur && fs.nodes[i].parent == *cur {
                    entries.push((i, fs.nodes[i].kind == ramfs::NodeKind::Dir));
                }
            }
            if idx >= entries.len() { *selected = None; return; }
            let (node, is_dir) = entries[idx];
            if node == usize::MAX {
                *cur = fs.nodes[*cur].parent; *scroll = 0; *selected = None;
            } else if is_dir {
                if fs.can(node, ramfs::PERM_X) { *cur = node; *scroll = 0; *selected = None; }
            } else {
                *selected = Some(idx);
            }
        }
    }
}

// ── Souris : molette ──────────────────────────────────────────────────────────

pub(crate) fn wheel_to_app(w: &mut Win, mx: i32, my: i32, delta: i32) {
    if delta == 0 { return; }
    if let App::Files { scroll, .. } = &mut w.app {
        *scroll = (*scroll - delta).max(0);
        return;
    }
    if let App::Browser { state } = &mut w.app {
        let bx = w.x + 3;
        let by = w.y + TITLE_H + 2;
        let bw = (w.w - 6).max(1) as usize;
        let bh = (w.h - TITLE_H - 4).max(1) as usize;
        if mx < bx || mx >= bx + bw as i32 || my < by || my >= by + bh as i32 { return; }
        if let ChromeEvent::ScrollTo(s) = chrome::on_wheel(state, delta, bh) {
            state.tab_mut().scroll = s;
        }
    }
}

// ── Rendu ─────────────────────────────────────────────────────────────────────

pub(crate) fn draw_app(w: &Win) {
    let bx = w.x.max(0) as usize + 3;
    let by = w.y.max(0) as usize + TITLE_H as usize + 2;
    let bw = w.w as usize - 6;
    let bh = w.h as usize - TITLE_H as usize - 4;
    match &w.app {
        App::Terminal { sb, input, cwd }    => terminal::draw(sb, input, *cwd, bx, by, bw, bh),
        App::Files { cur, scroll, selected } => file_explorer::draw(*cur, *scroll, *selected, bx, by, bw, bh),
        App::Browser { state }              => chrome::draw(state, bx, by, bw, bh),
        App::Calc { expr }                  => calculator::draw(expr, bx, by, bw, bh),
        App::Monitor                        => system_info::draw(bx, by, bw, bh),
    }
}

// ── Gestionnaire d'événements navigateur ──────────────────────────────────────

fn handle_browser_event(
    state:     &mut browser::BrowserState,
    event:     ChromeEvent,
    bx: usize, by: usize, bw: usize, bh: usize,
    new_title: &mut Option<alloc::string::String>,
) {
    match event {
        ChromeEvent::Navigate(href) => {
            let target = if href == state.tab().input {
                // Touche Entrée avec l'URL déjà dans la barre → résoudre
                browser::loader::resolve_input(&href, &state.tab().page)
            } else {
                href
            };
            chrome::draw_loading(&target, bx, by, bw, bh);
            fb::present();
            let (sess, pg) = browser::loader::open(&target, bw as i32);
            state.tab_mut().push_nav(&target);
            state.tab_mut().apply(&target, pg, sess);
            *new_title = Some(state.tab().title.clone());
        }

        ChromeEvent::Back => {
            if let Some(url) = state.tab_mut().go_back() {
                chrome::draw_loading(&url, bx, by, bw, bh);
                fb::present();
                let (sess, pg) = browser::loader::open(&url, bw as i32);
                state.tab_mut().apply(&url, pg, sess);
                *new_title = Some(state.tab().title.clone());
            }
        }

        ChromeEvent::Forward => {
            if let Some(url) = state.tab_mut().go_forward() {
                chrome::draw_loading(&url, bx, by, bw, bh);
                fb::present();
                let (sess, pg) = browser::loader::open(&url, bw as i32);
                state.tab_mut().apply(&url, pg, sess);
                *new_title = Some(state.tab().title.clone());
            }
        }

        ChromeEvent::Refresh => {
            let url = state.tab().url.clone();
            chrome::draw_loading(&url, bx, by, bw, bh);
            fb::present();
            let (sess, pg) = browser::loader::open(&url, bw as i32);
            state.tab_mut().apply(&url, pg, sess);
            *new_title = Some(state.tab().title.clone());
        }

        ChromeEvent::Home => {
            let url = "about:bouchaud".to_string();
            chrome::draw_loading(&url, bx, by, bw, bh);
            fb::present();
            let (sess, pg) = browser::loader::open(&url, bw as i32);
            state.tab_mut().push_nav(&url);
            state.tab_mut().apply(&url, pg, sess);
            *new_title = Some(state.tab().title.clone());
        }

        ChromeEvent::NewTab => {
            let url = "about:bouchaud".to_string();
            let (sess, pg) = browser::loader::open(&url, bw as i32);
            state.add_tab(url, pg, sess);
            *new_title = Some(state.tab().title.clone());
        }

        ChromeEvent::CloseTab(i) => {
            state.close_tab_at(i);
            *new_title = Some(state.tab().title.clone());
        }

        ChromeEvent::SelectTab(i) => {
            state.select(i);
            *new_title = Some(state.tab().title.clone());
        }

        ChromeEvent::ScrollTo(s) => {
            state.tab_mut().scroll = s;
        }

        ChromeEvent::InputChar(c) => {
            if state.tab().input.len() < 200 {
                state.tab_mut().input.push(c);
            }
        }

        ChromeEvent::InputBackspace => {
            state.tab_mut().input.pop();
        }

        ChromeEvent::DispatchJs(code) => {
            let pg = state.tab_mut().session.dispatch(&code);
            state.tab_mut().page = pg;
        }

        ChromeEvent::None => {}
    }
}
