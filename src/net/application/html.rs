//! Rendu HTML -> texte pour le navigateur (type Lynx minimal).
//!
//! Supprime les balises, ignore `<script>`/`<style>`, insere des sauts de ligne
//! sur les elements de bloc, decode les entites HTML, extrait le `<title>` et la
//! liste des liens (`<a href>`), resolus en URL absolues.

use alloc::string::String;
use alloc::vec::Vec;

/// Page rendue en texte.
pub struct Page {
    pub title: String,
    pub lines: Vec<String>,
    pub links: Vec<String>,
}

// Decoupe une URL de base en (scheme, host) pour resoudre les liens relatifs.
fn scheme_host(base: &str) -> (&str, &str) {
    let (scheme, rest) = if let Some(r) = base.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = base.strip_prefix("http://") {
        ("http", r)
    } else {
        ("http", base)
    };
    let host = match rest.find('/') { Some(i) => &rest[..i], None => rest };
    (scheme, host)
}

fn lower_ascii(b: u8) -> u8 { b.to_ascii_lowercase() }

// Recherche insensible a la casse de `needle` dans `hay` a partir de `from`.
fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() { return None; }
    let mut i = from;
    while i + needle.len() <= hay.len() {
        let mut k = 0;
        while k < needle.len() && lower_ascii(hay[i + k]) == lower_ascii(needle[k]) { k += 1; }
        if k == needle.len() { return Some(i); }
        i += 1;
    }
    None
}

// Decode les entites HTML d'un fragment et l'ajoute a `out`.
fn push_decoded(out: &mut String, text: &str) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some(semi) = b[i + 1..].iter().position(|&c| c == b';') {
                let ent = &text[i + 1..i + 1 + semi];
                if let Some(c) = decode_entity(ent) {
                    out.push(c);
                    i += 2 + semi;
                    continue;
                }
            }
            out.push('&');
            i += 1;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
}

fn decode_entity(ent: &str) -> Option<char> {
    if let Some(num) = ent.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        return char::from_u32(code);
    }
    let c = match ent {
        "amp" => '&', "lt" => '<', "gt" => '>', "quot" => '"', "apos" => '\'',
        "nbsp" => ' ', "copy" => '(', "reg" => '(', "hellip" => '.', "mdash" => '-',
        "ndash" => '-', "rsquo" | "lsquo" => '\'', "rdquo" | "ldquo" => '"',
        "eacute" => 'e', "egrave" => 'e', "ecirc" => 'e', "agrave" => 'a', "acirc" => 'a',
        "ccedil" => 'c', "ugrave" => 'u', "ucirc" => 'u', "icirc" => 'i', "iuml" => 'i',
        "ocirc" => 'o', "euml" => 'e', "Eacute" => 'E', "Egrave" => 'E', "agrave2" => 'a',
        "laquo" => '"', "raquo" => '"', "trade" => 't', "deg" => 'o', "middot" => '.',
        "times" => 'x', "euro" => 'E', "pound" => 'L', "cent" => 'c',
        "ntilde" => 'n', "ocirc2" => 'o', "ouml" => 'o', "auml" => 'a', "uuml" => 'u',
        "Ouml" => 'O', "Auml" => 'A', "Uuml" => 'U', "szlig" => 's', "aring" => 'a',
        "aelig" => 'a', "oslash" => 'o', "ocirc3" => 'o', "atilde" => 'a', "otilde" => 'o',
        "Ccedil" => 'C', "Agrave" => 'A', "Acirc" => 'A', "Ocirc" => 'O', "Ugrave" => 'U',
        "divide" => '/', "frac12" => ' ', "frac14" => ' ', "frac34" => ' ', "sect" => 'S',
        "para" => 'P', "bull" => '*', "dagger" => '+', "permil" => '%', "micro" => 'u',
        "plusmn" => '+', "sup2" => '2', "sup3" => '3', "iquest" => '?', "iexcl" => '!',
        "shy" => '-', "ensp" | "emsp" | "thinsp" => ' ',
        _ => return None,
    };
    Some(c)
}

// Elements de bloc qui imposent un saut de ligne.
fn is_block(name: &str) -> bool {
    matches!(name,
        "p" | "div" | "br" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" |
        "li" | "ul" | "ol" | "tr" | "table" | "section" | "article" |
        "header" | "footer" | "nav" | "blockquote" | "pre" | "hr" | "form")
}

/// Convertit un corps HTML en page texte (titre, lignes, liens).
pub fn render(html: &[u8], base_url: &str) -> Page {
    let (scheme, host) = scheme_host(base_url);
    let mut title = String::new();
    let mut links: Vec<String> = Vec::new();
    let mut buf = String::new(); // texte accumule, '\n' = saut de bloc

    let mut i = 0usize;
    while i < html.len() {
        if html[i] == b'<' {
            // Commentaire ?
            if html[i..].starts_with(b"<!--") {
                i = find_ci(html, b"-->", i).map(|p| p + 3).unwrap_or(html.len());
                continue;
            }
            let end = match find_ci(html, b">", i) { Some(p) => p, None => break };
            let tag = &html[i + 1..end];
            // Nom de balise (sans '/').
            let mut name = String::new();
            for &c in tag {
                if c == b'/' { continue; }
                if c.is_ascii_alphanumeric() { name.push(lower_ascii(c) as char); } else { break; }
            }
            let closing = tag.first() == Some(&b'/');

            if name == "script" || name == "style" {
                if !closing {
                    // Saute jusqu'a la fermeture correspondante.
                    let close = if name == "script" { b"</script".as_slice() } else { b"</style".as_slice() };
                    i = find_ci(html, close, end).map(|p| {
                        find_ci(html, b">", p).map(|q| q + 1).unwrap_or(html.len())
                    }).unwrap_or(html.len());
                    continue;
                }
                i = end + 1;
                continue;
            }

            if name == "title" && !closing {
                if let Some(close) = find_ci(html, b"</title", end) {
                    let txt = core::str::from_utf8(&html[end + 1..close]).unwrap_or("");
                    push_decoded(&mut title, txt);
                    i = find_ci(html, b">", close).map(|p| p + 1).unwrap_or(html.len());
                    continue;
                }
            }

            if name == "a" && !closing {
                if let Some(href) = attr_value(tag, b"href") {
                    let abs = resolve(scheme, host, &href);
                    links.push(abs);
                    buf.push_str(" [");
                    // index 1-based
                    let n = links.len();
                    push_num(&mut buf, n);
                    buf.push_str("] ");
                }
            }

            // Image : placeholder inline avec le texte alternatif s'il existe.
            if name == "img" && !closing {
                buf.push_str(" [img");
                if let Some(alt) = attr_value(tag, b"alt") {
                    let a = alt.trim();
                    if !a.is_empty() {
                        buf.push_str(": ");
                        push_decoded(&mut buf, a);
                    }
                }
                buf.push_str("] ");
            }

            if is_block(&name) {
                buf.push('\n');
                // Marqueurs de structure sur les balises ouvrantes seulement.
                if !closing {
                    match name.as_str() {
                        "h1" => buf.push_str("# "),
                        "h2" => buf.push_str("## "),
                        "h3" => buf.push_str("### "),
                        "h4" | "h5" | "h6" => buf.push_str("#### "),
                        "li" => buf.push_str("- "),
                        "blockquote" => buf.push_str("> "),
                        _ => {}
                    }
                }
            }
            i = end + 1;
        } else {
            // Texte : jusqu'au prochain '<'.
            let start = i;
            while i < html.len() && html[i] != b'<' { i += 1; }
            let frag = core::str::from_utf8(&html[start..i]).unwrap_or("");
            // Collapse des espaces multiples (mais on garde les \n de bloc deja poses).
            let mut collapsed = String::new();
            let mut prev_space = false;
            for ch in frag.chars() {
                if ch.is_whitespace() {
                    if !prev_space { collapsed.push(' '); prev_space = true; }
                } else {
                    collapsed.push(ch);
                    prev_space = false;
                }
            }
            push_decoded(&mut buf, &collapsed);
        }
    }

    // Construit les lignes : split sur '\n', trim, collapse des lignes vides.
    let mut lines: Vec<String> = Vec::new();
    let mut blank = true;
    for raw in buf.split('\n') {
        let t = raw.trim();
        if t.is_empty() {
            if !blank { lines.push(String::new()); blank = true; }
        } else {
            lines.push(String::from(t));
            blank = false;
        }
    }
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) { lines.pop(); }

    Page { title, lines, links }
}

// Ajoute la representation decimale de `n` a `out`.
fn push_num(out: &mut String, mut n: usize) {
    if n == 0 { out.push('0'); return; }
    let mut tmp = [0u8; 20];
    let mut k = 0;
    while n > 0 { tmp[k] = b'0' + (n % 10) as u8; n /= 10; k += 1; }
    while k > 0 { k -= 1; out.push(tmp[k] as char); }
}

// Extrait la valeur d'un attribut (ex. href="...") d'une balise brute.
fn attr_value(tag: &[u8], attr: &[u8]) -> Option<String> {
    let pos = find_ci(tag, attr, 0)?;
    let mut i = pos + attr.len();
    // Espaces puis '='.
    while i < tag.len() && (tag[i] == b' ' || tag[i] == b'\t') { i += 1; }
    if i >= tag.len() || tag[i] != b'=' { return None; }
    i += 1;
    while i < tag.len() && (tag[i] == b' ' || tag[i] == b'\t') { i += 1; }
    if i >= tag.len() { return None; }
    let (val_start, quote) = if tag[i] == b'"' || tag[i] == b'\'' {
        (i + 1, Some(tag[i]))
    } else {
        (i, None)
    };
    let mut j = val_start;
    match quote {
        Some(q) => { while j < tag.len() && tag[j] != q { j += 1; } }
        None => { while j < tag.len() && tag[j] != b' ' && tag[j] != b'\t' && tag[j] != b'>' { j += 1; } }
    }
    core::str::from_utf8(&tag[val_start..j]).ok().map(String::from)
}

// Resout une URL relative (reutilise la logique de net::http).
fn resolve(scheme: &str, host: &str, location: &str) -> String {
    super::http::resolve_location(scheme, host, location)
}

/// Auto-test du rendu.
pub fn selftest() -> Result<(), &'static str> {
    let html = b"<html><head><title>Test &amp; Co</title></head><body>\
        <h1>Bonjour</h1><p>Voici un <a href=\"/page\">lien</a> et du <b>gras</b>.</p>\
        <script>var x=1;</script><p>Fin&eacute;.</p></body></html>";
    let page = render(html, "https://exemple.fr/");
    if page.title != "Test & Co" { return Err("titre"); }
    if page.links.len() != 1 || page.links[0] != "https://exemple.fr/page" { return Err("lien"); }
    let joined = page.lines.join("\n");
    if !joined.contains("Bonjour") || !joined.contains("lien") { return Err("texte"); }
    if joined.contains("var x") { return Err("script non filtre"); }
    Ok(())
}
