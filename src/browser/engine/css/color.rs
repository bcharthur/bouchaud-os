//! Couleurs CSS → `0x00RRGGBB` (opaque, composité sur blanc pour l'alpha).
//!
//! Supporte : `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()/rgba()`,
//! `hsl()/hsla()` (syntaxe virgule ou espace `/ a`), ~150 noms CSS,
//! `transparent`, `currentcolor`, et l'extraction de la 1ʳᵉ couleur d'un
//! `linear-gradient(...)`.

fn fabsf(x: f32) -> f32 { if x < 0.0 { -x } else { x } }
fn chan(v: f32) -> u32 { let n = (v * 255.0 + 0.5) as i32; if n < 0 { 0 } else if n > 255 { 255 } else { n as u32 } }

// Composite un canal sur fond blanc selon l'alpha (0..1).
fn over_white(c: u32, a: f32) -> u32 {
    let v = c as f32 * a + 255.0 * (1.0 - a);
    let n = (v + 0.5) as i32;
    if n < 0 { 0 } else if n > 255 { 255 } else { n as u32 }
}

/// HSL → RGB (h en degrés, s/l en 0..1). `core` only (pas de round/floor/rem).
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> u32 {
    let mut hh = h % 360.0;
    if hh < 0.0 { hh += 360.0; }
    let c = (1.0 - fabsf(2.0 * l - 1.0)) * s;
    let hp = hh / 60.0;
    let x = c * (1.0 - fabsf(hp % 2.0 - 1.0));
    let (r1, g1, b1) = if hp < 1.0 { (c, x, 0.0) } else if hp < 2.0 { (x, c, 0.0) }
        else if hp < 3.0 { (0.0, c, x) } else if hp < 4.0 { (0.0, x, c) }
        else if hp < 5.0 { (x, 0.0, c) } else { (c, 0.0, x) };
    let m = l - c / 2.0;
    (chan(r1 + m) << 16) | (chan(g1 + m) << 8) | chan(b1 + m)
}

/// Couleur nommée CSS → RGB. `transparent`/`currentcolor` → None (héritage).
pub fn named(s: &str) -> Option<u32> {
    Some(match s {
        "black" => 0x000000, "white" => 0xffffff, "red" => 0xff0000, "green" => 0x008000,
        "blue" => 0x0000ff, "navy" => 0x000080, "gray" | "grey" => 0x808080, "silver" => 0xc0c0c0,
        "lightgray" | "lightgrey" => 0xd3d3d3, "darkgray" | "darkgrey" => 0xa9a9a9,
        "maroon" => 0x800000, "yellow" => 0xffff00, "olive" => 0x808000, "lime" => 0x00ff00,
        "aqua" | "cyan" => 0x00ffff, "teal" => 0x008080, "fuchsia" | "magenta" => 0xff00ff,
        "purple" => 0x800080, "orange" => 0xffa500, "pink" => 0xffc0cb, "brown" => 0xa52a2a,
        "gold" => 0xffd700,
        "whitesmoke" => 0xf5f5f5, "gainsboro" => 0xdcdcdc, "lightslategray" | "lightslategrey" => 0x778899,
        "slategray" | "slategrey" => 0x708090, "dimgray" | "dimgrey" => 0x696969, "darkslategray" => 0x2f4f4f,
        "ghostwhite" => 0xf8f8ff, "snow" => 0xfffafa, "ivory" => 0xfffff0, "beige" => 0xf5f5dc,
        "lightblue" => 0xadd8e6, "skyblue" => 0x87ceeb, "lightskyblue" => 0x87cefa, "deepskyblue" => 0x00bfff,
        "dodgerblue" => 0x1e90ff, "cornflowerblue" => 0x6495ed, "steelblue" => 0x4682b4, "royalblue" => 0x4169e1,
        "mediumblue" => 0x0000cd, "darkblue" => 0x00008b, "midnightblue" => 0x191970, "indigo" => 0x4b0082,
        "slateblue" => 0x6a5acd, "mediumslateblue" => 0x7b68ee, "blueviolet" => 0x8a2be2, "darkviolet" => 0x9400d3,
        "violet" => 0xee82ee, "orchid" => 0xda70d6, "plum" => 0xdda0dd, "mediumpurple" => 0x9370db,
        "crimson" => 0xdc143c, "firebrick" => 0xb22222, "darkred" => 0x8b0000, "tomato" => 0xff6347,
        "orangered" => 0xff4500, "coral" => 0xff7f50, "salmon" => 0xfa8072, "lightsalmon" => 0xffa07a,
        "darkorange" => 0xff8c00, "khaki" => 0xf0e68c, "darkkhaki" => 0xbdb76b, "wheat" => 0xf5deb3,
        "tan" => 0xd2b48c, "sandybrown" => 0xf4a460, "goldenrod" => 0xdaa520, "chocolate" => 0xd2691e,
        "sienna" => 0xa0522d, "saddlebrown" => 0x8b4513, "forestgreen" => 0x228b22, "seagreen" => 0x2e8b57,
        "mediumseagreen" => 0x3cb371, "limegreen" => 0x32cd32, "lawngreen" => 0x7cfc00, "chartreuse" => 0x7fff00,
        "greenyellow" => 0xadff2f, "yellowgreen" => 0x9acd32, "darkgreen" => 0x006400, "lightgreen" => 0x90ee90,
        "palegreen" => 0x98fb98, "springgreen" => 0x00ff7f, "mediumspringgreen" => 0x00fa9a,
        "turquoise" => 0x40e0d0, "mediumturquoise" => 0x48d1cc, "darkturquoise" => 0x00ced1, "lightcyan" => 0xe0ffff,
        "cadetblue" => 0x5f9ea0, "darkcyan" => 0x008b8b, "aquamarine" => 0x7fffd4, "lightyellow" => 0xffffe0,
        "lightpink" => 0xffb6c1, "hotpink" => 0xff69b4, "deeppink" => 0xff1493, "palevioletred" => 0xdb7093,
        "mediumvioletred" => 0xc71585, "lavender" => 0xe6e6fa, "thistle" => 0xd8bfd8, "mistyrose" => 0xffe4e1,
        "rebeccapurple" => 0x663399,
        // ajouts pour frameworks (Bootstrap/Material/Tailwind défauts usuels)
        "darkslateblue" => 0x483d8b, "darkmagenta" => 0x8b008b, "darkolivegreen" => 0x556b2f,
        "olivedrab" => 0x6b8e23, "darkgoldenrod" => 0xb8860b, "indianred" => 0xcd5c5c, "rosybrown" => 0xbc8f8f,
        "lightcoral" => 0xf08080, "peachpuff" => 0xffdab9, "moccasin" => 0xffe4b5, "papayawhip" => 0xffefd5,
        "navajowhite" => 0xffdead, "bisque" => 0xffe4c4, "blanchedalmond" => 0xffebcd, "cornsilk" => 0xfff8dc,
        "lemonchiffon" => 0xfffacd, "honeydew" => 0xf0fff0, "mintcream" => 0xf5fffa, "azure" => 0xf0ffff,
        "aliceblue" => 0xf0f8ff, "lavenderblush" => 0xfff0f5, "seashell" => 0xfff5ee, "oldlace" => 0xfdf5e6,
        "floralwhite" => 0xfffaf0, "linen" => 0xfaf0e6, "antiquewhite" => 0xfaebd7, "lightgoldenrodyellow" => 0xfafad2,
        "palegoldenrod" => 0xeee8aa, "darkseagreen" => 0x8fbc8f, "mediumaquamarine" => 0x66cdaa,
        "lightseagreen" => 0x20b2aa, "paleturquoise" => 0xafeeee, "powderblue" => 0xb0e0e6,
        "lightsteelblue" => 0xb0c4de, "mediumorchid" => 0xba55d3, "darkorchid" => 0x9932cc, "magenta2" => 0xff00ff,
        "peru" => 0xcd853f, "burlywood" => 0xdeb887,
        "transparent" | "currentcolor" => return None,
        _ => return None,
    })
}

/// Parse une valeur de couleur CSS. Retourne None si non reconnue.
pub fn parse(s: &str) -> Option<u32> {
    let s = s.trim();
    // linear-gradient / radial-gradient → 1ʳᵉ couleur (#hex ou nom rgb()).
    if s.contains("gradient(") {
        if let Some(pos) = s.find('#') {
            let end = s[pos + 1..].find(|c: char| !c.is_ascii_hexdigit())
                .map(|i| pos + 1 + i)
                .unwrap_or(s.len());
            return parse(&s[pos..end]);
        }
        if let Some(pos) = s.find("rgb") { return parse(&s[pos..]); }
        return None;
    }
    if let Some(h) = s.strip_prefix('#') {
        let h = h.trim();
        match h.len() {
            3 => {
                let r = u8::from_str_radix(&h[0..1], 16).ok()?;
                let g = u8::from_str_radix(&h[1..2], 16).ok()?;
                let b = u8::from_str_radix(&h[2..3], 16).ok()?;
                return Some(((r as u32 * 17) << 16) | ((g as u32 * 17) << 8) | (b as u32 * 17));
            }
            4 => {
                // #rgba : alpha composité sur blanc.
                let r = u8::from_str_radix(&h[0..1], 16).ok()? as u32 * 17;
                let g = u8::from_str_radix(&h[1..2], 16).ok()? as u32 * 17;
                let b = u8::from_str_radix(&h[2..3], 16).ok()? as u32 * 17;
                let a = u8::from_str_radix(&h[3..4], 16).ok()? as u32 * 17;
                let af = a as f32 / 255.0;
                return Some((over_white(r, af) << 16) | (over_white(g, af) << 8) | over_white(b, af));
            }
            6 => return u32::from_str_radix(&h[..6], 16).ok().map(|v| v & 0xffffff),
            8 => {
                let rgb = u32::from_str_radix(&h[..6], 16).ok()?;
                let a = u8::from_str_radix(&h[6..8], 16).ok()? as u32;
                let af = a as f32 / 255.0;
                let r = (rgb >> 16) & 0xff; let g = (rgb >> 8) & 0xff; let b = rgb & 0xff;
                return Some((over_white(r, af) << 16) | (over_white(g, af) << 8) | over_white(b, af));
            }
            _ => return None,
        }
    }
    if let Some(rest) = s.strip_prefix("rgb") {
        let inside = rest.trim_start_matches('a').trim_start_matches('(').trim_end_matches(')');
        // alpha éventuel (rgba ou `/ a`) → composite sur blanc.
        let (rgb_part, alpha) = split_alpha(inside);
        let mut it = rgb_part.split(|c: char| c == ',' || c == ' ').filter(|x| !x.trim().is_empty()).map(|x| x.trim().trim_end_matches('%'));
        let r: u32 = it.next()?.parse::<f32>().ok()? as u32;
        let g: u32 = it.next()?.parse::<f32>().ok()? as u32;
        let b: u32 = it.next()?.parse::<f32>().ok()? as u32;
        let (r, g, b) = (r & 255, g & 255, b & 255);
        return Some(match alpha {
            Some(a) if a < 1.0 => (over_white(r, a) << 16) | (over_white(g, a) << 8) | over_white(b, a),
            _ => (r << 16) | (g << 8) | b,
        });
    }
    if let Some(rest) = s.strip_prefix("hsl") {
        let inside = rest.trim_start_matches('a').trim_start_matches('(').trim_end_matches(')');
        let (hsl_part, alpha) = split_alpha(inside);
        let mut it = hsl_part.split(|c: char| c == ',' || c == ' ').filter(|x| !x.trim().is_empty());
        let h: f32 = it.next()?.trim().trim_end_matches("deg").parse().ok()?;
        let sp: f32 = it.next()?.trim().trim_end_matches('%').parse().ok()?;
        let lp: f32 = it.next()?.trim().trim_end_matches('%').parse().ok()?;
        let rgb = hsl_to_rgb(h, sp / 100.0, lp / 100.0);
        return Some(match alpha {
            Some(a) if a < 1.0 => {
                let r = (rgb >> 16) & 0xff; let g = (rgb >> 8) & 0xff; let b = rgb & 0xff;
                (over_white(r, a) << 16) | (over_white(g, a) << 8) | over_white(b, a)
            }
            _ => rgb,
        });
    }
    named(&s.to_ascii_lowercase())
}

// Sépare la composante alpha (après une virgule en 4ᵉ position, ou après `/`).
fn split_alpha(inside: &str) -> (&str, Option<f32>) {
    if let Some(slash) = inside.find('/') {
        let a = inside[slash + 1..].trim().trim_end_matches('%').parse::<f32>().ok();
        let a = a.map(|v| if v > 1.0 { v / 100.0 } else { v });
        return (&inside[..slash], a);
    }
    // rgba(r,g,b,a) : 4 champs séparés par virgule.
    let commas: alloc::vec::Vec<usize> = inside.match_indices(',').map(|(i, _)| i).collect();
    if commas.len() == 3 {
        let last = commas[2];
        let a = inside[last + 1..].trim().parse::<f32>().ok();
        return (&inside[..last], a);
    }
    (inside, None)
}
