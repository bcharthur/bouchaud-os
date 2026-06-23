//! Widgets de rendu du bureau : fenetres, barre des taches, menu, curseur.

use crate::gui::apps;
use crate::gui::framebuffer as fb;
use crate::gui::window::{clip, icon_rect, menu_rect, start_btn, taskbar_btn, Win, BAR_H, ICONS, MENU, TITLE_H};
use crate::arch::x86_64::rtc;
use crate::kernel::timer;
use crate::fs::ramfs;
use alloc::format;
use alloc::string::String;

// Accent + glyphe par icone de bureau (meme ordre que window::ICONS).
const ICON_STYLE: [(u32, &str); 4] = [
    (0x1a73e8, "N"),   // Nautile
    (0x34a853, "="),   // Calculatrice
    (0x202124, ">_"),  // Terminal
    (0xf9ab00, "[]"),  // Fichiers
];

/// Dessine le fond du bureau, la barre du haut et toutes les fenetres visibles.
pub(crate) fn draw_desktop(wins: &[Win]) {
    draw_wallpaper();
    fb::draw_text_rgb(fb::WIDTH / 2 - 88, fb::HEIGHT - 60, "Bouchaud OS", 0x33476b, 2);

    draw_icons();

    fb::fill_rect(0, 0, fb::WIDTH, BAR_H, fb::C_TITLE);
    fb::draw_text(2, 2, "Bouchaud OS", fb::C_WHITE);

    // Stats système au centre de la barre.
    let stats = sys_stats_str();
    let sw = stats.len() * 8;
    let sx = (fb::WIDTH / 2).saturating_sub(sw / 2);
    fb::draw_text(sx, 2, &stats, fb::C_CYAN);

    let dt = rtc::now();
    let clk = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    fb::draw_text(fb::WIDTH - clk.len() * 8 - 2, 2, &clk, fb::C_YELLOW);

    let focus = wins.iter().rposition(|w| !w.min);
    for (i, w) in wins.iter().enumerate() {
        if w.min { continue; }
        draw_window(w, Some(i) == focus);
    }
}

// Formate les statistiques système pour la barre du haut.
fn sys_stats_str() -> String {
    let cpu = timer::cpu_load_pct();

    let (used, _free, total) = crate::kernel::heap::stats();
    let ram_pct = if total > 0 { (used * 100 / total) as u8 } else { 0 };
    let ram_used_str = human_bytes(used);
    let ram_total_str = human_bytes(total);

    let fs = ramfs::fs();
    let disk_used = fs.used_nodes();
    let disk_total = crate::fs::ramfs::MAX_NODES;
    let disk_pct = if disk_total > 0 { (disk_used * 100 / disk_total) as u8 } else { 0 };

    format!(
        "CPU:{cpu:3}%  RAM:{ram_used_str}/{ram_total_str} {ram_pct:3}%  Disk:{disk_used}/{disk_total} {disk_pct:3}%"
    )
}

// Formate un nombre d'octets en B/Ko/Mo/Go lisible.
fn human_bytes(n: usize) -> String {
    if n >= 1_073_741_824 {
        format!("{}Go", n / 1_073_741_824)
    } else if n >= 1_048_576 {
        format!("{}Mo", n / 1_048_576)
    } else if n >= 1_024 {
        format!("{}Ko", n / 1_024)
    } else {
        format!("{}o", n)
    }
}

// Degrade vertical bleu nuit -> bleu (fond de bureau).
fn draw_wallpaper() {
    const TOP: (u32, u32, u32) = (0x0b, 0x16, 0x2a);
    const BOT: (u32, u32, u32) = (0x1c, 0x3a, 0x66);
    let h = fb::HEIGHT.max(1);
    let mut y = 0;
    while y < fb::HEIGHT {
        let t = y * 255 / h; // 0..255
        let r = TOP.0 + (BOT.0 - TOP.0) * t as u32 / 255;
        let g = TOP.1 + (BOT.1 - TOP.1) * t as u32 / 255;
        let b = TOP.2 + (BOT.2 - TOP.2) * t as u32 / 255;
        fb::fill_rect_rgb(0, y, fb::WIDTH, 1, (r << 16) | (g << 8) | b);
        y += 1;
    }
}

/// Dessine les icones de lancement sur le bureau.
fn draw_icons() {
    for (i, (label, _kind)) in ICONS.iter().enumerate() {
        let r = icon_rect(i);
        let (accent, glyph) = ICON_STYLE[i];
        // Vignette 40x40 centree dans la largeur de l'icone.
        let vw = 40i32;
        let vx = r.x + (r.w - vw) / 2;
        let vy = r.y;
        fb::fill_rect_rgb((vx + 2) as usize, (vy + 2) as usize, vw as usize, vw as usize, 0x101820); // ombre douce
        fb::fill_rect_rgb(vx as usize, vy as usize, vw as usize, vw as usize, accent);
        // Liseré clair.
        fb::fill_rect_rgb(vx as usize, vy as usize, vw as usize, 1, 0xffffff);
        // Glyphe centre (scale 2).
        let gw = glyph.len() as i32 * 16;
        fb::draw_text_rgb((vx + (vw - gw) / 2).max(0) as usize, (vy + 12) as usize, glyph, 0xffffff, 2);
        // Etiquette sous la vignette (police vectorielle antialiasée).
        let lw = fb::text_width(label, 12.0, false) as i32;
        fb::draw_text_prop(((r.x + (r.w - lw) / 2).max(0)) as usize, (vy + vw + 3) as usize, label, 0xffffff, 12.0, false);
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
