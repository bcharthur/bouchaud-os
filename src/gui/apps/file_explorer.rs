//! Explorateur de fichiers style Windows (barre d'outils + grille d'icones + barre d'etat).

use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::fs::ramfs;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub(crate) const TOOLBAR_H: usize = 32;
pub(crate) const STATUS_H:  usize = 24;
pub(crate) const ICON_COL_W: usize = 96;
pub(crate) const ICON_ROW_H: usize = 82;
pub(crate) const ICON_SIZE:  usize = 48;

/// Dessine l'explorateur dans la zone (bx, by, bw, bh).
pub(crate) fn draw(cur: usize, scroll: i32, selected: Option<usize>, bx: usize, by: usize, bw: usize, bh: usize) {
    // ── Fond ──
    fb::fill_rect_rgb(bx, by, bw, bh, 0xffffff);

    // BOUCHAUD_GFX_CULLING_AMONT_V1
    //
    // Les trois bandes sont independantes, et chacune coute autre chose que des
    // pixels : la grille parcourt le RAMFS entier et alloue une chaine par
    // entree, la barre d'etat le reparcourt pour compter, la barre d'outils
    // formate le chemin courant. Un degat ne touche presque jamais les trois.
    //
    // La decoupe jetait deja les pixels ; elle ne pouvait rien contre le
    // parcours qui les produit.

    // ── Barre d'outils ──
    if fb::decoupe_touche(bx, by, bw, TOOLBAR_H + 1) {
        draw_toolbar(cur, scroll, bx, by, bw);
    }

    // ── Grille de fichiers ──
    let grid_y = by + TOOLBAR_H + 1;
    let grid_h = bh.saturating_sub(TOOLBAR_H + STATUS_H + 2);
    if fb::decoupe_touche(bx, grid_y, bw, grid_h) {
        fb::fill_rect_rgb(bx, grid_y, bw, grid_h, 0xf8f8f8);
        draw_grid(cur, scroll, selected, bx, grid_y, bw, grid_h);
    }

    // ── Barre d'etat ──
    let status_y = by + bh - STATUS_H;
    if fb::decoupe_touche(bx, status_y, bw, STATUS_H) {
        fb::fill_rect_rgb(bx, status_y, bw, STATUS_H, 0xe0e0e0);
        fb::fill_rect_rgb(bx, status_y, bw, 1, 0xaaaaaa);
        let count = count_entries(cur);
        let status = format!("  {} element{}", count, if count != 1 { "s" } else { "" });
        fb::draw_text_prop(bx + 8, status_y + 4, &status, 0x333333, 13.0, false);
    }
}

fn draw_toolbar(cur: usize, _scroll: i32, bx: usize, by: usize, bw: usize) {
    fb::fill_rect_rgb(bx, by, bw, TOOLBAR_H, 0xf0f0f0);
    fb::fill_rect_rgb(bx, by + TOOLBAR_H, bw, 1, 0xaaaaaa);

    // Navigation : glyphes DejaVu Sans centres dans de vrais boutons.
    draw_btn(bx + 6, by + 5, 22, 22, "<", 0x444444);
    draw_btn(bx + 32, by + 5, 22, 22, ">", 0x444444);
    draw_btn(bx + 58, by + 5, 22, 22, "^", 0x444444);

    let fs = ramfs::fs();
    let path = ramfs::path_string(&fs, cur);
    let px = bx + 88;
    let pw = bw.saturating_sub(96);
    fb::fill_rect_rgb(px, by + 5, pw, 22, 0xffffff);
    fb::fill_rect_rgb(px, by + 5, pw, 1, 0xaaaaaa);
    fb::fill_rect_rgb(px, by + 26, pw, 1, 0xaaaaaa);
    fb::fill_rect_rgb(px, by + 5, 1, 22, 0xaaaaaa);
    fb::fill_rect_rgb(px + pw.saturating_sub(1), by + 5, 1, 22, 0xaaaaaa);
    let max_chars = pw / 7;
    fb::draw_text_prop(px + 6, by + 8, clip(&path, max_chars), 0x222222, 13.0, false);
}

fn draw_btn(x: usize, y: usize, w: usize, h: usize, label: &str, color: u32) {
    fb::fill_rect_rgb(x, y, w, h, 0xe5e7eb);
    fb::fill_rect_rgb(x, y, w, 1, 0xffffff);
    fb::fill_rect_rgb(x, y, 1, h, 0xffffff);
    fb::fill_rect_rgb(x, y + h - 1, w, 1, 0x9ca3af);
    fb::fill_rect_rgb(x + w - 1, y, 1, h, 0x9ca3af);
    let taille = 14.0;
    let largeur = fb::text_width(label, taille, true);
    fb::draw_text_prop(
        x + w.saturating_sub(largeur) / 2,
        y + h.saturating_sub(taille as usize) / 2,
        label,
        color,
        taille,
        true,
    );
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

        let cellule_x = bx + col * ICON_COL_W;
        let cellule_y = by + row * ICON_ROW_H;

        // Le libelle DejaVu Sans est mesure avant le culling afin de centrer
        // son rectangle reel et de conserver ses bords anticreneles.
        const HAUTEUR_LIBELLE: usize = 16;
        let max_chars = (ICON_COL_W - 8) / 7;
        let display = clip(name, max_chars);
        let tw = fb::text_width(display, 13.0, false);
        let tx = cellule_x + ICON_COL_W.saturating_sub(tw) / 2;
        let ty = iy + ICON_SIZE + 5;

        // BOUCHAUD_GFX_CULLING_AMONT_V1 : une cellule hors du degat ne coute
        // plus que son tour de boucle -- ni icone rasterisee, ni libelle pose
        // caractere par caractere. L'union couvre la cellule ET le debord du
        // libelle, de sorte que le culling ne peut rien perdre.
        let gauche = cellule_x.min(tx);
        let droite = (cellule_x + ICON_COL_W).max(tx + tw);
        let bas = (cellule_y + ICON_ROW_H).max(ty + HAUTEUR_LIBELLE);
        if !fb::decoupe_touche(gauche, cellule_y, droite - gauche,
            bas - cellule_y) { continue }

        let is_sel = selected == Some(idx);
        if is_sel {
            fb::fill_rect_rgb(cellule_x, cellule_y, ICON_COL_W, ICON_ROW_H, 0xcce4ff);
        }

        if is_dir {
            draw_folder_icon(ix + (ICON_COL_W - 8 - ICON_SIZE) / 2, iy, ICON_SIZE);
        } else {
            draw_file_icon(ix + (ICON_COL_W - 8 - ICON_SIZE) / 2, iy, ICON_SIZE);
        }

        fb::draw_text_prop(tx, ty, display, 0x222222, 13.0, false);
    }
}

// LA COPIE LOCALE A ETE RETIREE, PAS DEPLACEE PAR COMMODITE.
//
// Elle paniquait : `min > max. min = 447, max = 446`, releve le 16 septembre
// 2026 en ouvrant cette fenetre. Une boite de 34 de haut avec un rayon de 17
// -- la ligne interne de l'icone de fichier, juste en dessous -- rendait
// `clamp(haut + 17, haut + 16)`. Tout fichier affiche tuait le noyau.
//
// `gui::geometrie::dans_arrondi` borne le rayon a la moitie de la boite et
// devient totale. La meme primitive existait aussi dans `reference_gop`, pour
// le logo de demarrage, qui est peint AVANT que quoi que ce soit sache
// rapporter une faute.
use crate::gui::geometrie::dans_arrondi;

fn couverture_icone(px: usize, py: usize, size: usize, forme: u8) -> u8 {
    let mut compte = 0u16;
    for sy in 0..2 {
        for sx in 0..2 {
            let nx = ((px * 2 + sx) * 1024 / (size * 2).max(1)) as i32;
            let ny = ((py * 2 + sy) * 1024 / (size * 2).max(1)) as i32;
            let dedans = match forme {
                // Dossier : languette + corps arrondi.
                0 => dans_arrondi(nx, ny, 70, 260, 954, 910, 72)
                    || dans_arrondi(nx, ny, 90, 150, 535, 420, 62),
                1 => dans_arrondi(nx, ny, 110, 320, 914, 405, 36),
                // Fichier, pli et lignes internes.
                2 => dans_arrondi(nx, ny, 175, 70, 835, 950, 58)
                    && !(nx > 630 && ny < 275 && nx - ny > 545),
                3 => nx >= 630 && nx <= 835 && ny >= 70 && ny <= 275
                    && nx - 630 >= ny - 70,
                _ => (0..4).any(|ligne| {
                    let haut = 430 + ligne * 105;
                    dans_arrondi(nx, ny, 285, haut, if ligne == 3 { 650 } else { 735 }, haut + 34, 17)
                }),
            };
            if dedans { compte += 1; }
        }
    }
    (compte * 255 / 4) as u8
}

fn pose_forme_icone(x: usize, y: usize, size: usize, forme: u8, couleur: u32) {
    for py in 0..size {
        for px in 0..size {
            let alpha = couverture_icone(px, py, size, forme);
            if alpha != 0 { fb::blend_rgb(x + px, y + py, couleur, alpha); }
        }
    }
}

fn draw_folder_icon(x: usize, y: usize, size: usize) {
    pose_forme_icone(x, y, size, 0, 0xf2a900);
    pose_forme_icone(x, y, size, 1, 0xffd166);
}

fn draw_file_icon(x: usize, y: usize, size: usize) {
    // Une ombre douce detache la page du fond blanc.
    pose_forme_icone(x + 1, y + 1, size, 2, 0xb8c0cc);
    pose_forme_icone(x, y, size, 2, 0xffffff);
    pose_forme_icone(x, y, size, 3, 0xd8dee8);
    pose_forme_icone(x, y, size, 4, 0x7d8a9b);
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
    if x >= bx + 6  && x < bx + 28  { return ToolbarAction::Back; }
    if x >= bx + 32 && x < bx + 54  { return ToolbarAction::Forward; }
    if x >= bx + 58 && x < bx + 80  { return ToolbarAction::Up; }
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
