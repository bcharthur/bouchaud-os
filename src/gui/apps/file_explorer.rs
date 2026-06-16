//! Application Fichiers (rendu). La gestion des clics est dans
//! `gui::apps::app_click`.

use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::fs::ramfs;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Dessine soit la liste du repertoire courant, soit l'apercu d'un fichier.
pub(crate) fn draw(cur: usize, view: &Option<Vec<String>>, name: &str, bx: usize, by: usize, bw: usize, bh: usize) {
    let cols = bw / 8;
    if let Some(lines) = view {
        fb::draw_text(bx, by, clip(name, cols), fb::C_YELLOW);
        let mut yy = by + 10;
        for l in lines {
            if yy + 8 > by + bh { break; }
            fb::draw_text(bx, yy, clip(l, cols), fb::C_WHITE);
            yy += 8;
        }
    } else {
        let fs = ramfs::fs();
        let mut yy = by;
        if cur != 0 { fb::draw_text(bx, yy, "..", fb::C_YELLOW); yy += 9; }
        for i in 0..ramfs::MAX_NODES {
            if yy + 9 > by + bh { break; }
            if fs.nodes[i].used && i != cur && fs.nodes[i].parent == cur {
                if fs.nodes[i].kind == ramfs::NodeKind::Dir {
                    fb::draw_text(bx, yy, &format!("{}/", clip(fs.nodes[i].name_str(), cols - 1)), fb::C_CYAN);
                } else {
                    fb::draw_text(bx, yy, clip(fs.nodes[i].name_str(), cols), fb::C_WHITE);
                }
                yy += 9;
            }
        }
    }
}
