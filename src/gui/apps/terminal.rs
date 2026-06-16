//! Application Terminal (rendu). La logique d'execution reutilise le shell via
//! `shell::run_capture` (voir `gui::apps::key_to_app`).

use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::fs::ramfs;
use crate::users;
use alloc::format;
use alloc::string::String;

/// Dessine le terminal : historique defilant + ligne de saisie.
pub(crate) fn draw(sb: &[String], input: &str, cwd: usize, bx: usize, by: usize, bw: usize, bh: usize) {
    let cols = bw / 8;
    let rows = bh / 8;
    let shown = rows.saturating_sub(1);
    let start = if sb.len() > shown { sb.len() - shown } else { 0 };
    let mut yy = by;
    for line in &sb[start..] {
        fb::draw_text(bx, yy, clip(line, cols), fb::C_GREEN);
        yy += 8;
    }
    let prompt = format!("{}:{}$ ", users::session().username(), ramfs::path_string(ramfs::fs(), cwd));
    let cur = format!("{}{}_", prompt, input);
    fb::draw_text(bx, yy, clip(&cur, cols), fb::C_WHITE);
}
