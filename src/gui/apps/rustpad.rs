//! Application Rustpad — éditeur de code Rust + interpréteur embarqué.
//!
//! Permet d'écrire et d'exécuter des programmes Rust directement dans l'OS
//! sans compilateur externe, grâce à l'interpréteur `lang::mini_rust`.
//!
//! Contrôles :
//!   - Taper du texte  : édite la ligne courante
//!   - Entrée          : valide la ligne
//!   - Backspace       : efface le dernier caractère
//!   - Tab             : exécute le programme (Run ▶)
//!   - Haut/Bas        : fait défiler l'affichage
//!   - Ctrl (touche C) : efface le code (reset)

use crate::gui::framebuffer as fb;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ─── Couleurs ─────────────────────────────────────────────────────────────────

const BG_CODE:    u32 = 0x0d1117;   // fond éditeur (GitHub dark)
const BG_OUTPUT:  u32 = 0x161b22;   // fond sortie
const BG_TOOLBAR: u32 = 0x21262d;   // barre outil
const C_KEYWORD:  u32 = 0xff7b72;   // rouge (mots-clés)
const C_STRING:   u32 = 0xa5d6ff;   // bleu clair (chaînes)
const C_COMMENT:  u32 = 0x8b949e;   // gris (commentaires)
const C_MACRO:    u32 = 0xd2a8ff;   // violet (macros)
const C_NUMBER:   u32 = 0x79c0ff;   // bleu (nombres)
const C_DEFAULT:  u32 = 0xe6edf3;   // blanc cassé (texte normal)
const C_LINE_NUM: u32 = 0x484f58;   // numéros de ligne
const C_CURSOR:   u32 = 0x388bfd;   // curseur
const C_OUTPUT:   u32 = 0x3fb950;   // sortie standard (vert)
const C_ERROR:    u32 = 0xf85149;   // erreur (rouge)
const C_RUN_BTN:  u32 = 0x238636;   // bouton Run
const C_RUN_LBL:  u32 = 0xffffff;
const C_RESET:    u32 = 0x6e7681;

// ─── Programme Hello World par défaut ────────────────────────────────────────

pub(crate) const DEFAULT_CODE: &str = "\
fn main() {
    println!(\"Bonjour depuis Bouchaud OS !\");

    let n = 7;
    for i in 0..n {
        println!(\"  i = {}\", i);
    }

    let msg = saluer(\"Monde\");
    println!(\"{}\", msg);
}

fn saluer(nom: &str) -> String {
    format!(\"Bonjour, {} !\", nom)
}
";

// ─── État de l'application ────────────────────────────────────────────────────

pub(crate) struct RustpadState {
    pub code_lines: Vec<String>,   // lignes de code source
    pub cur_line:   String,        // ligne en cours de saisie
    pub output:     Vec<String>,   // sortie d'exécution
    pub has_error:  bool,          // dernière exécution en erreur
    pub scroll:     i32,           // défilement vertical (lignes)
    pub mode:       RustpadMode,
}

#[derive(PartialEq)]
pub(crate) enum RustpadMode {
    Edit,    // mode édition du code
    Output,  // affichage de la sortie
}

impl RustpadState {
    pub fn new() -> Self {
        let code_lines: Vec<String> = DEFAULT_CODE.lines().map(|l| l.to_string()).collect();
        RustpadState {
            code_lines,
            cur_line: String::new(),
            output:   Vec::new(),
            has_error: false,
            scroll:   0,
            mode:     RustpadMode::Edit,
        }
    }

    pub fn full_source(&self) -> String {
        let mut s = String::new();
        for line in &self.code_lines { s.push_str(line); s.push('\n'); }
        if !self.cur_line.is_empty() { s.push_str(&self.cur_line); }
        s
    }

    pub fn run_code(&mut self) {
        let src = self.full_source();
        let (stdout, err) = crate::lang::mini_rust::run(&src);
        self.output.clear();
        for line in stdout.lines() { self.output.push(line.to_string()); }
        if let Some(e) = err {
            self.output.push(format!("Erreur : {}", e));
            self.has_error = true;
        } else {
            self.has_error = false;
            if self.output.is_empty() { self.output.push("(aucune sortie)".into()); }
        }
        self.mode = RustpadMode::Output;
        self.scroll = 0;
    }

    pub fn reset(&mut self) {
        let code_lines: Vec<String> = DEFAULT_CODE.lines().map(|l| l.to_string()).collect();
        self.code_lines = code_lines;
        self.cur_line = String::new();
        self.output.clear();
        self.has_error = false;
        self.scroll = 0;
        self.mode = RustpadMode::Edit;
    }
}

// ─── Rendu ────────────────────────────────────────────────────────────────────

pub(crate) fn draw(st: &RustpadState, bx: usize, by: usize, bw: usize, bh: usize) {
    if bw < 20 || bh < 20 { return; }

    // ── Barre d'outils ──────────────────────────────────────────────────────
    let tb_h = 11usize;
    fb::fill_rect_rgb(bx, by, bw, tb_h, BG_TOOLBAR);

    // Bouton Run ▶
    let btn_w = 50usize;
    let btn_x = bx + 2;
    let btn_y = by + 1;
    fb::fill_rect_rgb(btn_x, btn_y, btn_w, 9, C_RUN_BTN);
    fb::draw_text_rgb(btn_x + 2, btn_y + 1, "Tab=Run", C_RUN_LBL, 1);

    // Bouton Reset
    let rst_x = btn_x + btn_w + 4;
    fb::fill_rect_rgb(rst_x, btn_y, 42, 9, C_RESET);
    fb::draw_text_rgb(rst_x + 2, btn_y + 1, "C=Reset", C_DEFAULT, 1);

    // Label mode
    let mode_lbl = if st.mode == RustpadMode::Edit { "[ EDIT ]" } else { "[ OUT  ]" };
    fb::draw_text_rgb(bx + bw - mode_lbl.len() * 8 - 2, btn_y + 1, mode_lbl, C_COMMENT, 1);

    // Séparateur
    let sep_y = by + tb_h;
    fb::fill_rect_rgb(bx, sep_y, bw, 1, 0x30363d);

    let content_y = sep_y + 1;
    let content_h = bh.saturating_sub(tb_h + 1);

    if st.mode == RustpadMode::Edit {
        draw_editor(st, bx, content_y, bw, content_h);
    } else {
        draw_output(st, bx, content_y, bw, content_h);
    }
}

fn draw_editor(st: &RustpadState, bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect_rgb(bx, by, bw, bh, BG_CODE);

    let char_h = 8usize;
    let line_num_w = 28usize;   // largeur des numéros de ligne (3 chiffres + espace)
    let code_x = bx + line_num_w;
    let code_w = bw.saturating_sub(line_num_w);
    let cols = code_w / 8;

    let visible_lines = bh / char_h;
    let scroll = st.scroll.max(0) as usize;

    let total_lines = st.code_lines.len() + 1; // +1 pour cur_line
    let _ = total_lines;

    for row in 0..visible_lines {
        let line_idx = scroll + row;
        let yy = by + row * char_h;

        // Numéro de ligne
        if line_idx < 9999 {
            let num_str = fmt_line_num(line_idx + 1);
            fb::draw_text_rgb(bx + 2, yy, &num_str, C_LINE_NUM, 1);
        }

        // Contenu
        let line = if line_idx < st.code_lines.len() {
            &st.code_lines[line_idx]
        } else if line_idx == st.code_lines.len() {
            &st.cur_line
        } else {
            break;
        };

        // Coloration syntaxique simple
        draw_highlighted_line(line, code_x, yy, cols);

        // Curseur sur la dernière ligne
        if line_idx == st.code_lines.len() {
            let cx = code_x + (line.len().min(cols - 1)) * 8;
            fb::fill_rect_rgb(cx, yy, 2, 7, C_CURSOR);
        }
    }
}

fn draw_output(st: &RustpadState, bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect_rgb(bx, by, bw, bh, BG_OUTPUT);

    let char_h = 8usize;
    let cols = bw / 8;
    let visible = bh / char_h;
    let scroll = st.scroll.max(0) as usize;

    // En-tête
    fb::draw_text_rgb(bx + 2, by + 1, "-- Sortie du programme --", C_COMMENT, 1);
    let content_y = by + char_h + 2;
    let visible = visible.saturating_sub(2);

    let color = if st.has_error { C_ERROR } else { C_OUTPUT };

    for row in 0..visible {
        let idx = scroll + row;
        if idx >= st.output.len() { break; }
        let yy = content_y + row * char_h;
        let line = &st.output[idx];
        let col = if line.starts_with("Erreur") { C_ERROR } else { color };
        let clipped = if line.len() > cols { &line[..cols] } else { line.as_str() };
        fb::draw_text_rgb(bx + 2, yy, clipped, col, 1);
    }

    // Indicateur "Entrée = retour à l'édition"
    let hint_y = by + bh.saturating_sub(9);
    fb::draw_text_rgb(bx + 2, hint_y, "Entree=retour edition", C_COMMENT, 1);
}

// ─── Coloration syntaxique ────────────────────────────────────────────────────

fn draw_highlighted_line(line: &str, x: usize, y: usize, cols: usize) {
    let b = line.as_bytes();
    let n = b.len().min(cols * 4); // limite large pour éviter OOB
    let mut i = 0usize;
    let mut cx = x;

    while i < n && (cx - x) / 8 < cols {
        // Commentaire //
        if i + 1 < b.len() && b[i] == b'/' && b[i+1] == b'/' {
            let rest = &line[i..];
            let trunc = rest.len().min((cols - (cx - x) / 8) * 4);
            fb::draw_text_rgb(cx, y, &rest[..trunc.min(rest.len())], C_COMMENT, 1);
            break;
        }
        // Chaîne littérale
        if b[i] == b'"' {
            let start = i; i += 1;
            while i < b.len() && (b[i] != b'"' || (i > 0 && b[i-1] == b'\\')) { i += 1; }
            if i < b.len() { i += 1; }
            let s = &line[start..i.min(line.len())];
            let trunc = s.len().min((cols - (cx - x) / 8) * 4);
            fb::draw_text_rgb(cx, y, &s[..trunc.min(s.len())], C_STRING, 1);
            cx += trunc * 8;
            continue;
        }
        // Identifiant / mot-clé / macro
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
            let is_macro = i < b.len() && b[i] == b'!';
            if is_macro { i += 1; }
            let word = &line[start..i.min(line.len())];
            let color = if is_macro { C_MACRO }
                else { keyword_color(if is_macro { &word[..word.len()-1] } else { word }) };
            let trunc = word.len().min((cols - (cx - x) / 8) * 4);
            fb::draw_text_rgb(cx, y, &word[..trunc.min(word.len())], color, 1);
            cx += trunc * 8;
            continue;
        }
        // Nombre
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') { i += 1; }
            let word = &line[start..i.min(line.len())];
            let trunc = word.len().min((cols - (cx - x) / 8) * 4);
            fb::draw_text_rgb(cx, y, &word[..trunc.min(word.len())], C_NUMBER, 1);
            cx += trunc * 8;
            continue;
        }
        // Caractère quelconque
        let ch = b[i] as char;
        let mut tmp = [0u8; 4];
        let s = ch.encode_utf8(&mut tmp);
        fb::draw_text_rgb(cx, y, s, C_DEFAULT, 1);
        cx += 8;
        i += 1;
    }
}

fn keyword_color(word: &str) -> u32 {
    match word {
        "fn" | "let" | "mut" | "if" | "else" | "while" | "for" | "in" | "return" |
        "break" | "continue" | "loop" | "pub" | "use" | "struct" | "impl" | "mod" |
        "match" | "enum" | "type" | "trait" | "where" | "self" | "Self" |
        "const" | "static" | "async" | "await" | "move" | "ref" | "dyn" | "as" => C_KEYWORD,
        "true" | "false" => C_NUMBER,
        _ => C_DEFAULT,
    }
}

fn fmt_line_num(n: usize) -> String {
    if n < 10   { alloc::format!("  {}", n) }
    else if n < 100 { alloc::format!(" {}", n) }
    else            { alloc::format!("{}", n) }
}

// ─── Interaction clavier ──────────────────────────────────────────────────────

/// Gère une touche clavier dans le Rustpad. Renvoie `true` si l'app doit fermer.
pub(crate) fn on_key(st: &mut RustpadState, k: crate::gui::event::Key) -> bool {
    use crate::gui::event::Key;
    match (&st.mode, k) {
        // ── Mode sortie ──────────────────────────────────────────────────────
        (RustpadMode::Output, Key::Enter) | (RustpadMode::Output, Key::Tab) => {
            st.mode = RustpadMode::Edit;
            st.scroll = 0;
        }
        (RustpadMode::Output, Key::Up)   => { st.scroll = (st.scroll - 1).max(0); }
        (RustpadMode::Output, Key::Down) => { st.scroll += 1; }

        // ── Mode édition ─────────────────────────────────────────────────────
        (RustpadMode::Edit, Key::Tab) => { st.run_code(); }
        (RustpadMode::Edit, Key::Enter) => {
            let line = core::mem::take(&mut st.cur_line);
            st.code_lines.push(line);
            // Auto-indentation : reprend l'indentation de la ligne précédente
            if let Some(prev) = st.code_lines.iter().rev().nth(1) {
                let indent: String = prev.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                // Ajoute un niveau si la ligne se termine par {
                if prev.trim_end().ends_with('{') {
                    st.cur_line = indent + "    ";
                } else {
                    st.cur_line = indent;
                }
            }
            // Ajuste le scroll pour voir la ligne courante
            let total = st.code_lines.len() + 1;
            st.scroll = (total as i32 - 20).max(0);
        }
        (RustpadMode::Edit, Key::Backspace) => {
            if st.cur_line.pop().is_none() {
                // Fusionne avec la ligne précédente
                if let Some(prev) = st.code_lines.pop() {
                    st.cur_line = prev;
                }
                let total = st.code_lines.len() as i32;
                st.scroll = (total - 20).max(0);
            }
        }
        (RustpadMode::Edit, Key::Up)   => { st.scroll = (st.scroll - 1).max(0); }
        (RustpadMode::Edit, Key::Down) => { st.scroll += 1; }
        (RustpadMode::Edit, Key::Char(c)) => {
            if c == b'c' || c == b'C' {
                // Ctrl simulé par 'C' en majuscule (Shift+c) — reset
                // En pratique, l'utilisateur peut écrire 'C' normalement.
                // On fait le reset uniquement si la ligne courante est vide (pas en train d'écrire).
                if st.cur_line.is_empty() && st.code_lines.is_empty() {
                    st.reset();
                } else {
                    if st.cur_line.len() < 512 { st.cur_line.push(c as char); }
                }
            } else {
                if st.cur_line.len() < 512 { st.cur_line.push(c as char); }
            }
        }
        _ => {}
    }
    false
}

/// Défilement molette.
pub(crate) fn on_wheel(st: &mut RustpadState, delta: i32) {
    st.scroll = (st.scroll - delta).max(0);
}
