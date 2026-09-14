//! Application Terminal (rendu). La logique d'execution reutilise le shell via
//! `shell::run_capture` (voir `gui::apps::key_to_app`).

use crate::fs::ramfs;
use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use crate::users;
use alloc::format;
use alloc::string::String;

/// Dessine le terminal : historique defilant + ligne de saisie.
pub(crate) fn draw(sb: &[String], input: &str, cwd: usize, bx: usize, by: usize, bw: usize, bh: usize) {
    let cols = bw / 7;
    let rows = bh / 18;
    let shown = rows.saturating_sub(1);
    let start = if sb.len() > shown { sb.len() - shown } else { 0 };
    let mut yy = by;
    for line in &sb[start..] {
        fb::draw_text_prop(bx, yy, clip(line, cols), 0x5bda8b, 14.0, false);
        yy += 18;
    }
    let prompt = format!("{}:{}$ ", users::session().username(), ramfs::path_string(&ramfs::fs(), cwd));
    let cur = format!("{}{}_", prompt, input);
    fb::draw_text_prop(bx, yy, clip(&cur, cols), 0xeff3f8, 14.0, false);
}
