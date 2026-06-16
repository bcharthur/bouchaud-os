//! Widgets de rendu du bureau : fenetres, barre des taches, menu, curseur.

use crate::gui::apps;
use crate::gui::framebuffer as fb;
use crate::gui::window::{clip, menu_rect, start_btn, taskbar_btn, Win, BAR_H, MENU, TITLE_H};
use crate::arch::x86_64::rtc;
use alloc::format;

/// Dessine le fond du bureau, la barre du haut et toutes les fenetres visibles.
pub(crate) fn draw_desktop(wins: &[Win]) {
    fb::clear(fb::C_DESKTOP);
    fb::fill_rect(0, fb::HEIGHT / 2, fb::WIDTH, fb::HEIGHT / 2, fb::C_DKBLUE);
    fb::draw_text(fb::WIDTH / 2 - 44, fb::HEIGHT / 2 - 4, "Bouchaud OS", fb::C_DKGRAY);

    fb::fill_rect(0, 0, fb::WIDTH, BAR_H, fb::C_TITLE);
    fb::draw_text(2, 2, "Bouchaud OS", fb::C_WHITE);
    let dt = rtc::now();
    let clk = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    fb::draw_text(fb::WIDTH - clk.len() * 8 - 2, 2, &clk, fb::C_YELLOW);

    let focus = wins.iter().rposition(|w| !w.min);
    for (i, w) in wins.iter().enumerate() {
        if w.min { continue; }
        draw_window(w, Some(i) == focus);
    }
}

fn draw_window(w: &Win, focused: bool) {
    let x = w.x.max(0) as usize;
    let y = w.y.max(0) as usize;
    let ww = w.w as usize;
    let wh = w.h as usize;
    fb::fill_rect(x + 2, y + 2, ww, wh, fb::C_DKGRAY); // ombre
    fb::fill_rect(x, y, ww, wh, fb::C_GRAY);
    fb::rect(x, y, ww, wh, fb::C_WHITE);
    fb::fill_rect(x, y, ww, TITLE_H as usize, if focused { fb::C_BLUE } else { fb::C_DKGRAY });
    fb::draw_text(x + 3, y + 1, clip(&w.title, (ww / 8).saturating_sub(5)), fb::C_WHITE);
    // Boutons de titre : minimiser, maximiser, fermer.
    fb::fill_rect(x + ww - 28, y + 1, 8, 8, fb::C_GRAY);
    fb::draw_text(x + ww - 27, y + 1, "_", fb::C_BLACK);
    fb::fill_rect(x + ww - 19, y + 1, 8, 8, fb::C_GRAY);
    fb::draw_text(x + ww - 18, y + 1, "o", fb::C_BLACK);
    fb::fill_rect(x + ww - 10, y + 1, 8, 8, fb::C_RED);
    fb::draw_text(x + ww - 9, y + 1, "x", fb::C_WHITE);

    apps::draw_app(w);

    // Poignee de redimensionnement (coin bas-droit).
    fb::fill_rect(x + ww - 6, y + wh - 6, 5, 5, fb::C_WHITE);
}

/// Dessine la barre des taches (bouton Demarrer + tuile par fenetre).
pub(crate) fn draw_taskbar(wins: &[Win], menu_open: bool) {
    fb::fill_rect(0, fb::HEIGHT - BAR_H, fb::WIDTH, BAR_H, fb::C_TITLE);
    let sb = start_btn();
    fb::fill_rect(sb.x as usize, sb.y as usize, sb.w as usize, sb.h as usize, if menu_open { fb::C_GREEN } else { fb::C_BLUE });
    fb::draw_text(sb.x as usize + 3, sb.y as usize + 1, "Start", fb::C_WHITE);
    for (i, w) in wins.iter().enumerate() {
        let b = taskbar_btn(i);
        if b.x + b.w > fb::WIDTH as i32 { break; }
        fb::fill_rect(b.x as usize, b.y as usize, b.w as usize, b.h as usize, fb::C_DKGRAY);
        fb::draw_text(b.x as usize + 2, b.y as usize + 1, clip(&w.title, 6), fb::C_WHITE);
    }
}

/// Dessine le menu Demarrer.
pub(crate) fn draw_menu() {
    let mr = menu_rect();
    fb::fill_rect(mr.x as usize, mr.y as usize, mr.w as usize, mr.h as usize, fb::C_GRAY);
    fb::rect(mr.x as usize, mr.y as usize, mr.w as usize, mr.h as usize, fb::C_WHITE);
    for (i, item) in MENU.iter().enumerate() {
        let iy = mr.y as usize + 1 + i * 10;
        let col = if i == MENU.len() - 1 { fb::C_RED } else { fb::C_BLUE };
        fb::draw_text(mr.x as usize + 4, iy + 1, item, col);
    }
}

/// Dessine le curseur souris (fleche 8x8).
pub(crate) fn draw_cursor(mx: usize, my: usize) {
    const CUR: [u8; 8] = [
        0b00000001, 0b00000011, 0b00000111, 0b00001111,
        0b00011111, 0b00000111, 0b00001101, 0b00011000,
    ];
    for (row, bits) in CUR.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                fb::pixel(mx + col, my + row, fb::C_WHITE);
            }
        }
    }
}
