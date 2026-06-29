//! Longueurs CSS → pixels. Centralise la conversion des unités courantes pour
//! que le layout n'ait plus à les éparpiller. `em`/`rem`/`%` dépendent du
//! contexte (taille de police, dimension parente) passé en paramètre.

/// Contexte de résolution d'une longueur relative.
#[derive(Clone, Copy)]
pub struct LenCtx {
    pub font_px: f32,   // pour em / ex / ch
    pub root_px: f32,   // pour rem
    pub percent_of: f32, // base du pourcentage (largeur/hauteur parente)
    pub vw: f32,        // 1% de la largeur viewport
    pub vh: f32,        // 1% de la hauteur viewport
}

impl LenCtx {
    pub fn simple(font_px: f32) -> LenCtx {
        LenCtx { font_px, root_px: 16.0, percent_of: 0.0, vw: 12.8, vh: 7.2 }
    }
}

/// Convertit une valeur CSS (`"12px"`, `"1.5em"`, `"50%"`, `"2rem"`, `"10vw"`…)
/// en pixels. Retourne None si non numérique/inconnu.
pub fn to_px(val: &str, ctx: &LenCtx) -> Option<f32> {
    let v = val.trim();
    if v.is_empty() { return None; }
    if v == "0" { return Some(0.0); }
    let (num, unit) = split_num_unit(v)?;
    Some(match unit {
        "px" | "" => num,
        "em" => num * ctx.font_px,
        "rem" => num * ctx.root_px,
        "ex" => num * ctx.font_px * 0.5,
        "ch" => num * ctx.font_px * 0.5,
        "%" => num / 100.0 * ctx.percent_of,
        "vw" => num * ctx.vw,
        "vh" => num * ctx.vh,
        "vmin" => num * ctx.vw.min(ctx.vh),
        "vmax" => num * ctx.vw.max(ctx.vh),
        "pt" => num * 96.0 / 72.0,
        "pc" => num * 16.0,
        "cm" => num * 96.0 / 2.54,
        "mm" => num * 96.0 / 25.4,
        "in" => num * 96.0,
        "q" => num * 96.0 / 101.6,
        _ => return None,
    })
}

/// Sépare un nombre de son unité (`"1.5em"` → `(1.5, "em")`).
pub fn split_num_unit(v: &str) -> Option<(f32, &str)> {
    let bytes = v.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') { i += 1; }
    let start_digits = i;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') { i += 1; }
    if i == start_digits { return None; }
    let num: f32 = v[..i].parse().ok()?;
    Some((num, v[i..].trim()))
}
