//! Applications natives du bureau et aiguillage des evenements vers la fenetre
//! active (entree clavier, clics, rendu).

pub mod chromium_stub;
pub mod file_explorer;
pub mod system_info;
pub mod terminal;

use crate::gui::event::Key;
use crate::gui::window::{App, Win, TITLE_H};
use crate::fs::ramfs;
use crate::users;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

fn first_word(s: &str) -> &str {
    s.split(' ').next().unwrap_or(s)
}

/// Commandes interactives a ne pas lancer dans le terminal graphique.
fn is_blocked(cmd: &str) -> bool {
    matches!(first_word(cmd),
        "edit" | "nano" | "desktop" | "gui" | "su" | "passwd" | "useradd" | "userdel" | "login")
}

/// Transmet une touche a l'app de la fenetre active. Renvoie true si l'app
/// demande sa fermeture (commande `exit`).
pub(crate) fn key_to_app(w: &mut Win, k: Key, _home: usize) -> bool {
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
                *content = chromium_stub::load_page(url);
                false
            }
            Key::Backspace => { input.pop(); false }
            Key::Char(c) => { if input.len() < 80 { input.push(c as char); } false }
            _ => false,
        },
        _ => false,
    }
}

/// Clic dans le corps d'une application (uniquement Fichiers pour l'instant).
pub(crate) fn app_click(w: &mut Win, _mx: i32, my: i32, _home: usize) {
    if let App::Files { cur, view, name } = &mut w.app {
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
            let mut s = alloc::string::String::new();
            for k in 0..fs.nodes[e].content_len { s.push(fs.nodes[e].content[k] as char); }
            for l in s.split('\n') { lines.push(l.to_string()); }
            *name = fs.nodes[e].name_str().to_string();
            *view = Some(lines);
        }
    }
}

/// Dessine le contenu applicatif d'une fenetre.
pub(crate) fn draw_app(w: &Win) {
    let bx = w.x.max(0) as usize + 3;
    let by = w.y.max(0) as usize + TITLE_H as usize + 2;
    let bw = w.w as usize - 6;
    let bh = w.h as usize - TITLE_H as usize - 4;
    match &w.app {
        App::Terminal { sb, input, cwd } => terminal::draw(sb, input, *cwd, bx, by, bw, bh),
        App::Files { cur, view, name } => file_explorer::draw(*cur, view, name, bx, by, bw, bh),
        App::Browser { url, input, content } => chromium_stub::draw(url, input, content, bx, by, bw, bh),
        App::Monitor => system_info::draw(bx, by, bw, bh),
    }
}
