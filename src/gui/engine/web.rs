//! Moteur de rendu web : HTML -> DOM -> CSS (subset) -> layout flux blocs/inline
//! -> liste d'affichage truecolor peinte dans le framebuffer HD.
//!
//! Pas un navigateur complet (JS volontairement minimal, CSS partiel), mais un vrai moteur :
//! arbre DOM, feuilles de style (`<style>` + `style=""`), cascade avec
//! selecteurs simples (balise/.classe/#id), couleurs reelles, tailles de
//! police, gras, alignement, fonds de blocs, masquage (`display:none`), liens
//! cliquables, **mini-JS inline** (`document.write`, `innerHTML`) et images
//! (PNG / data:URI / fetch reseau) downscalees.

use crate::gui::framebuffer as fb;
use crate::gui::image::{self, Image};
use crate::net::http::resolve_location;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ----------------------------------------------------------------------------
// DOM
// ----------------------------------------------------------------------------

pub struct Node {
    pub tag: Option<String>,      // None => noeud texte
    pub text: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<usize>,
}

pub struct Dom { pub nodes: Vec<Node> }

impl Dom {
    fn new() -> Dom {
        Dom { nodes: alloc::vec![Node { tag: Some("#root".to_string()), text: String::new(), attrs: Vec::new(), children: Vec::new() }] }
    }
    fn push(&mut self, parent: usize, node: Node) -> usize {
        let id = self.nodes.len();
        self.nodes.push(node);
        self.nodes[parent].children.push(id);
        id
    }
}

fn is_void(tag: &str) -> bool {
    matches!(tag, "area"|"base"|"br"|"col"|"embed"|"hr"|"img"|"input"|"link"|"meta"|"param"|"source"|"track"|"wbr")
}

fn lc(b: u8) -> u8 { b.to_ascii_lowercase() }

fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() { return None; }
    let mut i = from;
    while i + needle.len() <= hay.len() {
        let mut k = 0;
        while k < needle.len() && lc(hay[i + k]) == lc(needle[k]) { k += 1; }
        if k == needle.len() { return Some(i); }
        i += 1;
    }
    None
}

fn decode_entities(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'&' {
            if let Some(semi) = b[i + 1..].iter().take(12).position(|&x| x == b';') {
                let ent = &text[i + 1..i + 1 + semi];
                if let Some(ch) = entity(ent) { out.push(ch); i += 2 + semi; continue; }
            }
            out.push('&'); i += 1;
        } else if c == b'\t' {
            out.push(' '); i += 1;
        } else if c < 0x80 {
            out.push(c as char); i += 1;
        } else {
            // sequence UTF-8 : decode le caractere complet (accents, etc.)
            let len = if c >= 0xF0 { 4 } else if c >= 0xE0 { 3 } else { 2 };
            let end = (i + len).min(b.len());
            match core::str::from_utf8(&b[i..end]).ok().and_then(|s| s.chars().next()) {
                Some(ch) => { out.push(ch); i += ch.len_utf8(); }
                None => { i += 1; }
            }
        }
    }
    out
}

fn entity(ent: &str) -> Option<char> {
    if let Some(num) = ent.strip_prefix('#') {
        let code = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(h, 16).ok()?
        } else { num.parse::<u32>().ok()? };
        // Conserve le vrai caractere Unicode ; le repli vers l'ASCII se fait au
        // moment du rendu (police bitmap).
        return char::from_u32(code);
    }
    Some(match ent {
        "amp" => '&', "lt" => '<', "gt" => '>', "quot" => '"', "apos" => '\'', "nbsp" => '\u{a0}',
        "copy" => '©', "reg" => '®', "hellip" => '…', "mdash" => '—', "ndash" => '–',
        "rsquo" => '’', "lsquo" => '‘', "rdquo" => '”', "ldquo" => '“', "laquo" => '«', "raquo" => '»',
        "eacute" => 'é', "egrave" => 'è', "ecirc" => 'ê', "euml" => 'ë',
        "agrave" => 'à', "aacute" => 'á', "acirc" => 'â', "auml" => 'ä', "aring" => 'å', "atilde" => 'ã',
        "ccedil" => 'ç', "ugrave" => 'ù', "uacute" => 'ú', "ucirc" => 'û', "uuml" => 'ü',
        "icirc" => 'î', "iuml" => 'ï', "igrave" => 'ì', "iacute" => 'í',
        "ocirc" => 'ô', "ouml" => 'ö', "ograve" => 'ò', "oacute" => 'ó', "otilde" => 'õ', "oslash" => 'ø',
        "ntilde" => 'ñ', "yacute" => 'ý', "szlig" => 'ß',
        "Eacute" => 'É', "Egrave" => 'È', "Ecirc" => 'Ê', "Agrave" => 'À', "Acirc" => 'Â',
        "Ccedil" => 'Ç', "Ouml" => 'Ö', "Uuml" => 'Ü', "Auml" => 'Ä',
        "times" => '×', "divide" => '÷', "euro" => '€', "pound" => '£', "yen" => '¥', "cent" => '¢',
        "trade" => '™', "deg" => '°', "middot" => '·', "bull" => '•', "sect" => '*', "para" => 'P',
        _ => return None,
    })
}

/// Parse un document HTML en arbre DOM (tolerant).
pub fn parse(html: &[u8]) -> Dom {
    let mut dom = Dom::new();
    let mut stack: Vec<usize> = alloc::vec![0];
    let mut i = 0usize;
    while i < html.len() {
        if html[i] == b'<' {
            if html[i..].starts_with(b"<!--") {
                i = find_ci(html, b"-->", i).map(|p| p + 3).unwrap_or(html.len());
                continue;
            }
            if i + 1 < html.len() && html[i + 1] == b'!' {
                i = find_ci(html, b">", i).map(|p| p + 1).unwrap_or(html.len());
                continue;
            }
            let end = match find_ci(html, b">", i) { Some(p) => p, None => break };
            let raw = &html[i + 1..end];
            let closing = raw.first() == Some(&b'/');
            let mut name = String::new();
            let mut p = if closing { 1 } else { 0 };
            while p < raw.len() && (raw[p] as char).is_ascii_alphanumeric() { name.push(lc(raw[p]) as char); p += 1; }
            i = end + 1;
            if name.is_empty() { continue; }

            if name == "script" || name == "style" {
                // Le contenu de <style> est conserve comme noeud texte enfant
                // (utilise par le collecteur CSS) ; <script> est jete.
                let close: &[u8] = if name == "script" { b"</script" } else { b"</style" };
                let content_start = i;
                let close_pos = find_ci(html, close, i).unwrap_or(html.len());
                if !closing {
                    if name == "style" {
                        let txt = core::str::from_utf8(&html[content_start..close_pos]).unwrap_or("");
                        let parent = *stack.last().unwrap_or(&0);
                        let sid = dom.push(parent, Node { tag: Some("style".to_string()), text: String::new(), attrs: Vec::new(), children: Vec::new() });
                        dom.push(sid, Node { tag: None, text: txt.to_string(), attrs: Vec::new(), children: Vec::new() });
                    }
                    i = find_ci(html, b">", close_pos).map(|r| r + 1).unwrap_or(html.len());
                }
                continue;
            }

            // Le contenu SVG (chemins, defs...) n'est pas rendu : on saute tout le
            // bloc <svg>...</svg> pour ne pas afficher les coordonnees des
            // `<path d="...">` en texte (cf. YouTube). Idem <noscript>/<template>.
            if name == "svg" || name == "noscript" || name == "template" {
                if !closing && !(raw.last() == Some(&b'/')) {
                    let close: &[u8] = match name.as_str() {
                        "svg" => b"</svg",
                        "noscript" => b"</noscript",
                        _ => b"</template",
                    };
                    let close_pos = find_ci(html, close, i).unwrap_or(html.len());
                    i = find_ci(html, b">", close_pos).map(|r| r + 1).unwrap_or(html.len());
                }
                continue;
            }

            if closing {
                if let Some(pos) = stack.iter().rposition(|&n| dom.nodes[n].tag.as_deref() == Some(name.as_str())) {
                    stack.truncate(pos.max(1));
                }
                continue;
            }

            let attrs = parse_attrs(&raw[p..]);
            let self_closing = raw.last() == Some(&b'/');
            let parent = *stack.last().unwrap_or(&0);
            let id = dom.push(parent, Node { tag: Some(name.clone()), text: String::new(), attrs, children: Vec::new() });
            if !is_void(&name) && !self_closing { stack.push(id); }
        } else {
            let start = i;
            while i < html.len() && html[i] != b'<' { i += 1; }
            let frag = core::str::from_utf8(&html[start..i]).unwrap_or("");
            let decoded = decode_entities(frag);
            let parent = *stack.last().unwrap_or(&0);
            if decoded.trim().is_empty() {
                if decoded.contains(|c: char| c == ' ' || c == '\n') {
                    dom.push(parent, Node { tag: None, text: " ".to_string(), attrs: Vec::new(), children: Vec::new() });
                }
            } else {
                dom.push(parent, Node { tag: None, text: decoded, attrs: Vec::new(), children: Vec::new() });
            }
        }
        if dom.nodes.len() > 60_000 { break; }
    }
    dom
}

fn parse_attrs(raw: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        while i < raw.len() && (raw[i] == b' ' || raw[i] == b'\t' || raw[i] == b'\n' || raw[i] == b'/') { i += 1; }
        let ks = i;
        while i < raw.len() && raw[i] != b'=' && raw[i] != b' ' && raw[i] != b'\t' && raw[i] != b'\n' && raw[i] != b'>' { i += 1; }
        if i == ks { break; }
        let key: String = raw[ks..i].iter().map(|&c| lc(c) as char).collect();
        let mut val = String::new();
        while i < raw.len() && (raw[i] == b' ' || raw[i] == b'\t') { i += 1; }
        if i < raw.len() && raw[i] == b'=' {
            i += 1;
            while i < raw.len() && (raw[i] == b' ' || raw[i] == b'\t') { i += 1; }
            if i < raw.len() && (raw[i] == b'"' || raw[i] == b'\'') {
                let q = raw[i]; i += 1; let vs = i;
                while i < raw.len() && raw[i] != q { i += 1; }
                val = decode_entities(core::str::from_utf8(&raw[vs..i]).unwrap_or(""));
                i += 1;
            } else {
                let vs = i;
                while i < raw.len() && raw[i] != b' ' && raw[i] != b'>' && raw[i] != b'\t' { i += 1; }
                val = decode_entities(core::str::from_utf8(&raw[vs..i]).unwrap_or(""));
            }
        }
        out.push((key, val));
    }
    out
}

fn attr<'a>(node: &'a Node, name: &str) -> Option<&'a str> {
    node.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

// ----------------------------------------------------------------------------
// CSS (subset)
// ----------------------------------------------------------------------------

#[derive(Clone)]
enum Sel { Any, Tag(String), Class(String), Id(String), TagClass(String, String) }

struct Rule { sel: Sel, decls: Vec<(String, String)>, spec: u32 }

fn parse_decls(body: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for part in body.split(';') {
        if let Some(c) = part.find(':') {
            let prop = part[..c].trim().to_ascii_lowercase();
            let val = part[c + 1..].trim().to_string();
            if !prop.is_empty() && !val.is_empty() { v.push((prop, val)); }
        }
    }
    v
}

fn parse_selector(s: &str) -> (Sel, u32) {
    let s = s.trim();
    // On ne gere qu'un selecteur simple (dernier composant si descendant).
    let last = s.split(|c: char| c == ' ' || c == '>').filter(|x| !x.is_empty()).last().unwrap_or(s);
    if last == "*" || last.is_empty() { return (Sel::Any, 0); }
    if let Some(id) = last.strip_prefix('#') { return (Sel::Id(id.to_ascii_lowercase()), 100); }
    if let Some(cl) = last.strip_prefix('.') { return (Sel::Class(cl.to_string()), 10); }
    if let Some(dot) = last.find('.') {
        let tag = last[..dot].to_ascii_lowercase();
        let cl = last[dot + 1..].to_string();
        return (Sel::TagClass(tag, cl), 11);
    }
    // pseudo/attr non geres -> match par balise si alphanumerique
    let tag: String = last.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    if tag.is_empty() { (Sel::Any, 0) } else { (Sel::Tag(tag), 1) }
}

fn parse_stylesheet(text: &str, out: &mut Vec<Rule>) {
    // Retire les commentaires /* */.
    let mut cleaned = String::new();
    let mut i = 0; let b = text.as_bytes();
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            if let Some(e) = find_ci(b, b"*/", i) { i = e + 2; continue; } else { break; }
        }
        cleaned.push(b[i] as char); i += 1;
    }
    parse_css_block(&cleaned, out);
}

// Trouve la fermeture } correspondante (compte les imbrications).
fn css_find_close(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        if b[i] == b'{' { depth += 1; }
        else if b[i] == b'}' { if depth == 1 { return Some(i); } depth -= 1; }
        i += 1;
    }
    None
}

fn parse_css_block(text: &str, out: &mut Vec<Rule>) {
    let mut pos = 0usize;
    while pos < text.len() {
        let open = match text[pos..].find('{') { Some(o) => pos + o, None => break };
        let sel_part = text[pos..open].trim();
        let close = match css_find_close(text, open) { Some(c) => c, None => break };
        let body = &text[open + 1..close];
        pos = close + 1;
        // @font-face, @keyframes: skip entirely; @media: recurse into body
        if sel_part.starts_with('@') {
            let kw = sel_part.split_whitespace().next().unwrap_or("");
            let kw_lc = kw.to_ascii_lowercase();
            if kw_lc.starts_with("@media") || kw_lc.starts_with("@supports") || kw_lc.starts_with("@layer") {
                if out.len() < 4000 { parse_css_block(body, out); }
            }
            continue;
        }
        // Regle normale: body = declarations CSS simples (pas de '{' imbriques).
        if body.contains('{') { continue; } // sous-bloc inattendu
        let decls = parse_decls(body);
        if decls.is_empty() { continue; }
        for sel in sel_part.split(',') {
            let (s, spec) = parse_selector(sel);
            out.push(Rule { sel: s, decls: decls.clone(), spec });
        }
        if out.len() > 4000 { break; }
    }
}

fn starts_ci(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() >= needle.len() && hay[..needle.len()].iter().zip(needle).all(|(a, b)| lc(*a) == lc(*b))
}

/// Pre-traitement bulletproof : retire entierement `<script>...</script>` et
/// `<style>...</style>` du flux (le contenu CSS est extrait en regles), et
/// borne la taille. Garantit qu'aucun code ne peut fuiter dans le rendu, meme
/// si le parseur DOM a un cas limite ou si le flux est partiellement corrompu.
fn extract_and_strip(html: &[u8], max_len: usize) -> (Vec<u8>, Vec<Rule>) {
    let mut out: Vec<u8> = Vec::with_capacity(html.len().min(max_len));
    let mut css: Vec<Rule> = Vec::new();
    let mut i = 0usize;
    while i < html.len() {
        if out.len() >= max_len { break; }
        if html[i] == b'<' {
            if starts_ci(&html[i..], b"<script") {
                i = find_ci(html, b"</script", i + 1)
                    .map(|p| find_ci(html, b">", p).map(|q| q + 1).unwrap_or(html.len()))
                    .unwrap_or(html.len());
                continue;
            }
            if starts_ci(&html[i..], b"<style") {
                let gt = find_ci(html, b">", i).map(|p| p + 1).unwrap_or(html.len());
                let endc = find_ci(html, b"</style", gt).unwrap_or(html.len());
                if endc > gt && endc - gt < 400_000 {
                    let content = core::str::from_utf8(&html[gt..endc]).unwrap_or("");
                    parse_stylesheet(content, &mut css);
                }
                i = find_ci(html, b">", endc).map(|p| p + 1).unwrap_or(html.len());
                continue;
            }
        }
        out.push(html[i]);
        i += 1;
    }
    (out, css)
}

/// Pipeline complet : HTML -> (JS inline) -> (CSS extrait, DOM nettoye) -> page.
pub fn render(html: &[u8], base_url: &str, width: i32) -> Page {
    let scripted = crate::gui::js::execute_inline(html);
    render_scripted(&scripted, base_url, width)
}

// Met en page un HTML deja enrichi par le JS (DOM applique).
fn render_scripted(scripted: &[u8], base_url: &str, width: i32) -> Page {
    let (clean, css) = extract_and_strip(scripted, 1_500_000);
    let dom = parse(&clean);
    layout(&dom, base_url, width, &css)
}

/// Session interactive : conserve le contexte JS (etat + DOM) d'une page pour
/// rejouer les gestionnaires `onclick` (mini-applications type calculatrice).
pub struct Session {
    ctx: crate::gui::js::PageCtx,
    base: String,
    width: i32,
}

impl Session {
    /// Ouvre une page interactive : execute le JS initial, renvoie (session, page).
    /// Reserve aux pages internes (about:calc, about:wasm...) qui embarquent des
    /// mini-applications JS.
    pub fn open(html: &[u8], base_url: &str, width: i32) -> (Session, Page) {
        let (ctx, scripted) = crate::gui::js::open_page(html);
        let page = render_scripted(&scripted, base_url, width);
        (Session { ctx, base: base_url.to_string(), width }, page)
    }
    /// Ouvre une page reseau en mode souverain : DOM + CSS + images, SANS executer
    /// le JS de la page (les SPA modernes ne peuvent pas tourner ici et ne
    /// produisent que du texte parasite). Rendu propre et lisible.
    pub fn open_static(html: &[u8], base_url: &str, width: i32) -> (Session, Page) {
        let (ctx, scripted) = crate::gui::js::open_page_static(html);
        let page = render_scripted(&scripted, base_url, width);
        (Session { ctx, base: base_url.to_string(), width }, page)
    }
    /// Rejoue un gestionnaire (code d'un lien `javascript:`) et re-rend la page.
    pub fn dispatch(&mut self, code: &str) -> Page {
        let scripted = self.ctx.dispatch(code);
        render_scripted(&scripted, &self.base, self.width)
    }
    /// Re-rend a une nouvelle largeur sans rejouer de code.
    pub fn relayout(&mut self, width: i32) -> Page {
        self.width = width;
        let html = self.ctx.html();
        render_scripted(&html, &self.base, self.width)
    }
}

fn sel_matches(sel: &Sel, tag: &str, classes: &str, id: &str) -> bool {
    match sel {
        Sel::Any => true,
        Sel::Tag(t) => t == tag,
        Sel::Id(x) => x == id,
        Sel::Class(c) => classes.split(' ').any(|cl| cl == c),
        Sel::TagClass(t, c) => t == tag && classes.split(' ').any(|cl| cl == c),
    }
}

// Couleurs --------------------------------------------------------------------

fn named_color(s: &str) -> Option<u32> {
    Some(match s {
        "black" => 0x000000, "white" => 0xffffff, "red" => 0xff0000, "green" => 0x008000,
        "blue" => 0x0000ff, "navy" => 0x000080, "gray" | "grey" => 0x808080, "silver" => 0xc0c0c0,
        "lightgray" | "lightgrey" => 0xd3d3d3, "darkgray" | "darkgrey" => 0xa9a9a9,
        "maroon" => 0x800000, "yellow" => 0xffff00, "olive" => 0x808000, "lime" => 0x00ff00,
        "aqua" | "cyan" => 0x00ffff, "teal" => 0x008080, "fuchsia" | "magenta" => 0xff00ff,
        "purple" => 0x800080, "orange" => 0xffa500, "pink" => 0xffc0cb, "brown" => 0xa52a2a,
        "gold" => 0xffd700,
        // palette etendue (web colors usuelles)
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
        "transparent" => return None,
        _ => return None,
    })
}

// HSL -> RGB (h en degres, s/l en 0..1). N'utilise que des operations dispo en
// `core` (pas de round/floor/rem_euclid, indisponibles dans ce build no_std).
fn fabsf(x: f32) -> f32 { if x < 0.0 { -x } else { x } }
fn chan(v: f32) -> u32 { let n = (v * 255.0 + 0.5) as i32; if n < 0 { 0 } else if n > 255 { 255 } else { n as u32 } }
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> u32 {
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

fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    // linear-gradient / radial-gradient → extract first color
    if s.contains("gradient(") {
        if let Some(pos) = s.find('#') {
            let end = s[pos + 1..].find(|c: char| !c.is_ascii_hexdigit())
                .map(|i| pos + 1 + i)
                .unwrap_or(s.len());
            return parse_color(&s[pos..end]);
        }
        return None;
    }
    if let Some(h) = s.strip_prefix('#') {
        let h = h.trim();
        if h.len() == 3 {
            let r = u8::from_str_radix(&h[0..1], 16).ok()?;
            let g = u8::from_str_radix(&h[1..2], 16).ok()?;
            let b = u8::from_str_radix(&h[2..3], 16).ok()?;
            return Some(((r as u32 * 17) << 16) | ((g as u32 * 17) << 8) | (b as u32 * 17));
        }
        if h.len() >= 6 {
            return u32::from_str_radix(&h[..6], 16).ok().map(|v| v & 0xffffff);
        }
        return None;
    }
    if let Some(rest) = s.strip_prefix("rgb") {
        // separateurs virgule ou espace (syntaxe CSS moderne `rgb(r g b / a)`).
        let inside = rest.trim_start_matches('a').trim_start_matches('(').trim_end_matches(')');
        let inside = inside.split('/').next().unwrap_or(inside);
        let mut it = inside.split(|c: char| c == ',' || c == ' ').filter(|x| !x.trim().is_empty()).map(|x| x.trim().trim_end_matches('%'));
        let r: u32 = it.next()?.parse::<f32>().ok()? as u32;
        let g: u32 = it.next()?.parse::<f32>().ok()? as u32;
        let b: u32 = it.next()?.parse::<f32>().ok()? as u32;
        return Some(((r & 255) << 16) | ((g & 255) << 8) | (b & 255));
    }
    if let Some(rest) = s.strip_prefix("hsl") {
        let inside = rest.trim_start_matches('a').trim_start_matches('(').trim_end_matches(')');
        let inside = inside.split('/').next().unwrap_or(inside);
        let mut it = inside.split(|c: char| c == ',' || c == ' ').filter(|x| !x.trim().is_empty());
        let h: f32 = it.next()?.trim().trim_end_matches("deg").parse().ok()?;
        let sp: f32 = it.next()?.trim().trim_end_matches('%').parse().ok()?;
        let lp: f32 = it.next()?.trim().trim_end_matches('%').parse().ok()?;
        return Some(hsl_to_rgb(h, sp / 100.0, lp / 100.0));
    }
    named_color(&s.to_ascii_lowercase())
}

fn font_px(s: &str) -> Option<i32> {
    let s = s.trim();
    match s {
        "xx-small" => return Some(10), "x-small" => return Some(12), "small" => return Some(13),
        "medium" => return Some(16), "large" => return Some(20), "x-large" => return Some(26),
        "xx-large" => return Some(33), _ => {}
    }
    if let Some(px) = s.strip_suffix("px") { return px.trim().parse::<f32>().ok().map(|v| v as i32); }
    if let Some(em) = s.strip_suffix("em") { return em.trim().parse::<f32>().ok().map(|v| (v * 16.0) as i32); }
    if let Some(pt) = s.strip_suffix("pt") { return pt.trim().parse::<f32>().ok().map(|v| (v * 4.0 / 3.0) as i32); }
    s.parse::<f32>().ok().map(|v| v as i32)
}

fn px_to_scale(px: i32) -> usize {
    if px >= 30 { 4 } else if px >= 22 { 3 } else if px >= 15 { 2 } else { 1 }
}

// ----------------------------------------------------------------------------
// Style calcule
// ----------------------------------------------------------------------------

// Proprietes de texte heritees (cascade).
#[derive(Clone)]
struct Style {
    color: u32,
    scale: usize,
    bold: bool,
    align: u8,        // 0 gauche, 1 centre, 2 droite
    href: Option<String>,
    pre: bool,        // white-space: pre (conserve espaces/sauts)
}

fn default_style() -> Style { Style { color: 0x202124, scale: 2, bold: false, align: 0, href: None, pre: false } }

#[derive(Clone, Copy, PartialEq)]
enum Disp { Block, Inline, InlineBlock, Flex, None }
#[derive(Clone, Copy, PartialEq)]
enum FloatK { None, Left, Right }
#[derive(Clone, Copy)]
enum Len { Px(i32), Pct(i32) }
impl Len { fn resolve(self, avail: i32) -> i32 { match self { Len::Px(p) => p, Len::Pct(p) => (avail * p / 100).max(0) } } }

fn parse_len(s: &str) -> Option<Len> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') { return p.trim().parse::<f32>().ok().map(|v| Len::Pct(v as i32)); }
    font_px(s).map(Len::Px)
}

// Proprietes de boite, propres a l'element (non heritees).
struct BoxProps {
    hidden: bool,
    bg: Option<u32>,
    width: Option<Len>,
    height: Option<Len>,
    max_width: Option<Len>,
    min_width: Option<Len>,
    min_height: Option<Len>,
    center: bool,          // margin:auto (centre le bloc)
    disp: Option<Disp>,
    float: FloatK,
    pad: i32,              // padding uniforme (px) — espace interieur
    mar_y: i32,            // marge verticale (haut/bas, px)
    border_w: i32,         // epaisseur de bordure (px)
    border_color: u32,     // couleur de bordure
}
fn default_box() -> BoxProps {
    BoxProps { hidden: false, bg: None, width: None, height: None, max_width: None, min_width: None, min_height: None,
        center: false, disp: None, float: FloatK::None, pad: 0, mar_y: 0, border_w: 0, border_color: 0x000000 }
}

// Premiere longueur d'une valeur raccourcie (`10px 20px` -> 10).
fn first_len(val: &str) -> Option<i32> {
    val.split_whitespace().find_map(|t| parse_len(t).map(|l| l.resolve(0)))
}

// ----------------------------------------------------------------------------
// Liste d'affichage
// ----------------------------------------------------------------------------

pub enum Item {
    Rect { x: i32, y: i32, w: i32, h: i32, color: u32 },
    Text { x: i32, y: i32, s: String, color: u32, scale: usize, bold: bool },
    Image { x: i32, y: i32, w: i32, h: i32, idx: usize },
}

pub struct Link { pub x: i32, pub y: i32, pub w: i32, pub h: i32, pub href: String }

pub struct Page {
    pub title: String,
    pub items: Vec<Item>,
    pub links: Vec<Link>,
    pub images: Vec<Image>,
    pub height: i32,
    pub bg: u32,
}

const PAD: i32 = 8;

// Element d'une ligne en cours (positions relatives au debut de ligne).
enum LineItem {
    Word { dx: i32, w: i32, s: String, color: u32, scale: usize, bold: bool, href: Option<String> },
    Img { dx: i32, w: i32, h: i32, idx: usize },
    Box { dx: i32, w: i32, h: i32, fill: u32, value: String },
    Frag { dx: i32, w: i32, h: i32, items: Vec<Item>, links: Vec<Link> },
}

// Fragment mis en page dans son propre espace (coordonnees relatives a 0,0).
struct Frag { items: Vec<Item>, links: Vec<Link>, width: i32, height: i32 }

// Etat partage entre la page et tous les sous-fragments (images, budget, etc.).
struct Ctx<'a> {
    css: &'a [Rule],
    css_vars: Vec<(String, String)>,
    images: Vec<Image>,
    img_cache: Vec<(String, usize)>,
    img_budget: u32,
    scheme: String,
    host: String,
    title: String,
    visited: usize,
}

impl<'a> Ctx<'a> {
    // Charge une image (data:URI ou reseau), downscale, renvoie son index.
    fn load_image(&mut self, src: &str, max_w: usize, max_h: usize) -> Option<usize> {
        if let Some(&(_, idx)) = self.img_cache.iter().find(|(u, _)| u == src) { return Some(idx); }
        if self.img_budget == 0 { return None; }
        let raw: Vec<u8>;
        if let Some(rest) = src.strip_prefix("data:") {
            let comma = rest.find(',')?;
            let meta = &rest[..comma];
            let data = &rest[comma + 1..];
            raw = if meta.contains("base64") { base64_decode(data) } else { data.bytes().collect() };
        } else {
            let abs = resolve_location(&self.scheme, &self.host, src);
            let doc = crate::net::fetch_document(&abs);
            if !doc.ok || doc.body.is_empty() { return None; }
            self.img_budget -= 1;
            raw = doc.body;
        }
        let img = image::decode(&raw)?;
        let mh = if max_h == 0 { img.h.max(1) } else { max_h };
        let img = image::downscale(&img, max_w.max(16), mh.max(16));
        if img.w == 0 || img.h == 0 { return None; }
        let idx = self.images.len();
        self.images.push(img);
        self.img_cache.push((src.to_string(), idx));
        Some(idx)
    }
}

// Layouteur de flux : remplit `items`/`links` en coordonnees locales (origine
// 0,0). `x0` = decalage gauche (indentation), `avail` = largeur de contenu.
struct Flow<'c, 'a> {
    ctx: &'c mut Ctx<'a>,
    items: Vec<Item>,
    links: Vec<Link>,
    x0: i32,
    avail: i32,
    y: i32,
    line: Vec<LineItem>,
    line_h: i32,
    align: u8,
    used_w: i32,
}

const LINE_GAP: i32 = 6;
fn base_line_h() -> i32 { 8 * 2 + LINE_GAP }

impl<'c, 'a> Flow<'c, 'a> {
    fn new(ctx: &'c mut Ctx<'a>, x0: i32, avail: i32) -> Flow<'c, 'a> {
        Flow { ctx, items: Vec::new(), links: Vec::new(), x0, avail: avail.max(16), y: 0, line: Vec::new(), line_h: base_line_h(), align: 0, used_w: 0 }
    }

    fn line_cursor(&self) -> i32 {
        self.line.iter().map(|it| match it {
            LineItem::Word { dx, w, .. } | LineItem::Img { dx, w, .. } | LineItem::Box { dx, w, .. } | LineItem::Frag { dx, w, .. } => dx + w,
        }).max().unwrap_or(0)
    }

    fn flush_line(&mut self) {
        if self.line.is_empty() { self.y += self.line_h; self.line_h = base_line_h(); return; }
        let used = self.line_cursor();
        if used > self.used_w { self.used_w = used; }
        let off = match self.align {
            1 => ((self.avail - used) / 2).max(0),
            2 => (self.avail - used).max(0),
            _ => 0,
        };
        let base_x = self.x0 + off;
        let y = self.y;
        let lh = self.line_h;
        let line = core::mem::take(&mut self.line);
        for it in line {
            match it {
                LineItem::Word { dx, w, s, color, scale, bold, href } => {
                    let tx = base_x + dx;
                    let ty = y + (lh - 8 * scale as i32).max(0);
                    if let Some(h) = href { self.links.push(Link { x: tx, y, w, h: lh, href: h }); }
                    self.items.push(Item::Text { x: tx, y: ty, s, color, scale, bold });
                }
                LineItem::Img { dx, w, h, idx } => {
                    self.items.push(Item::Image { x: base_x + dx, y, w, h, idx });
                }
                LineItem::Box { dx, w, h, fill, value } => {
                    self.items.push(Item::Rect { x: base_x + dx, y, w, h, color: 0x9aa0a6 });
                    self.items.push(Item::Rect { x: base_x + dx + 1, y: y + 1, w: (w - 2).max(0), h: (h - 2).max(0), color: fill });
                    if !value.is_empty() {
                        self.items.push(Item::Text { x: base_x + dx + 3, y: y + 3, s: value, color: 0x202124, scale: 2, bold: false });
                    }
                }
                LineItem::Frag { dx, items, links, .. } => {
                    let ox = base_x + dx;
                    for mut sub in items { translate_item(&mut sub, ox, y); self.items.push(sub); }
                    for mut lk in links { lk.x += ox; lk.y += y; self.links.push(lk); }
                }
            }
        }
        self.y += lh;
        self.line_h = base_line_h();
    }

    fn push_word(&mut self, s: &str, st: &Style) {
        // Largeurs PROPORTIONNELLES via la police vectorielle (repli monospace).
        let px = 8 * st.scale as i32;
        let cw = super::font_ttf::char_width(' ', px).max(1); // espace inter-mots
        let wpx = super::font_ttf::text_width(s, px);
        let lh = 8 * st.scale as i32 + LINE_GAP;
        if lh > self.line_h { self.line_h = lh; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += cw; }
        if cur + wpx > self.avail && cur > 0 { self.flush_line(); cur = 0; if lh > self.line_h { self.line_h = lh; } }
        self.line.push(LineItem::Word { dx: cur, w: wpx, s: s.to_string(), color: st.color, scale: st.scale, bold: st.bold, href: st.href.clone() });
    }

    fn push_text(&mut self, text: &str, st: &Style) {
        if st.pre {
            // white-space: pre — conserve les sauts de ligne et les espaces.
            for (n, segline) in text.split('\n').enumerate() {
                if n > 0 { self.flush_line(); }
                if !segline.is_empty() { self.push_word(&segline.replace('\t', "    "), st); }
            }
            return;
        }
        for w in text.split(|c: char| c == ' ' || c == '\n' || c == '\t' || c == '\r') {
            if !w.is_empty() { self.push_word(w, st); }
            if self.items.len() + self.line.len() > 90_000 { return; }
        }
    }

    fn push_image(&mut self, idx: usize) {
        let (iw, ih) = { let im = &self.ctx.images[idx]; (im.w as i32, im.h as i32) };
        let lh = ih + 4;
        if lh > self.line_h { self.line_h = lh; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += 8; }
        if cur + iw > self.avail && cur > 0 { self.flush_line(); cur = 0; if lh > self.line_h { self.line_h = lh; } }
        self.line.push(LineItem::Img { dx: cur, w: iw, h: ih, idx });
    }

    fn push_box(&mut self, w: i32, h: i32, fill: u32, value: String) {
        if h > self.line_h { self.line_h = h; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += 16; }
        if cur + w > self.avail && cur > 0 { self.flush_line(); cur = 0; }
        self.line.push(LineItem::Box { dx: cur, w, h, fill, value });
    }

    // Place un fragment (inline-block / float / element flex) sur la ligne.
    fn push_frag(&mut self, frag: Frag, gap: i32) {
        if frag.height > self.line_h { self.line_h = frag.height; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += gap; }
        if cur + frag.width > self.avail && cur > 0 { self.flush_line(); cur = 0; if frag.height > self.line_h { self.line_h = frag.height; } }
        self.line.push(LineItem::Frag { dx: cur, w: frag.width, h: frag.height, items: frag.items, links: frag.links });
    }
}

fn translate_item(it: &mut Item, dx: i32, dy: i32) {
    match it {
        Item::Rect { x, y, .. } | Item::Text { x, y, .. } | Item::Image { x, y, .. } => { *x += dx; *y += dy; }
    }
}

fn base64_decode(s: &str) -> Vec<u8> {
    fn val(c: u8) -> i32 {
        match c { b'A'..=b'Z' => (c - b'A') as i32, b'a'..=b'z' => (c - b'a' + 26) as i32,
                  b'0'..=b'9' => (c - b'0' + 52) as i32, b'+' => 62, b'/' => 63, _ => -1 }
    }
    let mut out = Vec::new();
    let mut acc = 0i32; let mut nbits = 0;
    for &c in s.as_bytes() {
        let v = val(c);
        if v < 0 { continue; }
        acc = (acc << 6) | v; nbits += 6;
        if nbits >= 8 { nbits -= 8; out.push((acc >> nbits) as u8); }
    }
    out
}

fn block_tag(t: &str) -> bool {
    matches!(t, "p"|"div"|"h1"|"h2"|"h3"|"h4"|"h5"|"h6"|"ul"|"ol"|"li"|"section"|"article"|
        "header"|"footer"|"nav"|"main"|"aside"|"blockquote"|"pre"|"figure"|"table"|"tr"|
        "form"|"address"|"fieldset"|"dl"|"dt"|"dd"|"title"|"body"|"html"|"head"|"center")
}

fn heading_scale(t: &str) -> Option<usize> {
    match t { "h1" => Some(4), "h2" => Some(3), "h3" => Some(3), "h4" | "h5" | "h6" => Some(2), _ => None }
}

/// Heuristique : un noeud texte ressemble-t-il a du code/CSS/JSON ayant fuite ?
/// Sert de garde-fou — la prose normale ne declenche aucun de ces marqueurs.
fn looks_like_code(text: &str) -> bool {
    let t = text.trim();
    if t.len() < 12 { return false; }
    // Marqueurs JS/JSON quasi-absents du texte lisible.
    for m in ["document.write", "requireLazy", "function(", "=>", "]]]", "})(", "});",
              "__d(", "rsrc.php", "Preloader_{", "RelayPreloader", ");}", "void 0", "!function"] {
        if t.contains(m) { return true; }
    }
    // CSS : forte densite de { } ; : (selecteurs + declarations).
    let braces = t.bytes().filter(|&b| b == b'{' || b == b'}').count();
    let semis = t.bytes().filter(|&b| b == b';').count();
    let colons = t.bytes().filter(|&b| b == b':').count();
    if braces >= 2 && semis >= 3 && colons >= 2 { return true; }
    false
}

/// Construit la page a partir du DOM (+ regles CSS pre-extraites).
fn layout(dom: &Dom, base_url: &str, width: i32, css: &[Rule]) -> Page {
    let (scheme, host) = scheme_host(base_url);
    let mut bg = 0xffffff_u32;
    let mut css_vars: Vec<(String, String)> = Vec::new();
    for r in css {
        if matches!(&r.sel, Sel::Tag(t) if t == "body" || t == "html")
            || matches!(&r.sel, Sel::Any) {
            for (p, v) in &r.decls {
                if p == "background" || p == "background-color" {
                    if let Some(c) = parse_color(v.split(' ').next().unwrap_or(v)) { bg = c; }
                }
                if p.starts_with("--") { css_vars.push((p.clone(), v.clone())); }
            }
        }
    }
    let mut ctx = Ctx {
        css, css_vars, images: Vec::new(), img_cache: Vec::new(), img_budget: 24,
        scheme: scheme.to_string(), host: host.to_string(), title: String::new(), visited: 0,
    };
    let content_w = (width - 2 * PAD).max(40);
    let mut f = Flow::new(&mut ctx, PAD, content_w);
    f.y = PAD;
    walk(&mut f, dom, 0, &default_style(), 0);
    f.flush_line();
    let height = f.y + PAD;
    let items = core::mem::take(&mut f.items);
    let links = core::mem::take(&mut f.links);
    drop(f);
    Page { title: ctx.title, items, links, images: ctx.images, height, bg }
}

// Resout les variables CSS var(--name) dans une valeur.
fn resolve_var(val: &str, css_vars: &[(String, String)]) -> String {
    if !val.contains("var(") { return val.to_string(); }
    let mut result = String::new();
    let b = val.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if i + 4 <= b.len() && &b[i..i+4] == b"var(" {
            let start = i + 4;
            let mut depth = 1usize;
            let mut j = start;
            while j < b.len() && depth > 0 {
                if b[j] == b'(' { depth += 1; } else if b[j] == b')' { depth -= 1; }
                if depth > 0 { j += 1; } else { break; }
            }
            let inner = val[start..j].trim();
            let (varname, fallback) = if let Some(ci) = inner.find(',') {
                (inner[..ci].trim(), Some(inner[ci + 1..].trim()))
            } else { (inner, None) };
            if let Some((_, v)) = css_vars.iter().find(|(n, _)| n == varname) {
                result.push_str(v);
            } else if let Some(fb) = fallback {
                result.push_str(fb);
            }
            i = j + 1;
        } else { result.push(b[i] as char); i += 1; }
    }
    result
}

fn apply_decls(decls: &[(String, String)], st: &mut Style, bx: &mut BoxProps, css_vars: &[(String, String)]) {
    for (p, v) in decls {
        if p.starts_with("--") { continue; } // proprietes CSS custom (deja collectees)
        let resolved;
        let val = if v.contains("var(") { resolved = resolve_var(v, css_vars); resolved.trim() } else { v.trim() };
        match p.as_str() {
            "color" => { if let Some(c) = parse_color(val) { st.color = c; } }
            "background" | "background-color" => {
                if let Some(c) = parse_color(val.split(' ').next().unwrap_or(val)) { bx.bg = Some(c); }
            }
            "font-size" => { if let Some(px) = font_px(val) { st.scale = px_to_scale(px); } }
            "font-weight" => { if val == "bold" || val == "bolder" || val == "700" || val == "800" || val == "900" { st.bold = true; } else if val == "normal" || val == "400" { st.bold = false; } }
            "text-align" => { st.align = match val { "center" => 1, "right" => 2, _ => 0 }; }
            "white-space" => { if val.starts_with("pre") { st.pre = true; } }
            "display" => {
                let d = match val { "none" => Disp::None, "inline" => Disp::Inline, "inline-block" => Disp::InlineBlock,
                    "flex" | "inline-flex" | "grid" | "inline-grid" => Disp::Flex, _ => Disp::Block };
                if d == Disp::None { bx.hidden = true; }
                bx.disp = Some(d);
            }
            "visibility" => { if val == "hidden" { bx.hidden = true; } }
            "width" => { bx.width = parse_len(val); }
            "height" => { bx.height = parse_len(val); }
            "max-width" => { bx.max_width = parse_len(val); }
            "min-width" => { bx.min_width = parse_len(val); }
            "min-height" => { bx.min_height = parse_len(val); }
            "float" => { bx.float = match val { "left" => FloatK::Left, "right" => FloatK::Right, _ => FloatK::None }; }
            "margin" => {
                if val.contains("auto") { bx.center = true; }
                if let Some(px) = first_len(val) { bx.mar_y = px.max(0); }
            }
            "margin-left" | "margin-right" => { if val.contains("auto") { bx.center = true; } }
            "margin-top" | "margin-bottom" => { if let Some(px) = first_len(val) { bx.mar_y = bx.mar_y.max(px.max(0)); } }
            "padding" | "padding-top" | "padding-bottom" | "padding-left" | "padding-right" => {
                if let Some(px) = first_len(val) { bx.pad = bx.pad.max(px.max(0)); }
            }
            // `border: 1px solid #ccc` -> epaisseur + couleur ; le style (solid...) est ignore.
            "border" | "border-width" | "border-top" | "border-bottom" | "border-left" | "border-right" => {
                for tok in val.split_whitespace() {
                    if let Some(l) = parse_len(tok) { let w = l.resolve(0); if w > 0 { bx.border_w = w; } }
                    else if let Some(c) = parse_color(tok) { bx.border_color = c; if bx.border_w == 0 { bx.border_w = 1; } }
                }
            }
            "border-color" => { if let Some(c) = parse_color(val) { bx.border_color = c; if bx.border_w == 0 { bx.border_w = 1; } } }
            "border-style" => { if val != "none" && bx.border_w == 0 { bx.border_w = 1; } }
            "opacity" => { if val == "0" { bx.hidden = true; } }
            _ => {}
        }
    }
}

// Calcule le style herite + la boite de l'element (cascade CSS + style inline).
fn compute(f: &Flow, node: &Node, tag: &str, st: &Style) -> (Style, BoxProps) {
    let mut cst = st.clone();
    let mut bx = default_box();
    if let Some(s) = heading_scale(tag) { cst.scale = s; cst.bold = true; }
    if matches!(tag, "b" | "strong" | "th") { cst.bold = true; }
    if tag == "center" { cst.align = 1; }
    if tag == "pre" { cst.pre = true; }
    if tag == "a" {
        if let Some(href) = attr(node, "href") {
            if let Some(code) = href.strip_prefix("javascript:") {
                cst.href = Some(alloc::format!("javascript:{}", code)); // lien-action (non resolu)
                cst.color = 0x1a0dab;
            } else {
                cst.href = Some(resolve_location(&f.ctx.scheme, &f.ctx.host, href));
                cst.color = 0x1a0dab;
            }
        }
    }
    // onclick (boutons et autres elements interactifs) -> action JS cliquable.
    if cst.href.is_none() {
        if let Some(code) = attr(node, "onclick") {
            cst.href = Some(alloc::format!("javascript:{}", code));
            if tag == "button" { cst.color = 0x1a0dab; }
        }
    }
    let classes = attr(node, "class").unwrap_or("").to_string();
    let id = attr(node, "id").unwrap_or("").to_ascii_lowercase();
    let mut matched: Vec<&Rule> = f.ctx.css.iter().filter(|r| sel_matches(&r.sel, tag, &classes, &id)).collect();
    matched.sort_by_key(|r| r.spec);
    for r in matched { apply_decls(&r.decls, &mut cst, &mut bx, &f.ctx.css_vars); }
    if let Some(style) = attr(node, "style") { apply_decls(&parse_decls(style), &mut cst, &mut bx, &f.ctx.css_vars); }
    (cst, bx)
}

fn walk(f: &mut Flow, dom: &Dom, idx: usize, st: &Style, depth: u32) {
    f.ctx.visited += 1;
    if f.ctx.visited > 300_000 || depth > 200 || f.items.len() > 80_000 { return; }
    let node = &dom.nodes[idx];
    if node.tag.is_none() {
        // Garde-fou : ne jamais peindre du code/CSS qui aurait fuite en texte
        // (ex. contenu de <script> apres un `</script>` imbrique dans une chaine).
        if !node.text.is_empty() && !looks_like_code(&node.text) { f.push_text(&node.text, st); }
        return;
    }
    let tag = node.tag.as_deref().unwrap_or("");
    if tag == "style" || tag == "script" { return; }
    if tag == "head" { for &c in &node.children { walk(f, dom, c, st, depth + 1); } return; }
    if tag == "title" {
        let mut t = String::new();
        for &c in &node.children { if dom.nodes[c].tag.is_none() { t.push_str(&dom.nodes[c].text); } }
        f.ctx.title = t.trim().to_string();
        return;
    }

    let (cst, bx) = compute(f, node, tag, st);
    if bx.hidden { return; }

    // --- elements speciaux ---
    if tag == "br" { f.flush_line(); return; }
    if tag == "hr" {
        f.flush_line();
        f.items.push(Item::Rect { x: f.x0, y: f.y + 4, w: f.avail, h: 2, color: 0xcccccc });
        f.y += 12;
        return;
    }
    if tag == "img" {
        let attr_w = attr(node, "width").and_then(|s| s.trim_end_matches("px").parse::<i32>().ok()).unwrap_or(0);
        let attr_h = attr(node, "height").and_then(|s| s.trim_end_matches("px").parse::<i32>().ok()).unwrap_or(0);
        let maxw = if attr_w > 0 { attr_w.clamp(16, f.avail) } else { bx.width.map(|l| l.resolve(f.avail)).unwrap_or(f.avail) }.max(16) as usize;
        let maxh = if attr_h > 0 { attr_h.min(2000) } else { bx.height.map(|l| l.resolve(0)).unwrap_or(1600) }.max(16) as usize;
        if let Some(src) = attr(node, "src") {
            if let Some(i) = f.ctx.load_image(src, maxw, maxh) { f.push_image(i); return; }
        }
        // Placeholder : boite grise avec texte alt
        let alt = attr(node, "alt").unwrap_or("image");
        let ph_w = (maxw as i32).clamp(24, f.avail);
        let ph_h = if attr_h > 0 { attr_h.min(300).max(24) } else { 48 };
        f.push_box(ph_w, ph_h, 0xe8e8e8, alt.to_string());
        return;
    }
    if tag == "input" || tag == "textarea" || tag == "select" {
        let cw = 8 * 2;
        let h = 8 * 2 + 8;
        let w = bx.width.map(|l| l.resolve(f.avail)).unwrap_or((16 * cw).min(f.avail / 2)).clamp(cw, f.avail);
        let val = attr(node, "value").unwrap_or("").to_string();
        f.push_box(w, h, 0xffffff, val);
        return;
    }

    // Mode d'affichage : explicite (CSS) sinon defaut par balise. tr -> flex.
    let mut disp = bx.disp.unwrap_or(if block_tag(tag) { Disp::Block } else { Disp::Inline });
    if tag == "tr" { disp = Disp::Flex; }
    if bx.float != FloatK::None && disp == Disp::Block { disp = Disp::InlineBlock; }

    match disp {
        Disp::None => {}
        Disp::Inline => { for &c in &node.children { walk(f, dom, c, &cst, depth + 1); } }
        Disp::InlineBlock => {
            let w = bx.width.map(|l| l.resolve(f.avail)).unwrap_or(f.avail).clamp(8, f.avail);
            let frag = make_frag(f, dom, node, &cst, &bx, tag, w, !bx.width.is_some(), depth + 1);
            f.push_frag(frag, 8);
        }
        Disp::Flex => { f.flush_line(); flex_layout(f, dom, node, &cst, depth); f.y += LINE_GAP; }
        Disp::Block => { block_layout(f, dom, node, &cst, &bx, tag, depth); }
    }
}

// Met en page le contenu d'un element dans son propre fragment (largeur `w`).
// `shrink` : ajuste la largeur finale au contenu (inline-block sans width).
fn make_frag(f: &mut Flow, dom: &Dom, node: &Node, cst: &Style, bx: &BoxProps, tag: &str, w: i32, shrink: bool, depth: u32) -> Frag {
    let mut sub = Flow::new(f.ctx, 0, w);
    sub.align = cst.align;
    if tag == "li" { let b = Style { color: 0x5f6368, ..cst.clone() }; sub.push_word("*", &b); }
    for &c in &node.children { walk(&mut sub, dom, c, cst, depth + 1); }
    sub.flush_line();
    let used = sub.used_w.clamp(1, w);
    let h = sub.y;
    let mut items = core::mem::take(&mut sub.items);
    let links = core::mem::take(&mut sub.links);
    drop(sub);
    let fw = if shrink { used } else { w };
    if let Some(c) = bx.bg {
        let mut v = alloc::vec![Item::Rect { x: 0, y: 0, w: fw, h: h.max(1), color: c }];
        v.extend(items);
        items = v;
    }
    Frag { items, links, width: fw, height: h }
}

// Bloc en flux normal : nouvelle ligne, largeur eventuellement contrainte
// (width/max-width) et centree (margin:auto), fond, marges verticales.
fn block_layout(f: &mut Flow, dom: &Dom, node: &Node, cst: &Style, bx: &BoxProps, tag: &str, depth: u32) {
    f.flush_line();
    if heading_scale(tag).is_some() && f.y > PAD { f.y += LINE_GAP; }

    // Marge haute (box model).
    f.y += bx.mar_y;

    // Largeur de la boite bordure incluse.
    let mut cw = f.avail;
    if let Some(wv) = bx.width { cw = wv.resolve(f.avail); }
    if let Some(mw) = bx.max_width { let m = mw.resolve(f.avail); if cw > m { cw = m; } }
    if let Some(mn) = bx.min_width { let m = mn.resolve(f.avail); if cw < m { cw = m; } }
    cw = cw.clamp(8, f.avail);

    let indent = match tag { "ul" | "ol" | "blockquote" | "dl" | "dd" => 18, _ => 0 };
    let mut left = indent;
    if cw < f.avail - indent && bx.center { left = indent + (f.avail - indent - cw) / 2; }
    let outer = cw.min(f.avail - left).max(8);

    let bw = bx.border_w.max(0);
    let pad = bx.pad.max(0);
    // Largeur du CONTENU = boite - bordures - paddings (gauche + droite).
    let inner = (outer - 2 * (bw + pad)).max(8);

    let box_top = f.y;
    let bg_insert = f.items.len();
    let (sx0, sav, sal) = (f.x0, f.avail, f.align);
    f.x0 = sx0 + left + bw + pad;   // contenu insere par bordure + padding
    f.avail = inner;
    f.align = cst.align;
    f.y += bw + pad;                // bordure + padding du haut

    if tag == "li" { let b = Style { color: 0x5f6368, ..cst.clone() }; f.push_word("*", &b); }
    for &c in &node.children { walk(f, dom, c, cst, depth + 1); }
    f.flush_line();

    f.y += pad + bw;               // bordure + padding du bas
    // height / min-height contraignent la hauteur finale de la boite.
    if let Some(hv) = bx.height { let target = box_top + hv.resolve(0); if f.y < target { f.y = target; } }
    if let Some(mn) = bx.min_height { let target = box_top + mn.resolve(0); if f.y < target { f.y = target; } }
    let box_bottom = f.y;
    f.x0 = sx0; f.avail = sav; f.align = sal;

    let h = (box_bottom - box_top).max(0);
    // Fond : remplit toute la boite (insere SOUS le contenu).
    if let Some(bgc) = bx.bg {
        if h > 0 { f.items.insert(bg_insert, Item::Rect { x: sx0 + left, y: box_top, w: outer, h, color: bgc }); }
    }
    // Bordure : 4 traits dessines PAR-DESSUS (haut, bas, gauche, droite).
    if bw > 0 && h > 0 {
        let x = sx0 + left;
        let bc = bx.border_color;
        f.items.push(Item::Rect { x, y: box_top, w: outer, h: bw, color: bc });
        f.items.push(Item::Rect { x, y: box_bottom - bw, w: outer, h: bw, color: bc });
        f.items.push(Item::Rect { x, y: box_top, w: bw, h, color: bc });
        f.items.push(Item::Rect { x: x + outer - bw, y: box_top, w: bw, h, color: bc });
    }

    // Marge basse + espacement par defaut entre blocs.
    f.y += bx.mar_y;
    if matches!(tag, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "blockquote" | "table" |
        "form" | "div" | "section" | "article" | "header" | "footer" | "li" | "tr" | "figure") { f.y += LINE_GAP; }
}

// Conteneur flex / ligne de tableau : enfants elements places cote a cote,
// largeur repartie en colonnes egales (avec retour a la ligne si depassement).
fn flex_layout(f: &mut Flow, dom: &Dom, node: &Node, cst: &Style, depth: u32) {
    let kids: Vec<usize> = node.children.iter().cloned().filter(|&c| dom.nodes[c].tag.is_some()).collect();
    if kids.is_empty() {
        let saved = f.align; f.align = cst.align;
        for &c in &node.children { walk(f, dom, c, cst, depth + 1); }
        f.flush_line(); f.align = saved;
        return;
    }
    let gap = 8;
    let n = kids.len() as i32;
    let colw = ((f.avail - gap * (n - 1)) / n).clamp(24, f.avail);
    for &c in &kids {
        let mut sub = Flow::new(f.ctx, 0, colw);
        walk(&mut sub, dom, c, cst, depth + 1);
        sub.flush_line();
        let h = sub.y;
        let items = core::mem::take(&mut sub.items);
        let links = core::mem::take(&mut sub.links);
        drop(sub);
        f.push_frag(Frag { items, links, width: colw, height: h }, gap);
    }
    f.flush_line();
}

fn scheme_host(base: &str) -> (&str, &str) {
    let (scheme, rest) = if let Some(r) = base.strip_prefix("https://") { ("https", r) }
        else if let Some(r) = base.strip_prefix("http://") { ("http", r) }
        else { ("http", base) };
    let host = match rest.find('/') { Some(i) => &rest[..i], None => rest };
    (scheme, host)
}

// ----------------------------------------------------------------------------
// Peinture
// ----------------------------------------------------------------------------

pub fn paint(page: &Page, scroll: i32, bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect_rgb(bx, by, bw, bh, page.bg);
    let bxi = bx as i32; let byi = by as i32; let bwi = bw as i32; let bhi = bh as i32;
    for it in &page.items {
        match it {
            Item::Rect { x, y, w, h, color } => {
                let sy = byi + y - scroll;
                if sy + h <= byi || sy >= byi + bhi { continue; }
                let yy = sy.max(byi);
                let hh = (sy + h).min(byi + bhi) - yy;
                let xx = bxi + x;
                let ww = (*w).min(bwi - x).max(0);
                if hh > 0 && ww > 0 && xx >= bxi {
                    fb::fill_rect_rgb(xx as usize, yy as usize, ww as usize, hh as usize, *color);
                }
            }
            Item::Text { x, y, s, color, scale, bold } => {
                let sy = byi + y - scroll;
                let h = 8 * *scale as i32;
                if sy < byi || sy + h > byi + bhi { continue; }
                let xx = bxi + x;
                if xx >= bxi && xx < bxi + bwi {
                    // Police vectorielle antialiasee si dispo ; sinon repli bitmap.
                    let px = 8 * *scale as i32;
                    if !super::font_ttf::draw_text(xx, sy, s, *color, px, *bold) {
                        fb::draw_text_rgb(xx as usize, sy as usize, s, *color, *scale);
                        if *bold { fb::draw_text_rgb((xx + 1) as usize, sy as usize, s, *color, *scale); }
                    }
                }
            }
            Item::Image { x, y, w: _w, h, idx } => {
                let sy = byi + y - scroll;
                if sy + h <= byi || sy >= byi + bhi { continue; }
                if let Some(img) = page.images.get(*idx) {
                    let xx = bxi + x;
                    if xx >= bxi {
                        let skip = (byi - sy).max(0) as usize;
                        let draw_h = img.h.saturating_sub(skip);
                        let start = skip.saturating_mul(img.w).min(img.pix.len());
                        fb::blit_rgb(
                            xx as usize,
                            sy.max(byi) as usize,
                            img.w,
                            draw_h,
                            &img.pix[start..],
                            bx,
                            by,
                            bw,
                            bh,
                        );
                    }
                }
            }
        }
    }
}
