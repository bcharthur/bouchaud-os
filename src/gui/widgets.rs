//! Widgets de rendu du bureau : fenetres, barre des taches, menu, curseur.

use crate::gui::apps;
use crate::gui::framebuffer as fb;
use crate::gui::window::{
    clip, icon_rect, menu_rect, start_btn, taskbar_btn, Win,
    BAR_H, ICONS, MENU, MENU_HEADER_H, MENU_ITEM_H, TITLE_H,
};
use crate::arch::x86_64::rtc;
use crate::kernel::timer;
use crate::fs::ramfs;
use alloc::format;
use alloc::string::String;

// ─── Utilitaires couleur ───────────────────────────────────────────────────

fn lerp_color(c1: u32, c2: u32, t: usize, max: usize) -> u32 {
    let m = max.max(1) as i32;
    let t = t as i32;
    let ch = |shift: u32| -> u32 {
        let a = ((c1 >> shift) & 0xff) as i32;
        let b = ((c2 >> shift) & 0xff) as i32;
        ((a + (b - a) * t / m).clamp(0, 255)) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

fn draw_circle(cx: usize, cy: usize, r: i32, color: u32) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx*dx + dy*dy <= r*r {
                let px = cx as i32 + dx;
                let py = cy as i32 + dy;
                if px >= 0 && py >= 0 && (px as usize) < fb::WIDTH && (py as usize) < fb::HEIGHT {
                    fb::pixel_rgb(px as usize, py as usize, color);
                }
            }
        }
    }
}

fn draw_circle_highlight(cx: usize, cy: usize, r: i32, base: u32) {
    draw_circle(cx, cy, r, base);
    // Top highlight arc
    for dx in -(r-1)..=(r-1) {
        let dy = -r + 1;
        let px = cx as i32 + dx; let py = cy as i32 + dy;
        if px >= 0 && py >= 0 && (px as usize) < fb::WIDTH && (py as usize) < fb::HEIGHT {
            fb::pixel_rgb(px as usize, py as usize, lerp_color(base, 0xffffff, 60, 100));
        }
    }
}

// ─── Bureau ────────────────────────────────────────────────────────────────

/// Dessine le fond du bureau, la barre du haut et toutes les fenetres visibles.
pub(crate) fn draw_desktop(wins: &[Win]) {
    draw_wallpaper();

    // Filigrane "Bouchaud OS" centré en bas
    fb::draw_text_rgb(fb::WIDTH / 2 - 88, fb::HEIGHT - 60, "Bouchaud OS", 0x33476b, 2);

    draw_icons();
    draw_topbar();

    let focus = wins.iter().rposition(|w| !w.min);
    for (i, w) in wins.iter().enumerate() {
        if w.min { continue; }
        draw_window(w, Some(i) == focus);
    }
}

fn draw_topbar() {
    // Fond gradient vertical foncé
    for y in 0..BAR_H {
        let c = lerp_color(0x0d1a30, 0x162340, y, BAR_H);
        fb::fill_rect_rgb(0, y, fb::WIDTH, 1, c);
    }
    // Ligne de séparation en bas
    fb::fill_rect_rgb(0, BAR_H - 1, fb::WIDTH, 1, 0x2a4580);

    // "Bouchaud OS" à gauche en TTF gras
    fb::draw_text_prop(4, 1, "Bouchaud OS", 0x7ab8f5, 9.0, true);

    // Stats CPU/RAM/Disk au centre
    let stats = sys_stats_str();
    let sw = fb::text_width(&stats, 9.0, false);
    let sx = (fb::WIDTH / 2).saturating_sub(sw / 2);
    fb::draw_text_prop(sx, 1, &stats, 0x5ab4d6, 9.0, false);

    // Horloge à droite
    let dt = rtc::now();
    let clk = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    let cw = fb::text_width(&clk, 9.0, true);
    fb::draw_text_prop(fb::WIDTH - cw - 4, 1, &clk, 0xf0c060, 9.0, true);
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
    // Gradient bleu nuit du haut (profond) vers le bas (moins foncé)
    let h = fb::HEIGHT.max(1);
    for y in 0..fb::HEIGHT {
        let c = lerp_color(0x080e1c, 0x1a2f50, y, h);
        fb::fill_rect_rgb(0, y, fb::WIDTH, 1, c);
    }
    // Subtiles étoiles (pixels clairs fixes, déterministes)
    let stars: &[(usize, usize)] = &[
        (120, 80), (340, 45), (600, 130), (780, 60), (1000, 90),
        (200, 200), (500, 180), (850, 220), (1100, 150), (1200, 300),
        (50, 350), (420, 400), (700, 380), (950, 420), (1150, 500),
    ];
    for &(sx, sy) in stars {
        if sx < fb::WIDTH && sy < fb::HEIGHT {
            fb::pixel_rgb(sx, sy, 0x4a6fa5);
        }
    }
}

// ─── Icônes ────────────────────────────────────────────────────────────────

fn draw_icons() {
    for i in 0..ICONS.len() {
        let (label, _kind) = ICONS[i];
        let r = icon_rect(i);
        let vw = 40i32;
        let vx = r.x + (r.w - vw) / 2;
        let vy = r.y;

        // Ombre portée
        fb::fill_rect_rgb((vx + 3) as usize, (vy + 3) as usize, vw as usize, vw as usize, 0x06090f);

        // Fond de l'icône (carré arrondi simulé)
        draw_app_icon(i, vx as usize, vy as usize, vw as usize);

        // Halo de sélection (cadre bleu subtil)
        // fb::rect(vx as usize, vy as usize, vw as usize, vw as usize, fb::C_DKBLUE);

        // Label TTF antialiasé avec ombre
        let lw = fb::text_width(label, 10.0, false) as i32;
        let lx = (r.x + (r.w - lw) / 2).max(0) as usize;
        let ly = (vy + vw + 3) as usize;
        // Ombre du texte
        fb::draw_text_prop(lx + 1, ly + 1, label, 0x000000, 10.0, false);
        fb::draw_text_prop(lx, ly, label, 0xe8f4fd, 10.0, false);
    }
}

/// Dessine l'icone pixel-art de l'application `kind` dans un carre `vw x vw` en (vx, vy).
fn draw_app_icon(icon_idx: usize, vx: usize, vy: usize, vw: usize) {
    match icon_idx {
        0 => draw_icon_nautile(vx, vy, vw),
        1 => draw_icon_calculator(vx, vy, vw),
        2 => draw_icon_terminal(vx, vy, vw),
        3 => draw_icon_files(vx, vy, vw),
        4 => draw_icon_rustpad(vx, vy, vw),
        _ => { fb::fill_rect_rgb(vx, vy, vw, vw, 0x555555); }
    }
}

/// Remplit un disque plein (cx, cy = coordonnees ecran) clippé dans la zone icone.
fn fill_circle(scx: i32, scy: i32, r: i32, col: u32, clip_x: usize, clip_y: usize, clip_w: usize) {
    if r <= 0 { return; }
    for dy in -r..=r {
        let hw = isqrt(r * r - dy * dy);
        let y = scy + dy;
        if y < clip_y as i32 || y >= (clip_y + clip_w) as i32 { continue; }
        let x0 = (scx - hw).max(clip_x as i32);
        let x1 = (scx + hw).min((clip_x + clip_w - 1) as i32);
        if x0 <= x1 {
            fb::fill_rect_rgb(x0 as usize, y as usize, (x1 - x0 + 1) as usize, 1, col);
        }
    }
}

/// Logo Nautile Navigateur : coquille nautile en pixel art (spirale logarithmique simplifiee).
fn draw_icon_nautile(vx: usize, vy: usize, vw: usize) {
    // Fond : bleu ocean profond
    fb::fill_rect_rgb(vx, vy, vw, vw, 0x081525);

    let vwi = vw as i32;
    let cx  = vx as i32 + vwi / 2;
    let cy  = vy as i32 + vwi / 2;
    let r   = vwi / 2 - 2;

    // Couche 1 : coque externe (or creme)
    fill_circle(cx, cy, r, 0xf5c040, vx, vy, vw);
    // Separateur de chambre (bord fonce)
    fill_circle(cx - r / 5, cy + r / 5, r * 7 / 10 + 1, 0x050f1c, vx, vy, vw);
    // Chambre 2 : orange dore
    fill_circle(cx - r / 5, cy + r / 5, r * 7 / 10, 0xe09010, vx, vy, vw);
    // Separateur
    fill_circle(cx - r * 2 / 5, cy + r * 2 / 5, r / 2 + 1, 0x050f1c, vx, vy, vw);
    // Chambre 3 : orange roux
    fill_circle(cx - r * 2 / 5, cy + r * 2 / 5, r / 2, 0xb06808, vx, vy, vw);
    // Separateur
    fill_circle(cx - r * 3 / 5, cy + r * 3 / 5, r / 4 + 1, 0x050f1c, vx, vy, vw);
    // Chambre 4 : brun dore (coeur)
    fill_circle(cx - r * 3 / 5, cy + r * 3 / 5, r / 4, 0x7a3a06, vx, vy, vw);
    // Oeil central (trou)
    fill_circle(cx - r * 3 / 4, cy + r * 3 / 4, r / 7 + 1, 0x030a12, vx, vy, vw);
    // Reflet blanc en haut a gauche de la coque
    fill_circle(cx - r / 2, cy - r / 2, r / 6, 0xfff5d0, vx, vy, vw);
}

fn isqrt(n: i32) -> i32 {
    if n <= 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}


fn draw_icon_calculator(vx: usize, vy: usize, vw: usize) {
    // Fond gris clair avec dégradé
    for dy in 0..vw {
        let c = lerp_color(0xf0f0f0, 0xd0d0d0, dy, vw);
        fb::fill_rect_rgb(vx, vy + dy, vw, 1, c);
    }
    // Cadre foncé
    fb::fill_rect_rgb(vx, vy, vw, 1, 0x888888);
    fb::fill_rect_rgb(vx, vy + vw - 1, vw, 1, 0x888888);
    fb::fill_rect_rgb(vx, vy, 1, vw, 0x888888);
    fb::fill_rect_rgb(vx + vw - 1, vy, 1, vw, 0x888888);
    // Écran
    fb::fill_rect_rgb(vx + 3, vy + 3, vw - 6, 10, 0x0a1628);
    fb::fill_rect_rgb(vx + 3, vy + 3, vw - 6, 1, 0x1e3a5f);
    fb::draw_text_rgb(vx + 6, vy + 5, "0", 0x00ff88, 1);
    // Grille boutons (3x4)
    let colors = [
        [0xdddddd, 0xdddddd, 0xff5555u32],
        [0xdddddd, 0xdddddd, 0xdddddd],
        [0xdddddd, 0xdddddd, 0xdddddd],
        [0xdddddd, 0xdddddd, 0x3377ff],
    ];
    for row in 0..4usize {
        for col in 0..3usize {
            let bx = vx + 3 + col * 11;
            let by = vy + 15 + row * 6;
            fb::fill_rect_rgb(bx, by, 10, 5, colors[row][col]);
            fb::fill_rect_rgb(bx, by, 10, 1, lerp_color(colors[row][col], 0xffffff, 60, 100));
        }
    }
}

fn draw_icon_terminal(vx: usize, vy: usize, vw: usize) {
    // Fond noir profond
    fb::fill_rect_rgb(vx, vy, vw, vw, 0x0a0e1a);
    // Barre de titre macOS-style
    fb::fill_rect_rgb(vx, vy, vw, 8, 0x1c1c1c);
    // Boutons traffic lights
    draw_circle(vx + 5, vy + 4, 2, 0xff5f57);
    draw_circle(vx + 11, vy + 4, 2, 0xffbd2e);
    draw_circle(vx + 17, vy + 4, 2, 0x28c840);
    // Prompt
    fb::draw_text_rgb(vx + 2, vy + 10, ">_", 0x00ff88, 1);
    // Lignes de texte simulées
    let line_cols = [0x3d6b8a, 0x2a4f6e, 0x3d6b8a, 0x1e3a52, 0x2a4f6e];
    for (i, &c) in line_cols.iter().enumerate() {
        let ly = vy + 20 + i * 4;
        let lw = if i % 2 == 0 { vw * 3 / 4 } else { vw / 2 };
        fb::fill_rect_rgb(vx + 2, ly, lw, 2, c);
    }
    // Curseur clignotant (bleu)
    fb::fill_rect_rgb(vx + 10, vy + 10, 2, 7, 0x4488ff);
}

fn draw_icon_files(vx: usize, vy: usize, vw: usize) {
    // Corps du dossier avec dégradé
    let body_y = vy + 8;
    let body_h = vw - 10;
    for dy in 0..body_h {
        let c = lerp_color(0xffc107, 0xf57f17, dy, body_h);
        fb::fill_rect_rgb(vx + 1, body_y + dy, vw - 2, 1, c);
    }
    // Onglet
    for dy in 0..5usize {
        let c = lerp_color(0xffca28, 0xffa000, dy, 5);
        fb::fill_rect_rgb(vx + 1, vy + 4 + dy, 14, 1, c);
    }
    // Reflet en haut du dossier
    fb::fill_rect_rgb(vx + 1, body_y, vw - 2, 2, 0xffe082);
    // Ombre en bas
    fb::fill_rect_rgb(vx + 1, body_y + body_h - 3, vw - 2, 3, 0xe65100);
    // Documents à l'intérieur
    fb::fill_rect_rgb(vx + 6, body_y + 5, vw - 14, 2, 0xfff8e1);
    fb::fill_rect_rgb(vx + 6, body_y + 9, vw - 14, 2, 0xfff8e1);
    fb::fill_rect_rgb(vx + 6, body_y + 13, (vw - 14) * 2 / 3, 2, 0xfff8e1);
}

fn draw_icon_rustpad(vx: usize, vy: usize, vw: usize) {
    // Fond GitHub dark
    fb::fill_rect_rgb(vx, vy, vw, vw, 0x0d1117);
    // Cadre de l'éditeur
    let pad = vw / 8;
    fb::fill_rect_rgb(vx + pad, vy + pad, vw - pad*2, vw - pad*2, 0x161b22);
    fb::fill_rect_rgb(vx + pad, vy + pad, vw - pad*2, 1, 0x30363d);
    // Lignes de code colorées (syntaxe highlight)
    let lh = (vw / 9).max(2);
    let lx = vx + pad + 3;
    let pairs: &[(u32, usize)] = &[
        (0xff7b72, vw / 2),       // fn (rouge)
        (0xa5d6ff, vw / 3),       // let (bleu)
        (0x3fb950, vw * 2 / 5),   // string (vert)
        (0xa5d6ff, vw / 4),       // valeur
        (0x8b949e, vw / 3),       // commentaire
    ];
    for (i, &(color, w)) in pairs.iter().enumerate() {
        let indent = if i == 0 { 0 } else { vw / 8 };
        fb::fill_rect_rgb(lx + indent, vy + pad + 3 + i * (lh + 2), w, lh, color);
    }
    // Bouton Run ▶ (triangle vert)
    let bx = vx + vw - pad - vw / 5;
    let by = vy + pad + 2;
    let bs = (vw / 5).max(4);
    for row in 0..bs {
        let w = (row * 2 + 1).min(bs);
        fb::fill_rect_rgb(bx, by + row, w, 1, 0x238636);
    }
}

// ─── Fenêtres ──────────────────────────────────────────────────────────────

fn draw_window(w: &Win, focused: bool) {
    let x = w.x.max(0) as usize;
    let y = w.y.max(0) as usize;
    let ww = w.w as usize;
    let wh = w.h as usize;

    // Ombre portée
    fb::fill_rect_rgb(x + 4, y + 4, ww, wh, 0x04080f);

    // Fond de la fenêtre
    fb::fill_rect_rgb(x, y, ww, wh, 0x111827);

    // Barre de titre : gradient bleu (focused) ou gris foncé (inactive)
    let title_h = TITLE_H as usize;
    let (tc_top, tc_bot) = if focused {
        (0x1a4c8f, 0x0e2d57)
    } else {
        (0x1f2937, 0x111827)
    };
    for ty in 0..title_h {
        let c = lerp_color(tc_top, tc_bot, ty, title_h);
        fb::fill_rect_rgb(x, y + ty, ww, 1, c);
    }
    // Séparateur bas de la barre de titre
    fb::fill_rect_rgb(x, y + title_h, ww, 1, if focused { 0x2563eb } else { 0x1f2937 });

    // Titre fenêtre en TTF
    let title_clipped = clip(&w.title, (ww / 8).saturating_sub(6));
    fb::draw_text_prop(x + 4, y + 1, title_clipped, 0xe2e8f0, 9.0, false);

    // Boutons de contrôle style macOS (cercles colorés)
    if ww > 36 {
        let btn_y = y + title_h / 2;
        // Minimiser (jaune)
        draw_circle_highlight(x + ww - 26, btn_y, 3, 0xe5a820);
        // Maximiser (vert)
        draw_circle_highlight(x + ww - 17, btn_y, 3, 0x1da44a);
        // Fermer (rouge)
        draw_circle_highlight(x + ww - 8, btn_y, 3, 0xe5463a);
    }

    // Contenu de l'application
    apps::draw_app(w);

    // Bordure de fenêtre
    let bc = if focused { 0x2563eb } else { 0x1f2937 };
    fb::fill_rect_rgb(x, y, ww, 1, bc);
    fb::fill_rect_rgb(x, y, 1, wh, bc);
    fb::fill_rect_rgb(x + ww - 1, y, 1, wh, bc);
    fb::fill_rect_rgb(x, y + wh - 1, ww, 1, bc);

    // Poignée de redimensionnement (coin bas-droit)
    if ww > 10 && wh > 10 {
        for i in 0..5usize {
            fb::pixel_rgb(x + ww - 2 - i, y + wh - 2, 0x4a7bbb);
            fb::pixel_rgb(x + ww - 2, y + wh - 2 - i, 0x4a7bbb);
        }
    }
}

// ─── Barre des tâches ──────────────────────────────────────────────────────

/// Dessine la barre des taches.
pub(crate) fn draw_taskbar(wins: &[Win], menu_open: bool) {
    // Fond gradient foncé
    for y in 0..BAR_H {
        let c = lerp_color(0x0d1a30, 0x0a1224, y, BAR_H);
        fb::fill_rect_rgb(0, fb::HEIGHT - BAR_H + y, fb::WIDTH, 1, c);
    }
    // Ligne de séparation en haut
    fb::fill_rect_rgb(0, fb::HEIGHT - BAR_H, fb::WIDTH, 1, 0x1e4080);

    // Bouton Start
    let sb = start_btn();
    let (sb_top, sb_bot) = if menu_open {
        (0x0c5cbf, 0x0a4a9e)
    } else {
        (0x1a3f6b, 0x102a4a)
    };
    for dy in 0..(sb.h as usize) {
        let c = lerp_color(sb_top, sb_bot, dy, sb.h as usize);
        fb::fill_rect_rgb(sb.x as usize, sb.y as usize + dy, sb.w as usize, 1, c);
    }
    // Reflet haut du bouton Start
    fb::fill_rect_rgb(sb.x as usize, sb.y as usize, sb.w as usize, 1, 0x5599ee);
    // Bordure Start
    fb::fill_rect_rgb(sb.x as usize, sb.y as usize, 1, sb.h as usize, 0x2a5aaa);
    fb::fill_rect_rgb(sb.x as usize + sb.w as usize - 1, sb.y as usize, 1, sb.h as usize, 0x2a5aaa);
    fb::draw_text_prop(sb.x as usize + 4, sb.y as usize + 1, "Start", 0xffffff, 9.0, true);

    // Boutons des fenêtres dans la barre
    for (i, w) in wins.iter().enumerate() {
        let b = taskbar_btn(i);
        if b.x + b.w > fb::WIDTH as i32 { break; }
        let bx = b.x as usize; let by = b.y as usize;
        let bw = b.w as usize; let bh = b.h as usize;
        // Fond du bouton
        for dy in 0..bh {
            let c = lerp_color(0x1e3462, 0x142446, dy, bh);
            fb::fill_rect_rgb(bx, by + dy, bw, 1, c);
        }
        // Reflet
        fb::fill_rect_rgb(bx, by, bw, 1, 0x4477cc);
        // Bordure
        fb::fill_rect_rgb(bx, by, 1, bh, 0x2a4a88);
        fb::fill_rect_rgb(bx + bw - 1, by, 1, bh, 0x2a4a88);
        // Label
        let lbl = clip(&w.title, 7);
        fb::draw_text_prop(bx + 2, by + 1, lbl, 0xd0e8ff, 9.0, false);
    }
}

// ─── Menu Démarrer (style Windows moderne) ─────────────────────────────────

/// Dessine le menu Démarrer avec hover selon la position souris (mx, my).
pub(crate) fn draw_menu(mx: i32, my: i32) {
    let mr = menu_rect();
    let mxi = mr.x as usize;
    let myi = mr.y as usize;
    let mw = mr.w as usize;
    let mh = mr.h as usize;

    // Ombre portée
    fb::fill_rect_rgb(mxi + 4, myi + 4, mw, mh, 0x030608);

    // Fond principal sombre
    for dy in 0..mh {
        let c = lerp_color(0x131c2e, 0x0d1424, dy, mh);
        fb::fill_rect_rgb(mxi, myi + dy, mw, 1, c);
    }

    // Bande d'accent bleue à gauche
    let stripe_w = 4usize;
    for dy in 0..mh {
        let c = lerp_color(0x0078d4, 0x004a8a, dy, mh);
        fb::fill_rect_rgb(mxi, myi + dy, stripe_w, 1, c);
    }

    // Bordure du menu
    fb::fill_rect_rgb(mxi, myi, mw, 1, 0x2a4580);
    fb::fill_rect_rgb(mxi, myi, 1, mh, 0x2a4580);
    fb::fill_rect_rgb(mxi + mw - 1, myi, 1, mh, 0x2a4580);
    fb::fill_rect_rgb(mxi, myi + mh - 1, mw, 1, 0x2a4580);

    // Icônes associées à chaque item
    let icon_colors: &[u32] = &[
        0x28c840, // Terminal
        0xf9ab00, // Fichiers
        0x1a73e8, // Nautile
        0x00b4d8, // Moniteur
        0x888888, // Calculatrice
        0xff7b72, // Rustpad
        0xef4444, // Quitter
    ];

    // Calcul de l'item survolé
    let hover_row: Option<usize> = {
        let rel_y = my - mr.y - MENU_HEADER_H;
        if mx >= mr.x + stripe_w as i32 && mx < mr.x + mr.w
            && rel_y >= 0 && rel_y < (MENU.len() as i32 * MENU_ITEM_H)
        {
            Some((rel_y / MENU_ITEM_H) as usize)
        } else {
            None
        }
    };

    let sep_idx = MENU.len() - 1; // index de "Quitter"

    // Zone vide en haut
    // (optionnel: on pourrait y mettre un logo)

    for (i, item) in MENU.iter().enumerate() {
        let iy = myi + MENU_HEADER_H as usize + i * MENU_ITEM_H as usize;

        // Séparateur avant Quitter
        if i == sep_idx {
            fb::fill_rect_rgb(mxi + stripe_w + 4, iy, mw - stripe_w - 8, 1, 0x1e3a5f);
        }

        // Fond survolé (highlight)
        if hover_row == Some(i) {
            for dy in 0..MENU_ITEM_H as usize {
                let c = lerp_color(0x1e3f6b, 0x162f55, dy, MENU_ITEM_H as usize);
                fb::fill_rect_rgb(mxi + stripe_w, iy + dy, mw - stripe_w, 1, c);
            }
            // Bordure de sélection à gauche
            fb::fill_rect_rgb(mxi + stripe_w, iy, 2, MENU_ITEM_H as usize, 0x4a9eff);
        }

        // Icône de l'item (petit carré coloré 12x12)
        let ic = icon_colors.get(i).copied().unwrap_or(0x555577);
        let icon_x = mxi + stripe_w + 6;
        let icon_y = iy + (MENU_ITEM_H as usize - 12) / 2;
        fb::fill_rect_rgb(icon_x, icon_y, 12, 12, ic);
        // Reflet en haut de l'icône
        fb::fill_rect_rgb(icon_x, icon_y, 12, 1, lerp_color(ic, 0xffffff, 50, 100));
        fb::fill_rect_rgb(icon_x, icon_y, 1, 12, lerp_color(ic, 0xffffff, 30, 100));

        // Texte de l'item en TTF
        let is_quit = i == sep_idx;
        let text_col = if is_quit {
            0xff7b7b
        } else if hover_row == Some(i) {
            0xffffff
        } else {
            0xb8d0ee
        };
        fb::draw_text_prop(
            mxi + stripe_w + 24,
            iy + (MENU_ITEM_H as usize - 10) / 2,
            item,
            text_col,
            10.0,
            hover_row == Some(i),
        );
    }
}

// ─── Curseur souris ────────────────────────────────────────────────────────

/// Dessine le curseur souris avec couleur adaptee au fond.
pub(crate) fn draw_cursor(mx: usize, my: usize) {
    const CUR: [u16; 12] = [
        0b0000000000000001,
        0b0000000000000011,
        0b0000000000000111,
        0b0000000000001111,
        0b0000000000011111,
        0b0000000000111111,
        0b0000000001111111,
        0b0000000000001111,
        0b0000000000011011,
        0b0000000000110001,
        0b0000000001100000,
        0b0000000001000000,
    ];
    let px = mx.min(fb::WIDTH.saturating_sub(1));
    let py = my.min(fb::HEIGHT.saturating_sub(1));
    let bg = fb::get_pixel_rgb(px, py);
    let lum = ((bg >> 16 & 0xff) * 299 + (bg >> 8 & 0xff) * 587 + (bg & 0xff) * 114) / 1000;
    let (fill, outline) = if lum > 140 { (0x000000u32, 0xffffffu32) } else { (0xffffffu32, 0x000000u32) };

    // Outline
    for (row, &bits) in CUR.iter().enumerate() {
        for col in 0..12usize {
            if bits & (1 << col) != 0 {
                for (ddy, ddx) in [(-1i32,0i32),(1,0),(0,-1),(0,1)].iter() {
                    let nx = col as i32 + ddx;
                    let ny = row as i32 + ddy;
                    if nx >= 0 && ny >= 0 && (nx as usize) < 12 {
                        let nr = ny as usize;
                        if nr < CUR.len() && CUR[nr] & (1 << nx) == 0 {
                            let px2 = mx + col;
                            let py2 = my + row;
                            if px2 < fb::WIDTH && py2 < fb::HEIGHT {
                                fb::pixel_rgb(px2, py2, outline);
                            }
                        }
                    }
                }
            }
        }
    }
    // Fill
    for (row, &bits) in CUR.iter().enumerate() {
        for col in 0..12usize {
            if bits & (1 << col) != 0 && mx + col < fb::WIDTH && my + row < fb::HEIGHT {
                fb::pixel_rgb(mx + col, my + row, fill);
            }
        }
    }
}
