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
    // Fonctions CSS modernes frequentes dans Google/Ecosia. On gere les cas
    // simples sans moteur d'expression complet : var(--x, fallback), min(a,b),
    // max(a,b), clamp(min, preferred, max), calc(A +/- B). Si une variable CSS
    // reste non resolue, le fallback standard repasse a auto au lieu de paniquer.
    if let Some(inner) = func_inner(v, "var") {
        let vals = split_args(inner);
        if vals.len() >= 2 { return to_px(vals[1], ctx); }
        return None;
    }
    if let Some(inner) = func_inner(v, "min") {
        let vals = split_args(inner);
        let mut best: Option<f32> = None;
        for a in vals { if let Some(px) = to_px(a, ctx) { best = Some(best.map(|b| b.min(px)).unwrap_or(px)); } }
        return best;
    }
    if let Some(inner) = func_inner(v, "max") {
        let vals = split_args(inner);
        let mut best: Option<f32> = None;
        for a in vals { if let Some(px) = to_px(a, ctx) { best = Some(best.map(|b| b.max(px)).unwrap_or(px)); } }
        return best;
    }
    if let Some(inner) = func_inner(v, "clamp") {
        let vals = split_args(inner);
        if vals.len() >= 3 {
            let lo = to_px(vals[0], ctx)?;
            let mid = to_px(vals[1], ctx)?;
            let hi = to_px(vals[2], ctx)?;
            return Some(mid.max(lo).min(hi.max(lo)));
        }
    }
    if let Some(inner) = func_inner(v, "calc") {
        return eval_calc_px(inner, ctx);
    }
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

fn func_inner<'a>(v: &'a str, name: &str) -> Option<&'a str> {
    let pfx = { let mut s = alloc::string::String::new(); s.push_str(name); s.push('('); s };
    if !v.starts_with(&pfx) || !v.ends_with(')') { return None; }
    Some(&v[pfx.len()..v.len()-1])
}

fn split_args(s: &str) -> alloc::vec::Vec<&str> {
    let mut out = alloc::vec::Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => { out.push(s[start..i].trim()); start = i + 1; }
            _ => {}
        }
    }
    if start <= s.len() { out.push(s[start..].trim()); }
    out
}

fn eval_calc_px(s: &str, ctx: &LenCtx) -> Option<f32> {
    // Calcul gauche-droite pour additions/soustractions de longueurs.
    let mut total = 0.0f32;
    let mut sign = 1.0f32;
    let mut tok = alloc::string::String::new();
    let mut depth = 0i32;
    let flush = |tok: &mut alloc::string::String, total: &mut f32, sign: f32| -> Option<()> {
        let t = tok.trim();
        if !t.is_empty() { *total += sign * to_px(t, ctx)?; }
        tok.clear();
        Some(())
    };
    for ch in s.chars() {
        match ch {
            '(' => { depth += 1; tok.push(ch); }
            ')' => { depth -= 1; tok.push(ch); }
            '+' if depth == 0 => { flush(&mut tok, &mut total, sign)?; sign = 1.0; }
            '-' if depth == 0 => { flush(&mut tok, &mut total, sign)?; sign = -1.0; }
            _ => tok.push(ch),
        }
    }
    flush(&mut tok, &mut total, sign)?;
    Some(total.max(0.0))
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
