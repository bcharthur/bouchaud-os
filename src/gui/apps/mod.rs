//! Applications natives du bureau et aiguillage des événements.
//!
//! Ce module connecte le gestionnaire de fenêtres aux applications :
//! terminal, explorateur de fichiers, calculatrice, moniteur système.
//!
//! Le navigateur ne figure plus ici : il vit en ring 3, dans
//! `tools/userland/navigateur/`, et s'affiche par Qt sur `/dev/fb0`.

pub mod calculator;
pub mod file_explorer;
pub mod journal;
pub mod rustpad;
pub mod notes;
pub mod insights;
pub mod system_info;
pub mod services;
pub mod terminal;

use crate::gui::event::Key;
use crate::gui::window::{App, Win, TITLE_H};
use crate::fs::ramfs;
use crate::users;
use alloc::format;
use alloc::string::ToString;

fn first_word(s: &str) -> &str {
    s.split(' ').next().unwrap_or(s)
}

fn is_blocked(cmd: &str) -> bool {
    matches!(first_word(cmd),
        "edit" | "nano" | "desktop" | "gui" | "su" | "passwd" | "useradd" | "userdel" | "login")
}

fn journal_gui_special(cmd: &str, cwd: usize, status: i32, sortie: Option<&str>) {
    let chemin = ramfs::path_string(&ramfs::fs(), cwd);
    crate::kernel::blackbox::terminal_commande("gui", &chemin, cmd);
    if let Some(texte) = sortie {
        crate::kernel::blackbox::terminal_sortie_texte("gui", texte);
    }
    crate::kernel::blackbox::terminal_resultat("gui", status);
}

// ── Demandes d'ouverture ──────────────────────────────────────────────────────
//
// BOUCHAUD_NOTES_OUVRIR_V1 : Fichiers demande qu'un fichier s'ouvre dans Notes.
// Une application n'a pas acces a la liste des fenetres : elle depose la
// demande, le bureau la prend a son prochain tour et ouvre la fenetre.

static DEMANDE_NOTES: crate::kernel::sync::SpinLock<Option<alloc::string::String>> =
    crate::kernel::sync::SpinLock::new(None);

pub(crate) fn demande_notes(chemin: alloc::string::String) {
    *DEMANDE_NOTES.lock() = Some(chemin);
}

pub(crate) fn prends_demande_notes() -> Option<alloc::string::String> {
    DEMANDE_NOTES.lock().take()
}

// ── Clavier ───────────────────────────────────────────────────────────────────

/// Transmet une touche à l'application de la fenêtre active.
/// Retourne `true` si l'application demande sa fermeture.
pub(crate) fn key_to_app(w: &mut Win, k: Key, _home: usize) -> bool {
    let new_title: Option<alloc::string::String> = None;
    let (files_l, files_h) = zone_files(w.w, w.h);

    let close = match &mut w.app {
        App::Terminal { sb, input, cwd } => match k {
            Key::Enter => {
                let prompt = format!("{}:{}$ ", users::session().username(),
                    ramfs::path_string(&ramfs::fs(), *cwd));
                sb.push(format!("{}{}", prompt, input));
                let cmd = input.trim().to_string();
                input.clear();
                if cmd.is_empty() { return false; }
                if cmd == "exit" {
                    journal_gui_special(&cmd, *cwd, 0, Some("fermeture du terminal"));
                    return true;
                }
                if cmd == "clear" {
                    journal_gui_special(&cmd, *cwd, 0, Some("ecran efface"));
                    sb.clear();
                    return false;
                }
                if is_blocked(&cmd) {
                    let message = format!("{}: a lancer depuis le shell texte", first_word(&cmd));
                    journal_gui_special(&cmd, *cwd, 126, Some(&message));
                    sb.push(message);
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
        App::Files { cur, scroll, selected, history, ui } => {
            file_explorer::touche_clavier(cur, scroll, selected, history, ui, files_l, files_h, k);
            false
        }
        App::Rustpad { state } => rustpad::on_key(state, k),
        App::Notes { state } => notes::on_key(state, k),
        App::Journal { state } => journal::on_key(state, k),

        _ => false,
    };

    if let Some(t) = new_title { w.title = t; }
    close
}

// ── Souris : clic ─────────────────────────────────────────────────────────────

pub(crate) fn app_click(w: &mut Win, mx: i32, my: i32, _home: usize) {
    if matches!(w.app, App::Services) {
        services::click(mx - w.x - 3, my - w.y - TITLE_H - 2);
        return;
    }
    let win_w = w.w;
    let win_h = w.h;
    let bx    = (w.x + 3).max(0) as usize;
    let by    = (w.y + TITLE_H + 2).max(0) as usize;

    if let App::Notes { state } = &mut w.app {
        notes::click(state, mx - bx as i32, my - by as i32);
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

    let window_x = w.window.x;
    let window_y = w.window.y;
    if let App::Files { cur, scroll, selected, history, ui } = &mut w.app {
        let (bw, bh) = zone_files(win_w, win_h);
        let bx = window_x + 3;
        let by = window_y + TITLE_H + 2;
        file_explorer::clic(cur, scroll, selected, history, ui, bw, bh, mx - bx, my - by);
    }
}

/// Taille de la zone utile d'une fenetre Fichiers (celle que `draw_app` passe
/// au peintre).
fn zone_files(win_w: i32, win_h: i32) -> (i32, i32) {
    ((win_w - 6).max(1), (win_h - TITLE_H - 4).max(1))
}

/// Clic droit dans une application native. Seul Fichiers a un menu
/// contextuel ; les autres l'ignorent.
pub(crate) fn app_clic_droit(w: &mut Win, mx: i32, my: i32) {
    let (win_w, win_h, x0, y0) = (w.w, w.h, w.window.x + 3, w.window.y + TITLE_H + 2);
    if let App::Files { cur, scroll, selected, ui, .. } = &mut w.app {
        let (bw, bh) = zone_files(win_w, win_h);
        file_explorer::clic_droit(*cur, *scroll, selected, ui, bw, bh, mx - x0, my - y0);
    }
}

/// Echap dans une application native : rend `true` si l'application l'a pris
/// (un menu contextuel s'est ferme). Sinon le bureau ferme la fenetre, comme
/// avant.
pub(crate) fn app_echap(w: &mut Win) -> bool {
    if let App::Files { ui, .. } = &mut w.app {
        return file_explorer::echap(ui);
    }
    false
}

/// Survol dans une application native : rend les rectangles ECRAN a
/// recomposer (ancienne et nouvelle cible) quand la cible survolee change.
pub(crate) fn app_survol(w: &mut Win, mx: i32, my: i32) -> Option<(crate::gui::protocole::Rect, crate::gui::protocole::Rect)> {
    let (win_w, win_h, x0, y0) = (w.w, w.h, w.window.x + 3, w.window.y + TITLE_H + 2);
    if let App::Files { cur, scroll, ui, .. } = &mut w.app {
        let (bw, bh) = zone_files(win_w, win_h);
        let (a, b) = file_explorer::survol(*cur, *scroll, ui, bw, bh, mx - x0, my - y0)?;
        let ecran = |r: crate::gui::atelier::dock::Boite| crate::gui::protocole::Rect::neuf(
            r.x + x0 - 1, r.y + y0 - 1, (r.w.max(0) + 2) as u32, (r.h.max(0) + 2) as u32);
        return Some((ecran(a), ecran(b)));
    }
    None
}

// ── Souris : molette ──────────────────────────────────────────────────────────

pub(crate) fn wheel_to_app(w: &mut Win, _mx: i32, _my: i32, delta: i32) {
    if delta == 0 { return; }
    let (win_w, win_h) = (w.w, w.h);
    if let App::Files { cur, scroll, ui, .. } = &mut w.app {
        let (bw, bh) = zone_files(win_w, win_h);
        file_explorer::molette(*cur, scroll, ui, bw, bh, delta);
        return;
    }
    if let App::Journal { state } = &mut w.app {
        journal::on_wheel(state, delta);
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
        App::Files { cur, scroll, selected, history, ui } =>
            file_explorer::draw(*cur, *scroll, *selected, history, ui, bx, by, bw, bh),
        App::Calc { expr }                  => calculator::draw(expr, bx, by, bw, bh),
        App::Services                       => services::draw(bx, by, bw, bh),
        App::Monitor                        => system_info::draw(bx, by, bw, bh),
        App::Insights                       => insights::draw(bx, by, bw, bh),
        App::Notes { state }                => notes::draw(state, bx, by, bw, bh),
        App::Rustpad { state }              => rustpad::draw(state, bx, by, bw, bh),
        App::Journal { state }              => journal::draw(state, bx, by, bw, bh),
        // Le contenu d'un client ring 3 n'est pas dessine : il est *compose*.
        // Les pixels existent deja, dans la surface partagee.
        App::Navigateur { client }          => crate::gui::widgets::compose_client(w, client),
    }
}
