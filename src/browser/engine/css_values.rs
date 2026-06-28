//! Parsing de valeurs CSS isole du layout.
//!
//! Ce module est destine a grossir avec `calc()`, couleurs, gradients et
//! longueurs. Pour l'instant il porte les transforms simples utilisees par le
//! layout.

fn first_px(val: &str) -> Option<i32> {
    let tok = val.trim().split_whitespace().next().unwrap_or("").trim();
    if tok.is_empty() { return None; }
    if let Some(px) = tok.strip_suffix("px") { return px.trim().parse::<f32>().ok().map(|v| v as i32); }
    if let Some(rem) = tok.strip_suffix("rem") { return rem.trim().parse::<f32>().ok().map(|v| (v * 16.0) as i32); }
    if let Some(em) = tok.strip_suffix("em") { return em.trim().parse::<f32>().ok().map(|v| (v * 16.0) as i32); }
    tok.parse::<f32>().ok().map(|v| v as i32)
}

/// Parse un sous-ensemble de `transform` compatible no_std :
/// `translate(x[, y])`, `translateX(x)`, `translateY(y)`, `scale(n)`.
pub fn parse_transform(val: &str) -> (i32, i32, i32) {
    let mut tx = 0i32;
    let mut ty = 0i32;
    let mut scale = 100i32;
    for part in val.split(')').filter(|p| !p.trim().is_empty()) {
        let p = part.trim();
        if let Some(args) = p.strip_prefix("translate(") {
            let mut it = args.split(',');
            tx += it.next().and_then(first_px).unwrap_or(0);
            ty += it.next().and_then(first_px).unwrap_or(0);
        } else if let Some(args) = p.strip_prefix("translateX(") {
            tx += first_px(args).unwrap_or(0);
        } else if let Some(args) = p.strip_prefix("translateY(") {
            ty += first_px(args).unwrap_or(0);
        } else if let Some(args) = p.strip_prefix("scale(") {
            let n = args.split(',').next().unwrap_or(args).trim();
            if let Ok(v) = n.parse::<f32>() { scale = (scale as f32 * v) as i32; }
        }
    }
    (tx, ty, scale.clamp(10, 400))
}
