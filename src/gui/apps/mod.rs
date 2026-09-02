//! Applications natives du bureau et aiguillage des événements.
//!
//! Ce module connecte le gestionnaire de fenêtres aux applications :
//! terminal, explorateur de fichiers, calculatrice, moniteur système.
//!
//! Le navigateur ne figure plus ici : il vit en ring 3, dans
//! `tools/userland/navigateur/`, et s'affiche par Qt sur `/dev/fb0`.

pub mod calculator;
pub mod file_explorer;
pub mod rustpad;
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

fn is_blocked(cmd: &str) -> bool {
    matches!(first_word(cmd),
        "edit" | "nano" | "desktop" | "gui" | "su" | "passwd" | "useradd" | "userdel" | "login")
}

// ── Clavier ───────────────────────────────────────────────────────────────────

/// Transmet une touche à l'application de la fenêtre active.
/// Retourne `true` si l'application demande sa fermeture.
pub(crate) fn key_to_app(w: &mut Win, k: Key, _home: usize) -> bool {
    let new_title: Option<alloc::string::String> = None;

    let close = match &mut w.app {
        App::Terminal { sb, input, cwd } => match k {
            Key::Enter => {
                let prompt = format!("{}:{}$ ", users::session().username(),
                    ramfs::path_string(&ramfs::fs(), *cwd));
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

        App::Calc { expr } => match k {
            Key::Enter     => { calculator::apply_key(expr, "="); false }
            Key::Backspace => { calculator::apply_key(expr, "<"); false }
            Key::Char(c)   => {
                if let Some(lbl) = calculator::key_char(c as char) { calculator::apply_key(expr, lbl); }
                false
            }
            _ => false,
        },
        App::Rustpad { state } => rustpad::on_key(state, k),

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

    if let App::Calc { expr } = &mut w.app {
        let bwi = (win_w - 6).max(1);
        let bhi = (win_h - TITLE_H - 4).max(1);
        if let Some(lbl) = calculator::key_at(bx as i32, by as i32, bwi, bhi, mx, my) {
            calculator::apply_key(expr, lbl);
        }
        return;
    }

    let window_x = w.window.x;
    let window_y = w.window.y;
    if let App::Files { cur, scroll, selected } = &mut w.app {
        let bx  = (window_x + 3).max(0) as usize;
        let by  = (window_y + TITLE_H + 2).max(0) as usize;
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

pub(crate) fn wheel_to_app(w: &mut Win, _mx: i32, _my: i32, delta: i32) {
    if delta == 0 { return; }
    if let App::Files { scroll, .. } = &mut w.app {
        *scroll = (*scroll - delta).max(0);
        return;
    }
    if let App::Rustpad { state } = &mut w.app {
        rustpad::on_wheel(state, delta);
    }
}

// ── Rendu ─────────────────────────────────────────────────────────────────────

pub(crate) fn draw_app(w: &Win) {
    let bx = w.x.max(0) as usize + 3;
    let by = w.y.max(0) as usize + TITLE_H as usize + 2;
    let bw = w.w as usize - 6;
    let bh = w.h as usize - TITLE_H as usize - 4;
    // BOUCHAUD_GFX_CULLING_AMONT_V1
    //
    // Le compositeur rappelle ce peintre pour CHAQUE rectangle de degat qui
    // touche la fenetre -- y compris un degat limite a la barre de titre, a une
    // bordure ou a l'ombre. Les peintres d'application ne se contentent pas de
    // poser des pixels : l'explorateur parcourt le RAMFS et alloue une chaine
    // par ligne, le moniteur lit l'horloge par ports d'E/S et prend les verrous
    // du tas et de l'ordonnanceur -- le tout sous le gros verrou du noyau.
    //
    // La decoupe jetait les pixels ; elle ne pouvait rien contre le travail qui
    // les produit. Ce test le peut.
    //
    // Le rectangle teste est `zone_utile`, pas la boite passee aux peintres :
    // `compose_client` recopie la surface d'un client sur la zone utile
    // ENTIERE, qui deborde de deux pixels a gauche et a droite et d'un en haut
    // et en bas. Tester la boite la plus etroite ecarterait les degats limites
    // a cette bordure, et personne ne la repeindrait.
    let visible = crate::gui::window::zone_utile(w);
    if !crate::gui::framebuffer::decoupe_touche(
        visible.x.max(0) as usize, visible.y.max(0) as usize,
        visible.largeur as usize, visible.hauteur as usize,
    ) {
        return;
    }
    match &w.app {
        App::Terminal { sb, input, cwd }    => terminal::draw(sb, input, *cwd, bx, by, bw, bh),
        App::Files { cur, scroll, selected } => file_explorer::draw(*cur, *scroll, *selected, bx, by, bw, bh),
        App::Calc { expr }                  => calculator::draw(expr, bx, by, bw, bh),
        App::Monitor                        => system_info::draw(bx, by, bw, bh),
        App::Rustpad { state }              => rustpad::draw(state, bx, by, bw, bh),
        // Le contenu d'un client ring 3 n'est pas dessine : il est *compose*.
        // Les pixels existent deja, dans la surface partagee.
        App::Navigateur { client }          => crate::gui::widgets::compose_client(w, client),
    }
}
