//! Explorateur de fichiers style Windows (barre d'outils + grille d'icones + barre d'etat).

use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::fs::ramfs;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub(crate) const TOOLBAR_H: usize = 16;
pub(crate) const STATUS_H:  usize = 10;
pub(crate) const ICON_COL_W: usize = 72;
pub(crate) const ICON_ROW_H: usize = 52;
pub(crate) const ICON_SIZE:  usize = 32;

/// Dessine l'explorateur dans la zone (bx, by, bw, bh).
pub(crate) fn draw(cur: usize, scroll: i32, selected: Option<usize>, bx: usize, by: usize, bw: usize, bh: usize) {
    // ── Fond ──
    fb::fill_rect_rgb(bx, by, bw, bh, 0xffffff);

    // ── Barre d'outils ──
    draw_toolbar(cur, scroll, bx, by, bw);

    // ── Grille de fichiers ──
    let grid_y = by + TOOLBAR_H + 1;
    let grid_h = bh.saturating_sub(TOOLBAR_H + STATUS_H + 2);
    fb::fill_rect_rgb(bx, grid_y, bw, grid_h, 0xf8f8f8);
    draw_grid(cur, scroll, selected, bx, grid_y, bw, grid_h);

    // ── Barre d'etat ──
    let status_y = by + bh - STATUS_H;
    fb::fill_rect_rgb(bx, status_y, bw, STATUS_H, 0xe0e0e0);
    fb::fill_rect_rgb(bx, status_y, bw, 1, 0xaaaaaa);
    let count = count_entries(cur);
    let status = format!("  {} element{}", count, if count != 1 { "s" } else { "" });
    fb::draw_text_rgb(bx + 2, status_y + 1, &status, 0x333333, 1);
}

fn draw_toolbar(cur: usize, _scroll: i32, bx: usize, by: usize, bw: usize) {
    fb::fill_rect_rgb(bx, by, bw, TOOLBAR_H, 0xf0f0f0);
    fb::fill_rect_rgb(bx, by + TOOLBAR_H, bw, 1, 0xaaaaaa);

    // Boutons ← → ↑
    draw_btn(bx + 2, by + 2, 12, 11, "<", 0x555555);
    draw_btn(bx + 16, by + 2, 12, 11, ">", 0x555555);
    draw_btn(bx + 30, by + 2, 12, 11, "^", 0x555555);

    // Chemin courant
    let fs = ramfs::fs();
    let path = ramfs::path_string(fs, cur);
    let px = bx + 46;
    let pw = bw.saturating_sub(50);
    fb::fill_rect_rgb(px, by + 3, pw, 10, 0xffffff);
    fb::fill_rect_rgb(px, by + 3, pw, 10, 0xffffff);
    // thin border
    fb::fill_rect_rgb(px, by + 3, pw, 1, 0xaaaaaa);
    fb::fill_rect_rgb(px, by + 12, pw, 1, 0xaaaaaa);
    fb::fill_rect_rgb(px, by + 3, 1, 10, 0xaaaaaa);
    fb::fill_rect_rgb(px + pw - 1, by + 3, 1, 10, 0xaaaaaa);
    let max_chars = pw / 6;
    fb::draw_text_rgb(px + 2, by + 4, clip(&path, max_chars), 0x222222, 1);
}

fn draw_btn(x: usize, y: usize, w: usize, h: usize, label: &str, color: u32) {
    fb::fill_rect_rgb(x, y, w, h, 0xdddddd);
    fb::fill_rect_rgb(x, y, w, 1, 0xffffff);
    fb::fill_rect_rgb(x, y, 1, h, 0xffffff);
    fb::fill_rect_rgb(x, y + h - 1, w, 1, 0x888888);
    fb::fill_rect_rgb(x + w - 1, y, 1, h, 0x888888);
    let lx = x + (w - label.len() * 6) / 2;
    fb::draw_text_rgb(lx, y + 2, label, color, 1);
}

fn draw_grid(cur: usize, scroll: i32, selected: Option<usize>, bx: usize, by: usize, bw: usize, bh: usize) {
    let cols = (bw / ICON_COL_W).max(1);
    let fs = ramfs::fs();

    let mut entries: Vec<(usize, bool, String)> = Vec::new();
    if cur != 0 {
        entries.push((usize::MAX, true, "..".into()));
    }
    for i in 0..ramfs::MAX_NODES {
        if fs.nodes[i].used && i != cur && fs.nodes[i].parent == cur {
            let is_dir = fs.nodes[i].kind == ramfs::NodeKind::Dir;
            entries.push((i, is_dir, fs.nodes[i].name_str().into()));
        }
    }

    let scroll_rows = scroll.max(0) as usize;
    let skip = scroll_rows * cols;

    for (idx, &(_node_idx, is_dir, ref name)) in entries.iter().enumerate() {
        if idx < skip { continue; }
        let visible_idx = idx - skip;
        let row = visible_idx / cols;
        let col = visible_idx % cols;
        let ix = bx + col * ICON_COL_W + 4;
        let iy = by + row * ICON_ROW_H + 4;
        if iy + ICON_ROW_H > by + bh { break; }

        let is_sel = selected == Some(idx);
        if is_sel {
            fb::fill_rect_rgb(bx + col * ICON_COL_W, by + row * ICON_ROW_H, ICON_COL_W, ICON_ROW_H, 0xcce4ff);
        }

        if is_dir {
            draw_folder_icon(ix + (ICON_COL_W - 8 - ICON_SIZE) / 2, iy, ICON_SIZE);
        } else {
            draw_file_icon(ix + (ICON_COL_W - 8 - ICON_SIZE) / 2, iy, ICON_SIZE);
        }

        let max_chars = (ICON_COL_W - 4) / 6;
        let display = clip(name, max_chars);
        let tw = display.len() * 6;
        let tx = bx + col * ICON_COL_W + (ICON_COL_W.saturating_sub(tw)) / 2;
        let ty = iy + ICON_SIZE + 2;
        fb::draw_text_rgb(tx, ty, display, 0x222222, 1);
    }
}

fn draw_folder_icon(x: usize, y: usize, size: usize) {
    // Tab
    fb::fill_rect_rgb(x, y + size / 5, size / 2, size / 8, 0xf9ab00);
    fb::fill_rect_rgb(x, y + size / 5, size / 2, 1, 0xffd04f);
    // Body
    let body_y = y + size / 5 + size / 8 - 1;
    let body_h = size - size / 5 - size / 8;
    fb::fill_rect_rgb(x, body_y, size, body_h, 0xf9ab00);
    fb::fill_rect_rgb(x, body_y, size, 2, 0xffd04f);
    fb::fill_rect_rgb(x, body_y + body_h - 3, size, 3, 0xc87b00);
    // Paper lines inside
    fb::fill_rect_rgb(x + size / 8, body_y + 4, size * 3 / 4, 2, 0xffffff80 & 0xffffffu32);
    fb::fill_rect_rgb(x + size / 8, body_y + 4, size * 3 / 4, 1, 0xffd88a);
    fb::fill_rect_rgb(x + size / 8, body_y + 8, size / 2, 1, 0xffd88a);
}

fn draw_file_icon(x: usize, y: usize, size: usize) {
    let fold = size / 4;
    let w = size * 3 / 4;
    // White page body
    fb::fill_rect_rgb(x, y, w, size, 0xffffff);
    // Folded corner (grey triangle approximation)
    fb::fill_rect_rgb(x + w - fold, y, fold, fold, 0xdddddd);
    for i in 0..fold {
        fb::fill_rect_rgb(x + w - fold + i, y + i, fold - i, 1, 0xcccccc);
    }
    // Page outline
    fb::fill_rect_rgb(x, y, w, 1, 0x888888);
    fb::fill_rect_rgb(x, y + size - 1, w, 1, 0x888888);
    fb::fill_rect_rgb(x, y, 1, size, 0x888888);
    fb::fill_rect_rgb(x + w - 1, y + fold, 1, size - fold, 0x888888);
    fb::fill_rect_rgb(x + w - fold, y + fold, fold, 1, 0x888888);
    // Text lines
    let line_x = x + 3;
    let line_w = w.saturating_sub(6);
    for row in 0..4usize {
        let ly = y + fold + 2 + row * 4;
        if ly + 1 >= y + size { break; }
        let lw = if row == 3 { line_w * 2 / 3 } else { line_w };
        fb::fill_rect_rgb(line_x, ly, lw, 1, 0xbbbbbb);
    }
}

fn count_entries(cur: usize) -> usize {
    let fs = ramfs::fs();
    let mut n = if cur != 0 { 1 } else { 0 };
    for i in 0..ramfs::MAX_NODES {
        if fs.nodes[i].used && i != cur && fs.nodes[i].parent == cur { n += 1; }
    }
    n
}

// ── Hit tests pour la barre d'outils ──────────────────────────────────────────

pub(crate) struct ToolbarHit { pub action: ToolbarAction }

pub(crate) enum ToolbarAction {
    Back, Forward, Up, None,
}

pub(crate) fn toolbar_hit(bx: usize, by: usize, mx: i32, my: i32) -> ToolbarAction {
    let x = mx as usize;
    let y = my as usize;
    if y < by || y >= by + TOOLBAR_H { return ToolbarAction::None; }
    if x >= bx + 2  && x < bx + 14  { return ToolbarAction::Back; }
    if x >= bx + 16 && x < bx + 28  { return ToolbarAction::Forward; }
    if x >= bx + 30 && x < bx + 42  { return ToolbarAction::Up; }
    ToolbarAction::None
}

/// Retourne le numero (1-indexed depuis 0) de l'entree cliquee, ou None.
pub(crate) fn grid_hit(
    _cur: usize, scroll: i32, bx: usize, by: usize, bw: usize, mx: i32, my: i32,
) -> Option<usize> {
    let cols = (bw / ICON_COL_W).max(1);
    let rel_x = (mx as usize).saturating_sub(bx);
    let rel_y = (my as usize).saturating_sub(by);
    let col = rel_x / ICON_COL_W;
    let row = rel_y / ICON_ROW_H;
    if col >= cols { return None; }
    let skip = scroll.max(0) as usize * cols;
    Some(skip + row * cols + col)
}
