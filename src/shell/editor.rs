//! Editeur de texte plein ecran minimal (style nano/vi simplifie).
//!
//! Navigation aux fleches, insertion/suppression, et un menu (touche Esc) pour
//! sauvegarder/quitter. Tampon en memoire (Vec<String>) grace a `alloc`.

use crate::drivers::keyboard::{self, Key};
use crate::drivers::vga::{self, COLOR_CYAN, COLOR_DEFAULT, COLOR_YELLOW};
use crate::fs::ramfs::{self, NodeKind, PERM_R};
use crate::shell::commands;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const HEADER_ROW: usize = 0;
const FIRST_TEXT_ROW: usize = 1;
const STATUS_ROW: usize = vga::HEIGHT - 1;
const VISIBLE: usize = STATUS_ROW - FIRST_TEXT_ROW; // lignes de texte affichables

/// Lance l'editeur sur `path` (cree le fichier a la sauvegarde s'il n'existe pas).
pub fn edit(path: &str, cwd: usize) {
    let mut lines = load(path, cwd);
    if lines.is_empty() { lines.push(String::new()); }

    let mut cy = 0usize; // ligne courante
    let mut cx = 0usize; // colonne courante
    let mut top = 0usize; // premiere ligne affichee
    let mut dirty = false;

    loop {
        // Defilement vertical pour garder le curseur visible.
        if cy < top { top = cy; }
        if cy >= top + VISIBLE { top = cy + 1 - VISIBLE; }

        render(path, &lines, cy, cx, top, dirty);

        match keyboard::read_key() {
            Key::Up => { if cy > 0 { cy -= 1; clamp(&lines, cy, &mut cx); } }
            Key::Down => { if cy + 1 < lines.len() { cy += 1; clamp(&lines, cy, &mut cx); } }
            Key::Left => {
                if cx > 0 { cx -= 1; }
                else if cy > 0 { cy -= 1; cx = lines[cy].len(); }
            }
            Key::Right => {
                if cx < lines[cy].len() { cx += 1; }
                else if cy + 1 < lines.len() { cy += 1; cx = 0; }
            }
            Key::Backspace => {
                if cx > 0 {
                    cx -= 1;
                    lines[cy].remove(cx);
                    dirty = true;
                } else if cy > 0 {
                    let cur = lines.remove(cy);
                    cy -= 1;
                    cx = lines[cy].len();
                    lines[cy].push_str(&cur);
                    dirty = true;
                }
            }
            Key::Enter => {
                let rest = lines[cy].split_off(cx);
                lines.insert(cy + 1, rest);
                cy += 1;
                cx = 0;
                dirty = true;
            }
            Key::Char(c) => {
                lines[cy].insert(cx, c as char);
                cx += 1;
                dirty = true;
            }
            Key::Other => {
                // Menu Esc : s=sauver, q=quitter, x=sauver+quitter.
                match menu() {
                    b's' | b'w' => { save(path, &lines, cwd); dirty = false; }
                    b'q' => { vga::clear(); return; }
                    b'x' => { save(path, &lines, cwd); vga::clear(); return; }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn clamp(lines: &[String], cy: usize, cx: &mut usize) {
    if *cx > lines[cy].len() { *cx = lines[cy].len(); }
}

fn load(path: &str, cwd: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let fs = ramfs::fs();
    if let Ok(idx) = fs.resolve_checked(path, cwd) {
        if fs.nodes[idx].kind == NodeKind::File && fs.can(idx, PERM_R) {
            let n = &fs.nodes[idx];
            let mut s = String::new();
            for i in 0..n.content_len { s.push(n.content[i] as char); }
            for line in s.split('\n') { out.push(line.to_string()); }
        }
    }
    out
}

fn save(path: &str, lines: &[String], cwd: usize) {
    let mut content = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 { content.push('\n'); }
        content.push_str(l);
    }
    commands::redirect(path, &content, false, cwd);
}

fn render(path: &str, lines: &[String], cy: usize, cx: usize, top: usize, dirty: bool) {
    vga::clear();
    // En-tete.
    vga::set_cursor(HEADER_ROW, 0);
    vga::set_color(COLOR_CYAN);
    print!("Bouchaud edit  {}{}", path, if dirty { " *" } else { "" });
    vga::set_color(COLOR_DEFAULT);

    // Zone de texte.
    for i in 0..VISIBLE {
        let li = top + i;
        if li >= lines.len() { break; }
        vga::set_cursor(FIRST_TEXT_ROW + i, 0);
        let line = &lines[li];
        let max = vga::WIDTH - 1;
        if line.len() > max {
            print!("{}", &line[..max]);
        } else {
            print!("{}", line);
        }
    }

    // Barre de statut.
    vga::set_cursor(STATUS_ROW, 0);
    vga::set_color(COLOR_YELLOW);
    print!("Esc=menu (s=sauver q=quitter x=tout)  Lig {}/{} Col {}", cy + 1, lines.len(), cx + 1);
    vga::set_color(COLOR_DEFAULT);

    // Place le curseur visible a la position d'edition.
    let scr_row = FIRST_TEXT_ROW + (cy - top);
    let scr_col = if cx >= vga::WIDTH { vga::WIDTH - 1 } else { cx };
    vga::set_cursor(scr_row, scr_col);
}

fn menu() -> u8 {
    vga::set_cursor(STATUS_ROW, 0);
    vga::set_color(COLOR_YELLOW);
    print!("commande> s=sauver  q=quitter  x=sauver+quitter  (autre=annuler)            ");
    vga::set_color(COLOR_DEFAULT);
    match keyboard::read_key() {
        Key::Char(c) => c,
        _ => 0,
    }
}
