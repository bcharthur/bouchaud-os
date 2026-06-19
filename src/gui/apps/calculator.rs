//! Application Calculatrice native.
//!
//! Interface dessinee au pixel (affichage + grille de touches), evaluation des
//! expressions par le moteur de langage embarque de l'OS (`gui::js`). Demontre
//! le systeme d'execution d'applications : une appl native qui delegue le calcul
//! a l'interpreteur JavaScript integre.

use crate::gui::framebuffer as fb;
use alloc::string::String;

// Disposition de la grille : 4 colonnes x 5 lignes (ordre lecture).
// "<" = effacement arriere, "C" = remise a zero, "=" = evaluation.
pub(crate) const KEYS: [&str; 20] = [
    "C", "(", ")", "/",
    "7", "8", "9", "*",
    "4", "5", "6", "-",
    "1", "2", "3", "+",
    "0", ".", "=", "<",
];

const PAD: i32 = 4;
const DISP_H: i32 = 34;

// Couleurs (truecolor).
const BG: u32 = 0x1f2023;
const DISP_BG: u32 = 0x0f1011;
const DISP_FG: u32 = 0x8ab4f8;
const KEY_BG: u32 = 0x3c4043;
const KEY_FG: u32 = 0xe8eaed;
const OP_FG: u32 = 0xfdd663;     // operateurs
const EQ_BG: u32 = 0x1a73e8;     // touche =
const CLR_FG: u32 = 0xff6b6b;    // touche C

// Geometrie d'une touche (col,row) dans le corps de fenetre (coords absolues).
fn cell_rect(idx: usize, bx: i32, by: i32, bw: i32, bh: i32) -> (i32, i32, i32, i32) {
    let col = (idx % 4) as i32;
    let row = (idx / 4) as i32;
    let grid_top = by + DISP_H + PAD;
    let avail_h = (by + bh) - grid_top;
    let cw = (bw - PAD * 5) / 4;
    let ch = (avail_h - PAD * 6) / 5;
    let x = bx + PAD + col * (cw + PAD);
    let y = grid_top + PAD + row * (ch + PAD);
    (x, y, cw.max(1), ch.max(1))
}

fn is_op(label: &str) -> bool {
    matches!(label, "/" | "*" | "-" | "+")
}

/// Applique l'appui sur une touche a l'expression courante.
pub(crate) fn apply_key(expr: &mut String, label: &str) {
    match label {
        "C" => expr.clear(),
        "<" => { expr.pop(); }
        "=" => {
            let src = if expr.is_empty() { "0" } else { expr.as_str() };
            *expr = match crate::gui::js::eval_expr(src) {
                Ok(r) => r,
                Err(_) => String::from("Erreur"),
            };
        }
        _ => {
            if expr == "Erreur" { expr.clear(); }
            if expr.len() < 64 { expr.push_str(label); }
        }
    }
}

/// Traduit une touche du clavier physique vers une touche calculatrice.
pub(crate) fn key_char(c: char) -> Option<&'static str> {
    Some(match c {
        '0' => "0", '1' => "1", '2' => "2", '3' => "3", '4' => "4",
        '5' => "5", '6' => "6", '7' => "7", '8' => "8", '9' => "9",
        '+' => "+", '-' => "-", '*' => "*", '/' => "/",
        '.' | ',' => ".", '(' => "(", ')' => ")",
        '=' => "=", 'c' | 'C' => "C",
        _ => return None,
    })
}

/// Clic dans le corps : renvoie l'eventuelle touche touchee.
pub(crate) fn key_at(bx: i32, by: i32, bw: i32, bh: i32, mx: i32, my: i32) -> Option<&'static str> {
    for (i, label) in KEYS.iter().enumerate() {
        let (x, y, w, h) = cell_rect(i, bx, by, bw, bh);
        if mx >= x && mx < x + w && my >= y && my < y + h {
            return Some(label);
        }
    }
    None
}

// Cadre de 1px (truecolor).
fn frame(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w <= 0 || h <= 0 { return; }
    let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
    fb::fill_rect_rgb(x, y, w, 1, c);
    fb::fill_rect_rgb(x, y + h - 1, w, 1, c);
    fb::fill_rect_rgb(x, y, 1, h, c);
    fb::fill_rect_rgb(x + w - 1, y, 1, h, c);
}

/// Dessine la calculatrice dans le corps de fenetre.
pub(crate) fn draw(expr: &str, bx: usize, by: usize, bw: usize, bh: usize) {
    let (bxi, byi, bwi, bhi) = (bx as i32, by as i32, bw as i32, bh as i32);
    fb::fill_rect_rgb(bx, by, bw, bh, BG);

    // Affichage (aligne a droite).
    fb::fill_rect_rgb(bx + PAD as usize, by + PAD as usize,
                      bw.saturating_sub(2 * PAD as usize), (DISP_H - PAD) as usize, DISP_BG);
    let shown = if expr.is_empty() { "0" } else { expr };
    // Choisit une echelle qui tient dans la largeur de l'affichage.
    let inner_w = bwi - 2 * PAD - 8;
    let mut scale = 3usize;
    while scale > 1 && (shown.chars().count() as i32) * 8 * scale as i32 > inner_w { scale -= 1; }
    let tw = (shown.chars().count() as i32) * 8 * scale as i32;
    let tx = (bxi + bwi - PAD - 4 - tw).max(bxi + PAD + 2);
    let ty = byi + (DISP_H - PAD - 8 * scale as i32) / 2 + 1;
    fb::draw_text_rgb(tx.max(0) as usize, ty.max(0) as usize, shown, DISP_FG, scale);

    // Touches.
    for (i, label) in KEYS.iter().enumerate() {
        let (x, y, w, h) = cell_rect(i, bxi, byi, bwi, bhi);
        let bg = if *label == "=" { EQ_BG } else { KEY_BG };
        let fg = if *label == "=" { 0xffffff }
                 else if *label == "C" || *label == "<" { CLR_FG }
                 else if is_op(label) { OP_FG }
                 else { KEY_FG };
        fb::fill_rect_rgb(x as usize, y as usize, w as usize, h as usize, bg);
        frame(x, y, w, h, 0x55585c);
        let glyph = if *label == "<" { "<x" } else { *label };
        let sc = 2usize;
        let gw = glyph.chars().count() as i32 * 8 * sc as i32;
        let gx = x + (w - gw) / 2;
        let gy = y + (h - 8 * sc as i32) / 2;
        fb::draw_text_rgb(gx.max(0) as usize, gy.max(0) as usize, glyph, fg, sc);
    }
}
