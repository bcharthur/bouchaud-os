//! Moteur de rendu web minimal : HTML -> DOM -> layout en flux de blocs ->
//! liste d'affichage peinte dans le framebuffer HD.
//!
//! Ce n'est pas un navigateur complet (pas de CSS arbitraire ni de JS), mais un
//! vrai moteur de rendu graphique : arbre DOM, mise en page en blocs/inline avec
//! retour a la ligne, tailles de police par balise (titres), liens colores et
//! cliquables, listes, regles horizontales, et champs de formulaire dessines.

use crate::gui::framebuffer as fb;
use crate::net::http::resolve_location;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ----------------------------------------------------------------------------
// DOM
// ----------------------------------------------------------------------------

pub struct Node {
    pub tag: Option<String>,      // None => noeud texte
    pub text: String,             // texte (noeud texte) ou valeur d'attribut utile
    pub attrs: Vec<(String, String)>,
    pub children: Vec<usize>,
}

pub struct Dom {
    pub nodes: Vec<Node>,         // 0 = racine
}

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

// Decodage d'entites HTML (replie les accents en ASCII pour la police bitmap).
fn decode_entities(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some(semi) = b[i + 1..].iter().take(12).position(|&c| c == b';') {
                let ent = &text[i + 1..i + 1 + semi];
                if let Some(c) = entity(ent) { out.push(c); i += 2 + semi; continue; }
            }
            out.push('&'); i += 1;
        } else {
            // garde l'ASCII imprimable ; replie le reste (UTF-8) sur '?'
            let c = b[i];
            if c == b'\t' { out.push(' '); } else { out.push(c as char); }
            i += 1;
        }
    }
    out
}

fn entity(ent: &str) -> Option<char> {
    if let Some(num) = ent.strip_prefix('#') {
        let code = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(h, 16).ok()?
        } else { num.parse::<u32>().ok()? };
        let c = char::from_u32(code)?;
        return Some(if c.is_ascii() { c } else { fold(c) });
    }
    Some(match ent {
        "amp" => '&', "lt" => '<', "gt" => '>', "quot" => '"', "apos" => '\'', "nbsp" => ' ',
        "copy" => 'c', "reg" => 'r', "hellip" => '.', "mdash" | "ndash" => '-',
        "rsquo" | "lsquo" => '\'', "rdquo" | "ldquo" | "laquo" | "raquo" => '"',
        "eacute" | "egrave" | "ecirc" | "euml" => 'e', "agrave" | "acirc" => 'a',
        "ccedil" => 'c', "ugrave" | "ucirc" => 'u', "icirc" | "iuml" => 'i', "ocirc" => 'o',
        "Eacute" | "Egrave" => 'E', "times" => 'x', "euro" => 'E', "trade" => 't',
        "deg" => 'o', "middot" => '.', "bull" => '*',
        _ => return None,
    })
}

fn fold(_c: char) -> char { '?' }

/// Parse un document HTML en arbre DOM (tolerant). Ignore script/style/commentaires.
pub fn parse(html: &[u8]) -> Dom {
    let mut dom = Dom::new();
    let mut stack: Vec<usize> = alloc::vec![0];
    let mut i = 0usize;
    while i < html.len() {
        if html[i] == b'<' {
            // Commentaire / doctype.
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
                if !closing {
                    let close: &[u8] = if name == "script" { b"</script" } else { b"</style" };
                    i = find_ci(html, close, i).map(|q| find_ci(html, b">", q).map(|r| r + 1).unwrap_or(html.len())).unwrap_or(html.len());
                }
                continue;
            }

            if closing {
                // Depile jusqu'a la balise correspondante (tolerant).
                if let Some(pos) = stack.iter().rposition(|&n| dom.nodes[n].tag.as_deref() == Some(name.as_str())) {
                    stack.truncate(pos.max(1));
                }
                continue;
            }

            // Balise ouvrante : parse les attributs.
            let attrs = parse_attrs(&raw[p..]);
            let self_closing = raw.last() == Some(&b'/');
            let parent = *stack.last().unwrap_or(&0);
            let id = dom.push(parent, Node { tag: Some(name.clone()), text: String::new(), attrs, children: Vec::new() });
            if !is_void(&name) && !self_closing {
                stack.push(id);
            }
        } else {
            let start = i;
            while i < html.len() && html[i] != b'<' { i += 1; }
            let frag = core::str::from_utf8(&html[start..i]).unwrap_or("");
            let decoded = decode_entities(frag);
            if decoded.trim().is_empty() {
                // espace inter-balises : garde un seul espace significatif
                if decoded.contains(|c: char| c == ' ' || c == '\n') {
                    let parent = *stack.last().unwrap_or(&0);
                    dom.push(parent, Node { tag: None, text: " ".to_string(), attrs: Vec::new(), children: Vec::new() });
                }
            } else {
                let parent = *stack.last().unwrap_or(&0);
                dom.push(parent, Node { tag: None, text: decoded, attrs: Vec::new(), children: Vec::new() });
            }
        }
        if dom.nodes.len() > 60_000 { break; } // garde-fou
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
// Layout -> liste d'affichage
// ----------------------------------------------------------------------------

pub enum Item {
    Rect { x: i32, y: i32, w: i32, h: i32, color: u8 },
    Text { x: i32, y: i32, s: String, color: u8, scale: usize },
}

pub struct Link { pub x: i32, pub y: i32, pub w: i32, pub h: i32, pub href: String }

pub struct Page {
    pub title: String,
    pub items: Vec<Item>,
    pub links: Vec<Link>,
    pub height: i32,
    pub bg: u8,
}

#[derive(Clone)]
struct Style { scale: usize, color: u8, href: Option<String>, indent: i32 }

struct Layout {
    items: Vec<Item>,
    links: Vec<Link>,
    width: i32,        // largeur de contenu
    x: i32,            // crayon inline
    y: i32,            // haut de la ligne courante
    line_h: i32,       // hauteur de la ligne courante
    margin: i32,       // marge gauche (indentation)
    title: String,
    scheme: String,
    host: String,
    link_n: usize,
}

const PAD: i32 = 6;

impl Layout {
    fn newline(&mut self) {
        self.y += self.line_h;
        self.x = PAD + self.margin;
        self.line_h = 8 * 2 + 4;
    }
    fn ensure_line_h(&mut self, h: i32) { if h > self.line_h { self.line_h = h; } }

    fn write_word(&mut self, word: &str, st: &Style) {
        let cw = 8 * st.scale as i32;
        let wpx = word.chars().count() as i32 * cw;
        let line_h = (8 * st.scale as i32) + 4;
        self.ensure_line_h(line_h);
        if self.x + wpx > PAD + self.width && self.x > PAD + self.margin {
            self.newline();
            self.ensure_line_h(line_h);
        }
        let color = st.color;
        if let Some(href) = &st.href {
            self.links.push(Link { x: self.x, y: self.y, w: wpx, h: line_h, href: href.clone() });
        }
        self.items.push(Item::Text { x: self.x, y: self.y, s: word.to_string(), color, scale: st.scale });
        self.x += wpx + cw; // + un espace
        if self.items.len() > 100_000 { /* garde-fou implicite via callers */ }
    }

    fn write_text(&mut self, text: &str, st: &Style) {
        for word in text.split(|c: char| c == ' ' || c == '\n' || c == '\t' || c == '\r') {
            if word.is_empty() { continue; }
            self.write_word(word, st);
        }
    }
}

fn block_tag(t: &str) -> bool {
    matches!(t, "p"|"div"|"h1"|"h2"|"h3"|"h4"|"h5"|"h6"|"ul"|"ol"|"li"|"section"|"article"|
        "header"|"footer"|"nav"|"main"|"aside"|"blockquote"|"pre"|"figure"|"table"|"tr"|
        "form"|"address"|"fieldset"|"dl"|"dt"|"dd"|"title"|"body"|"html"|"head")
}

fn heading_scale(t: &str) -> Option<usize> {
    match t { "h1" => Some(4), "h2" => Some(3), "h3" => Some(3), "h4" | "h5" | "h6" => Some(2), _ => None }
}

/// Construit la page (layout) a partir du DOM, pour une largeur de contenu donnee.
pub fn layout(dom: &Dom, base_url: &str, width: i32) -> Page {
    let (scheme, host) = scheme_host(base_url);
    let mut l = Layout {
        items: Vec::new(), links: Vec::new(), width: (width - 2 * PAD).max(40),
        x: PAD, y: PAD, line_h: 8 * 2 + 4, margin: 0, title: String::new(),
        scheme: scheme.to_string(), host: host.to_string(), link_n: 0,
    };
    let base = Style { scale: 2, color: fb::C_BLACK, href: None, indent: 0 };
    walk(dom, 0, &mut l, &base, 0);
    l.y += l.line_h;
    Page { title: l.title, items: l.items, links: l.links, height: l.y + PAD, bg: fb::C_WHITE }
}

fn walk(dom: &Dom, idx: usize, l: &mut Layout, st: &Style, depth: u32) {
    if l.items.len() > 80_000 || depth > 256 { return; }
    let node = &dom.nodes[idx];
    // Noeud texte.
    if node.tag.is_none() {
        if !node.text.is_empty() { l.write_text(&node.text, st); }
        return;
    }
    let tag = node.tag.as_deref().unwrap_or("");

    // <title> : capture pour la barre de titre, pas de rendu.
    if tag == "title" {
        let mut t = String::new();
        for &c in &node.children { if dom.nodes[c].tag.is_none() { t.push_str(&dom.nodes[c].text); } }
        l.title = t.trim().to_string();
        return;
    }
    if tag == "head" {
        for &c in &node.children { walk(dom, c, l, st, depth + 1); }
        return;
    }
    if matches!(tag, "br") { l.newline(); return; }
    if matches!(tag, "hr") {
        if l.x > PAD + l.margin { l.newline(); }
        l.items.push(Item::Rect { x: PAD, y: l.y + 4, w: l.width, h: 2, color: fb::C_GRAY });
        l.y += 12;
        return;
    }
    if tag == "img" {
        let alt = attr(node, "alt").unwrap_or("");
        let label = if alt.is_empty() { "[image]".to_string() } else { alloc::format!("[img: {}]", alt) };
        let st2 = Style { color: fb::C_GRAY, ..st.clone() };
        l.write_text(&label, &st2);
        return;
    }
    if matches!(tag, "input") {
        // Champ de formulaire : petite boite.
        let cw = 8 * st.scale as i32;
        if l.x + 24 * cw > PAD + l.width && l.x > PAD + l.margin { l.newline(); }
        let h = (8 * st.scale as i32) + 4;
        l.ensure_line_h(h);
        let w = (16 * cw).min(PAD + l.width - l.x);
        l.items.push(Item::Rect { x: l.x, y: l.y, w, h, color: fb::C_GRAY });
        l.items.push(Item::Rect { x: l.x + 1, y: l.y + 1, w: w - 2, h: h - 2, color: fb::C_WHITE });
        if let Some(v) = attr(node, "value") {
            l.items.push(Item::Text { x: l.x + 2, y: l.y + 2, s: v.to_string(), color: fb::C_BLACK, scale: st.scale });
        }
        l.x += w + cw;
        return;
    }

    // Style derive pour cet element.
    let mut child_st = st.clone();
    let is_block = block_tag(tag);
    if let Some(s) = heading_scale(tag) { child_st.scale = s; }
    if tag == "a" {
        if let Some(href) = attr(node, "href") {
            child_st.href = Some(resolve_location(&l.scheme, &l.host, href));
            child_st.color = fb::C_BLUE;
        }
    }
    if matches!(tag, "b" | "strong" | "em" | "i") { /* pas de gras : couleur inchangee */ }
    if matches!(tag, "ul" | "ol" | "blockquote" | "dl") { child_st.indent = st.indent + 16; }

    if is_block && l.x > PAD + l.margin { l.newline(); }
    if is_block {
        l.margin = child_st.indent;
        l.x = PAD + l.margin;
        if heading_scale(tag).is_some() { l.y += 6; } // espace avant titre
    }
    if tag == "li" {
        l.x = PAD + l.margin;
        let bst = Style { color: fb::C_DKGRAY, ..child_st.clone() };
        l.write_word("*", &bst);
    }

    for &c in &node.children { walk(dom, c, l, &child_st, depth + 1); }

    if is_block {
        if l.x > PAD + l.margin { l.newline(); }
        if matches!(tag, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "blockquote" | "table" | "form") {
            l.y += 6; // marge basse
        }
        l.margin = st.indent;
        l.x = PAD + l.margin;
    }
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

/// Peint la page dans la zone (bx,by,bw,bh), decalee verticalement de `scroll`.
pub fn paint(page: &Page, scroll: i32, bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect(bx, by, bw, bh, page.bg);
    let bx = bx as i32; let by = by as i32; let bw = bw as i32; let bh = bh as i32;
    for it in &page.items {
        match it {
            Item::Rect { x, y, w, h, color } => {
                let sy = by + y - scroll;
                if sy + h <= by || sy >= by + bh { continue; }
                let yy = sy.max(by);
                let hh = (sy + h).min(by + bh) - yy;
                let xx = bx + x;
                let ww = (*w).min(bw - x).max(0);
                if hh > 0 && ww > 0 && xx >= bx {
                    fb::fill_rect(xx as usize, yy as usize, ww as usize, hh as usize, *color);
                }
            }
            Item::Text { x, y, s, color, scale } => {
                let sy = by + y - scroll;
                let h = 8 * *scale as i32;
                if sy < by || sy + h > by + bh { continue; } // lignes entierement visibles
                let xx = bx + x;
                if xx >= bx && xx < bx + bw {
                    fb::draw_text_scaled(xx as usize, sy as usize, s, *color, *scale);
                }
            }
        }
    }
}
