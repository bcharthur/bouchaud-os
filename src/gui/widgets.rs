//! Widgets de rendu du bureau : fenetres, barre des taches, menu, curseur.

use crate::gui::apps;
use crate::gui::framebuffer as fb;
use crate::gui::window::{clip, icon_rect, menu_rect, start_btn, taskbar_btn, Win, BAR_H, ICONS, MENU, TITLE_H};
use crate::arch::x86_64::rtc;
use crate::kernel::timer;
use crate::fs::ramfs;
use alloc::format;
use alloc::string::String;

/// Dessine le fond du bureau, la barre du haut et toutes les fenetres visibles.
pub(crate) fn draw_desktop(wins: &[Win]) {
    draw_wallpaper();
    fb::draw_text_rgb(fb::WIDTH / 2 - 88, fb::HEIGHT - 60, "Bouchaud OS", 0x33476b, 2);

    draw_icons();

    fb::fill_rect(0, 0, fb::WIDTH, BAR_H, fb::C_TITLE);
    fb::draw_text_prop(2, 1, "Bouchaud OS", 0xffffff, 9.0, true);

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

fn human_bytes(n: usize) -> String {
    if n >= 1_073_741_824 { format!("{}Go", n / 1_073_741_824) }
    else if n >= 1_048_576 { format!("{}Mo", n / 1_048_576) }
    else if n >= 1_024    { format!("{}Ko", n / 1_024) }
    else                  { format!("{}o", n) }
}

fn draw_wallpaper() {
    const TOP: (u32, u32, u32) = (0x0b, 0x16, 0x2a);
    const BOT: (u32, u32, u32) = (0x1c, 0x3a, 0x66);
    let h = fb::HEIGHT.max(1);
    let mut y = 0;
    while y < fb::HEIGHT {
        let t = y * 255 / h;
        let r = TOP.0 + (BOT.0 - TOP.0) * t as u32 / 255;
        let g = TOP.1 + (BOT.1 - TOP.1) * t as u32 / 255;
        let b = TOP.2 + (BOT.2 - TOP.2) * t as u32 / 255;
        fb::fill_rect_rgb(0, y, fb::WIDTH, 1, (r << 16) | (g << 8) | b);
        y += 1;
    }
}

fn draw_icons() {
    for i in 0..ICONS.len() {
        let (label, _kind) = ICONS[i];
        let r = icon_rect(i);
        let vw = 40i32;
        let vx = r.x + (r.w - vw) / 2;
        let vy = r.y;
        // Shadow
        fb::fill_rect_rgb((vx + 2) as usize, (vy + 2) as usize, vw as usize, vw as usize, 0x101820);
        draw_app_icon(i, vx as usize, vy as usize, vw as usize);
        // Label (TTF antialiase)
        let lw = fb::text_width(label, 11.0, false) as i32;
        let lx = (r.x + (r.w - lw) / 2).max(0) as usize;
        fb::draw_text_prop(lx, (vy + vw + 3) as usize, label, 0xffffff, 11.0, false);
    }
}

/// Dessine l'icone pixel-art de l'application `kind` dans un carre `vw x vw` en (vx, vy).
fn draw_app_icon(icon_idx: usize, vx: usize, vy: usize, vw: usize) {
    match icon_idx {
        0 => draw_icon_browser(vx, vy, vw),
        1 => draw_icon_calculator(vx, vy, vw),
        2 => draw_icon_terminal(vx, vy, vw),
        3 => draw_icon_files(vx, vy, vw),
        4 => draw_icon_rustpad(vx, vy, vw),
        _ => { fb::fill_rect_rgb(vx, vy, vw, vw, 0x555555); }
    }
}

fn draw_icon_rustpad(vx: usize, vy: usize, vw: usize) {
    // Fond sombre (éditeur de code)
    fb::fill_rect_rgb(vx, vy, vw, vw, 0x0d1117);
    // Fenetre editor (cadre)
    let pad = vw / 8;
    fb::fill_rect_rgb(vx + pad, vy + pad, vw - pad*2, vw - pad*2, 0x161b22);
    // Lignes de code simulées
    let lh = vw / 8; let lx = vx + pad + 3;
    fb::fill_rect_rgb(lx, vy + pad + 3,           vw/2, lh.max(2), 0xff7b72);  // rouge (fn)
    fb::fill_rect_rgb(lx + vw/8, vy + pad + 3 + lh + 2, vw/3, lh.max(2), 0xa5d6ff);  // bleu
    fb::fill_rect_rgb(lx + vw/8, vy + pad + 3 + (lh+2)*2, vw/4, lh.max(2), 0x3fb950);  // vert
    fb::fill_rect_rgb(lx + vw/8, vy + pad + 3 + (lh+2)*3, vw/3, lh.max(2), 0xa5d6ff);
    // Bouton Run ▶ (triangle vert)
    let bx = vx + vw - pad - vw/5; let by = vy + vy.min(4) + vw/3;
    let bsize = vw / 5;
    for row in 0..bsize {
        let w = (row * 2 + 1).min(bsize);
        fb::fill_rect_rgb(bx, by + row, w, 1, 0x238636);
    }
}

fn isqrt(n: i32) -> i32 {
    if n <= 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}

fn draw_icon_browser(vx: usize, vy: usize, vw: usize) {
    fb::fill_rect_rgb(vx, vy, vw, vw, 0x1a73e8);
    let cx = vx + vw / 2;
    let cy = vy + vw / 2;
    let r = (vw / 2) as i32 - 3;
    // Globe outline: draw left and right edges of circle per row
    for dy in 0..vw as i32 {
        let y_off = dy - vw as i32 / 2;
        if y_off.abs() > r { continue; }
        let half_w = isqrt(r * r - y_off * y_off) as usize;
        let left = cx.saturating_sub(half_w);
        let right = cx + half_w;
        let y = vy + dy as usize;
        fb::fill_rect_rgb(left, y, 2, 1, 0xffffff);
        if right + 2 <= vx + vw { fb::fill_rect_rgb(right, y, 2, 1, 0xffffff); }
    }
    // Equator and meridian
    fb::fill_rect_rgb(vx + 3, cy - 1, vw - 6, 2, 0xffffff);
    fb::fill_rect_rgb(cx - 1, vy + 3, 2, vw - 6, 0xffffff);
    // Latitude lines (6px above and below equator)
    let lat_off = 6i32;
    let lat_half = isqrt(r * r - lat_off * lat_off) as usize;
    fb::fill_rect_rgb(cx.saturating_sub(lat_half), cy - lat_off as usize, lat_half * 2, 1, 0xbbd8ff);
    fb::fill_rect_rgb(cx.saturating_sub(lat_half), cy + lat_off as usize, lat_half * 2, 1, 0xbbd8ff);
}

fn draw_icon_calculator(vx: usize, vy: usize, vw: usize) {
    // Light grey background
    fb::fill_rect_rgb(vx, vy, vw, vw, 0xe8e8e8);
    // Dark frame outline
    fb::fill_rect_rgb(vx, vy, vw, 1, 0x555555);
    fb::fill_rect_rgb(vx, vy + vw - 1, vw, 1, 0x555555);
    fb::fill_rect_rgb(vx, vy, 1, vw, 0x555555);
    fb::fill_rect_rgb(vx + vw - 1, vy, 1, vw, 0x555555);
    // Display area
    fb::fill_rect_rgb(vx + 3, vy + 3, vw - 6, 9, 0x1a1a2e);
    fb::draw_text_rgb(vx + 5, vy + 4, "0", 0x00ff88, 1);
    // Button grid 3x4
    let colors = [
        [0xcccccc, 0xcccccc, 0xff5555],
        [0xcccccc, 0xcccccc, 0xcccccc],
        [0xcccccc, 0xcccccc, 0xcccccc],
        [0xcccccc, 0xcccccc, 0x4488ff],
    ];
    for row in 0..4usize {
        for col in 0..3usize {
            let bx = vx + 3 + col * 11;
            let by = vy + 14 + row * 6;
            fb::fill_rect_rgb(bx, by, 9, 5, colors[row][col]);
            fb::fill_rect_rgb(bx, by, 9, 1, 0xffffff);
        }
    }
}

fn draw_icon_terminal(vx: usize, vy: usize, vw: usize) {
    // Dark background
    fb::fill_rect_rgb(vx, vy, vw, vw, 0x0d1117);
    // Title bar
    fb::fill_rect_rgb(vx, vy, vw, 7, 0x21262d);
    // Traffic lights
    fb::fill_rect_rgb(vx + 3, vy + 2, 3, 3, 0xff5f57);
    fb::fill_rect_rgb(vx + 8, vy + 2, 3, 3, 0xffbd2e);
    fb::fill_rect_rgb(vx + 13, vy + 2, 3, 3, 0x28c840);
    // Prompt line
    fb::draw_text_rgb(vx + 2, vy + 9, ">", 0x58a6ff, 1);
    fb::draw_text_rgb(vx + 10, vy + 9, "_", 0xe6edf3, 1);
    // Second line (simulated text)
    fb::fill_rect_rgb(vx + 2, vy + 20, 18, 1, 0x3d4f5c);
    fb::fill_rect_rgb(vx + 2, vy + 23, 12, 1, 0x3d4f5c);
    fb::fill_rect_rgb(vx + 2, vy + 26, 24, 1, 0x3d4f5c);
    fb::fill_rect_rgb(vx + 2, vy + 29, 8, 1, 0x3d4f5c);
    // Cursor blink effect
    fb::fill_rect_rgb(vx + 2, vy + 18, 2, 7, 0x58a6ff);
}

fn draw_icon_files(vx: usize, vy: usize, vw: usize) {
    // Folder body
    let body_y = vy + 8;
    let body_h = vw - 10;
    fb::fill_rect_rgb(vx + 1, body_y, vw - 2, body_h, 0xf9ab00);
    // Folder tab (top-left)
    fb::fill_rect_rgb(vx + 1, vy + 4, 14, 5, 0xf9ab00);
    fb::fill_rect_rgb(vx + 1, vy + 4, 14, 1, 0xffd04f);
    // Top highlight on body
    fb::fill_rect_rgb(vx + 1, body_y, vw - 2, 2, 0xffd04f);
    // Bottom shadow
    fb::fill_rect_rgb(vx + 1, body_y + body_h - 3, vw - 2, 3, 0xc87b00);
    // Document lines inside folder
    fb::fill_rect_rgb(vx + 6, body_y + 5, vw - 14, 2, 0xffffff);
    fb::fill_rect_rgb(vx + 6, body_y + 9, vw - 14, 2, 0xffffff);
    fb::fill_rect_rgb(vx + 6, body_y + 13, (vw - 14) * 2 / 3, 2, 0xffffff);
}

fn draw_window(w: &Win, focused: bool) {
    let x = w.x.max(0) as usize;
    let y = w.y.max(0) as usize;
    let ww = w.w as usize;
    let wh = w.h as usize;
    fb::fill_rect(x + 2, y + 2, ww, wh, fb::C_DKGRAY);
    fb::fill_rect(x, y, ww, wh, fb::C_GRAY);
    fb::rect(x, y, ww, wh, fb::C_WHITE);
    fb::fill_rect(x, y, ww, TITLE_H as usize, if focused { fb::C_BLUE } else { fb::C_DKGRAY });
    fb::draw_text(x + 3, y + 1, clip(&w.title, (ww / 8).saturating_sub(5)), fb::C_WHITE);
    // Boutons titre
    fb::fill_rect(x + ww - 28, y + 1, 8, 8, fb::C_GRAY);
    fb::draw_text(x + ww - 27, y + 1, "_", fb::C_BLACK);
    fb::fill_rect(x + ww - 19, y + 1, 8, 8, fb::C_GRAY);
    fb::draw_text(x + ww - 18, y + 1, "o", fb::C_BLACK);
    fb::fill_rect(x + ww - 10, y + 1, 8, 8, fb::C_RED);
    fb::draw_text(x + ww - 9, y + 1, "x", fb::C_WHITE);

    apps::draw_app(w);

    fb::fill_rect(x + ww - 6, y + wh - 6, 5, 5, fb::C_WHITE);
}

/// Dessine la barre des taches.
pub(crate) fn draw_taskbar(wins: &[Win], menu_open: bool) {
    fb::fill_rect(0, fb::HEIGHT - BAR_H, fb::WIDTH, BAR_H, fb::C_TITLE);
    let sb = start_btn();
    fb::fill_rect(sb.x as usize, sb.y as usize, sb.w as usize, sb.h as usize,
        if menu_open { fb::C_GREEN } else { fb::C_BLUE });
    fb::draw_text_prop(sb.x as usize + 3, sb.y as usize + 1, "Start", 0xffffff, 9.0, true);
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

/// Dessine le curseur souris avec couleur adaptee au fond.
pub(crate) fn draw_cursor(mx: usize, my: usize) {
    const CUR: [u8; 8] = [
        0b00000001, 0b00000011, 0b00000111, 0b00001111,
        0b00011111, 0b00000111, 0b00001101, 0b00011000,
    ];
    let px = mx.min(fb::WIDTH.saturating_sub(1));
    let py = my.min(fb::HEIGHT.saturating_sub(1));
    let bg = fb::get_pixel_rgb(px, py);
    let lum = ((bg >> 16 & 0xff) * 299 + (bg >> 8 & 0xff) * 587 + (bg & 0xff) * 114) / 1000;
    let (fill, outline) = if lum > 140 { (fb::C_BLACK, fb::C_WHITE) } else { (fb::C_WHITE, fb::C_BLACK) };
    // Outline pass (decale de 1 pixel dans les 8 directions)
    for dy in 0usize..10 {
        for dx in 0usize..10 {
            let row = if dy == 0 { 0 } else { dy - 1 };
            let col_off = if dx == 0 { 0 } else { dx - 1 };
            if row < 8 && col_off < 8 && CUR[row] & (1 << col_off) != 0 {
                // outline pixel
            }
        }
    }
    for (row, &bits) in CUR.iter().enumerate() {
        for col in 0..8usize {
            if bits & (1 << col) != 0 {
                // Draw 1-pixel outline around each cursor pixel
                if mx + col < fb::WIDTH && my + row < fb::HEIGHT {
                    if col > 0 && bits & (1 << (col - 1)) == 0 {
                        fb::pixel(mx + col - 1, my + row, outline);
                    }
                    if row > 0 && CUR[row - 1] & (1 << col) == 0 {
                        fb::pixel(mx + col, my + row - 1, outline);
                    }
                }
            }
        }
    }
    for (row, &bits) in CUR.iter().enumerate() {
        for col in 0..8usize {
            if bits & (1 << col) != 0 {
                fb::pixel(mx + col, my + row, fill);
            }
        }
    }
}
