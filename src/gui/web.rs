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
        if b[i] == b'&' {
            if let Some(semi) = b[i + 1..].iter().take(12).position(|&c| c == b';') {
                let ent = &text[i + 1..i + 1 + semi];
                if let Some(c) = entity(ent) { out.push(c); i += 2 + semi; continue; }
            }
            out.push('&'); i += 1;
        } else {
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
        return Some(if c.is_ascii() { c } else { '?' });
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
    let rest = &cleaned;
    let mut pos = 0usize;
    let bytes = rest.as_bytes();
    while pos < bytes.len() {
        // saute les @media/@font-face... en sautant jusqu'au { equilibre ou ;
        // (approche simple : on lit selecteurs jusqu'a '{').
        let open = match rest[pos..].find('{') { Some(o) => pos + o, None => break };
        let sel_part = rest[pos..open].trim();
        let close = match rest[open + 1..].find('}') { Some(c) => open + 1 + c, None => break };
        let body = &rest[open + 1..close];
        pos = close + 1;
        if sel_part.starts_with('@') { continue; } // @media etc. : on ignore le bloc
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

/// Pipeline complet : HTML -> (CSS extrait, DOM nettoye) -> page mise en page.
pub fn render(html: &[u8], base_url: &str, width: i32) -> Page {
    let scripted = crate::gui::js::execute_inline(html);
    let (clean, css) = extract_and_strip(&scripted, 1_500_000);
    let dom = parse(&clean);
    layout(&dom, base_url, width, &css)
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
        "gold" => 0xffd700, "transparent" => return None,
        _ => return None,
    })
}

fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
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
        let inside = rest.trim_start_matches('a').trim_start_matches('(').trim_end_matches(')');
        let mut it = inside.split(',').map(|x| x.trim().trim_end_matches('%'));
        let r: u32 = it.next()?.parse().ok()?;
        let g: u32 = it.next()?.parse().ok()?;
        let b: u32 = it.next()?.parse().ok()?;
        return Some(((r & 255) << 16) | ((g & 255) << 8) | (b & 255));
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

#[derive(Clone)]
struct Style {
    color: u32,
    scale: usize,
    bold: bool,
    align: u8,        // 0 gauche, 1 centre, 2 droite
    href: Option<String>,
    indent: i32,
}

fn default_style() -> Style { Style { color: 0x202124, scale: 2, bold: false, align: 0, href: None, indent: 0 } }

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
}

struct Layout {
    items: Vec<Item>,
    links: Vec<Link>,
    images: Vec<Image>,
    img_cache: Vec<(String, usize)>,
    img_budget: u32,
    width: i32,
    x: i32,            // position courante sur la ligne (PAD+margin = debut)
    y: i32,
    line: Vec<LineItem>,
    line_h: i32,
    margin: i32,
    align: u8,
    title: String,
    scheme: String,
    host: String,
}

impl Layout {
    fn avail(&self) -> i32 { self.width }

    fn flush_line(&mut self) {
        if self.line.is_empty() {
            self.y += self.line_h;
            self.line_h = 8 * 2 + 6;
            self.x = PAD + self.margin;
            return;
        }
        let used = self.line.iter().map(|it| match it {
            LineItem::Word { dx, w, .. } => dx + w,
            LineItem::Img { dx, w, .. } => dx + w,
            LineItem::Box { dx, w, .. } => dx + w,
        }).max().unwrap_or(0);
        let off = match self.align {
            1 => ((self.avail() - used) / 2).max(0),
            2 => (self.avail() - used).max(0),
            _ => 0,
        };
        let base_x = PAD + self.margin + off;
        let y = self.y;
        let lh = self.line_h;
        let line = core::mem::take(&mut self.line);
        for it in line {
            match it {
                LineItem::Word { dx, w, s, color, scale, bold, href } => {
                    let tx = base_x + dx;
                    let ty = y + (lh - 8 * scale as i32).max(0); // aligne en bas de ligne
                    if let Some(h) = href {
                        self.links.push(Link { x: tx, y, w, h: lh, href: h });
                    }
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
            }
        }
        self.y += lh;
        self.x = PAD + self.margin;
        self.line_h = 8 * 2 + 6;
    }

    fn line_cursor(&self) -> i32 {
        // position dx du prochain element = max(dx+w) + espace
        self.line.iter().map(|it| match it {
            LineItem::Word { dx, w, .. } => dx + w,
            LineItem::Img { dx, w, .. } => dx + w,
            LineItem::Box { dx, w, .. } => dx + w,
        }).max().unwrap_or(0)
    }

    fn push_word(&mut self, s: &str, st: &Style) {
        let cw = 8 * st.scale as i32;
        let wpx = s.chars().count() as i32 * cw;
        let lh = 8 * st.scale as i32 + 6;
        if lh > self.line_h { self.line_h = lh; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += cw; } // espace avant le mot
        if cur + wpx > self.avail() && cur > 0 {
            self.flush_line();
            cur = 0;
            if lh > self.line_h { self.line_h = lh; }
        }
        self.line.push(LineItem::Word { dx: cur, w: wpx, s: s.to_string(), color: st.color, scale: st.scale, bold: st.bold, href: st.href.clone() });
    }

    fn push_text(&mut self, text: &str, st: &Style) {
        for w in text.split(|c: char| c == ' ' || c == '\n' || c == '\t' || c == '\r') {
            if !w.is_empty() { self.push_word(w, st); }
            if self.items.len() + self.line.len() > 90_000 { return; }
        }
    }

    fn push_image(&mut self, idx: usize) {
        let (iw, ih) = (self.images[idx].w as i32, self.images[idx].h as i32);
        let lh = ih + 4;
        if lh > self.line_h { self.line_h = lh; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += 8; }
        if cur + iw > self.avail() && cur > 0 { self.flush_line(); cur = 0; if lh > self.line_h { self.line_h = lh; } }
        self.line.push(LineItem::Img { dx: cur, w: iw, h: ih, idx });
    }

    fn push_box(&mut self, w: i32, h: i32, fill: u32, value: String) {
        if h > self.line_h { self.line_h = h; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += 16; }
        if cur + w > self.avail() && cur > 0 { self.flush_line(); cur = 0; }
        self.line.push(LineItem::Box { dx: cur, w, h, fill, value });
    }

    // Charge une image (data:URI ou reseau), downscale, renvoie son index.
    fn load_image(&mut self, src: &str) -> Option<usize> {
        if let Some(&(_, idx)) = self.img_cache.iter().find(|(u, _)| u == src) { return Some(idx); }
        if self.img_budget == 0 { return None; }
        let max_w = self.avail().max(16) as usize;
        let raw: Vec<u8>;
        if let Some(rest) = src.strip_prefix("data:") {
            // data:[<mime>][;base64],<data>
            let comma = rest.find(',')?;
            let meta = &rest[..comma];
            let data = &rest[comma + 1..];
            raw = if meta.contains("base64") { base64_decode(data) } else { data.bytes().collect() };
        } else {
            let abs = resolve_location(&self.scheme, &self.host, src);
            self.img_budget -= 1;
            let doc = crate::net::fetch_document(&abs);
            if !doc.ok || doc.body.is_empty() { return None; }
            raw = doc.body;
        }
        let img = image::decode(&raw)?;
        let img = image::downscale(&img, max_w, 320);
        if img.w == 0 || img.h == 0 { return None; }
        let idx = self.images.len();
        self.images.push(img);
        self.img_cache.push((src.to_string(), idx));
        Some(idx)
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

/// Construit la page a partir du DOM (+ regles CSS pre-extraites).
fn layout(dom: &Dom, base_url: &str, width: i32, css: &[Rule]) -> Page {
    let (scheme, host) = scheme_host(base_url);
    let mut bg = 0xffffff_u32;
    // background du body/html depuis la feuille de style.
    for r in css {
        if matches!(&r.sel, Sel::Tag(t) if t == "body" || t == "html") {
            for (p, v) in &r.decls {
                if p == "background" || p == "background-color" { if let Some(c) = parse_color(v.split(' ').next().unwrap_or(v)) { bg = c; } }
            }
        }
    }
    let mut l = Layout {
        items: Vec::new(), links: Vec::new(), images: Vec::new(), img_cache: Vec::new(), img_budget: 6,
        width: (width - 2 * PAD).max(40), x: PAD, y: PAD, line: Vec::new(), line_h: 8 * 2 + 6,
        margin: 0, align: 0, title: String::new(),
        scheme: scheme.to_string(), host: host.to_string(),
    };
    walk(dom, 0, &mut l, &default_style(), css, 0);
    l.flush_line();
    l.y += PAD;
    Page { title: l.title, items: l.items, links: l.links, images: l.images, height: l.y, bg }
}

fn apply_decls(decls: &[(String, String)], st: &mut Style, hidden: &mut bool, bg: &mut Option<u32>) {
    for (p, v) in decls {
        match p.as_str() {
            "color" => { if let Some(c) = parse_color(v) { st.color = c; } }
            "background" | "background-color" => {
                if let Some(c) = parse_color(v.split(' ').next().unwrap_or(v)) { *bg = Some(c); }
            }
            "font-size" => { if let Some(px) = font_px(v) { st.scale = px_to_scale(px); } }
            "font-weight" => { let b = v.trim(); if b == "bold" || b == "bolder" || b == "700" || b == "800" || b == "900" { st.bold = true; } else if b == "normal" || b == "400" { st.bold = false; } }
            "text-align" => { st.align = match v.trim() { "center" => 1, "right" => 2, _ => 0 }; }
            "display" => { if v.trim() == "none" { *hidden = true; } }
            "visibility" => { if v.trim() == "hidden" { *hidden = true; } }
            _ => {}
        }
    }
}

fn walk(dom: &Dom, idx: usize, l: &mut Layout, st: &Style, css: &[Rule], depth: u32) {
    if l.items.len() > 80_000 || depth > 256 { return; }
    let node = &dom.nodes[idx];
    if node.tag.is_none() {
        if !node.text.is_empty() { l.push_text(&node.text, st); }
        return;
    }
    let tag = node.tag.as_deref().unwrap_or("");
    if tag == "style" || tag == "script" || tag == "head" {
        if tag == "head" { for &c in &node.children { walk(dom, c, l, st, css, depth + 1); } }
        return;
    }
    if tag == "title" {
        let mut t = String::new();
        for &c in &node.children { if dom.nodes[c].tag.is_none() { t.push_str(&dom.nodes[c].text); } }
        l.title = t.trim().to_string();
        return;
    }

    // --- cascade : style herite + regles + style inline ---
    let mut child_st = st.clone();
    let mut hidden = false;
    let mut block_bg: Option<u32> = None;
    if let Some(s) = heading_scale(tag) { child_st.scale = s; child_st.bold = true; }
    if matches!(tag, "b" | "strong") { child_st.bold = true; }
    if tag == "center" { child_st.align = 1; }
    if tag == "a" {
        if let Some(href) = attr(node, "href") {
            child_st.href = Some(resolve_location(&l.scheme, &l.host, href));
            child_st.color = 0x1a0dab; // bleu lien (Google)
        }
    }
    if matches!(tag, "ul" | "ol" | "blockquote" | "dl") { child_st.indent = st.indent + 18; }

    // regles CSS (par specificite)
    let classes = attr(node, "class").unwrap_or("").to_string();
    let id = attr(node, "id").unwrap_or("").to_ascii_lowercase();
    let mut matched: Vec<&Rule> = css.iter().filter(|r| sel_matches(&r.sel, tag, &classes, &id)).collect();
    matched.sort_by_key(|r| r.spec);
    for r in matched { apply_decls(&r.decls, &mut child_st, &mut hidden, &mut block_bg); }
    if let Some(style) = attr(node, "style") { apply_decls(&parse_decls(style), &mut child_st, &mut hidden, &mut block_bg); }

    if hidden { return; }

    // --- elements speciaux ---
    if tag == "br" { l.flush_line(); return; }
    if tag == "hr" {
        l.flush_line();
        l.items.push(Item::Rect { x: PAD, y: l.y + 4, w: l.width, h: 2, color: 0xcccccc });
        l.y += 12;
        return;
    }
    if tag == "img" {
        if let Some(src) = attr(node, "src") {
            if let Some(i) = l.load_image(src) { l.push_image(i); return; }
        }
        let alt = attr(node, "alt").unwrap_or("");
        let label = if alt.is_empty() { "[image]".to_string() } else { alloc::format!("[img: {}]", alt) };
        let g = Style { color: 0x9aa0a6, ..child_st.clone() };
        l.push_text(&label, &g);
        return;
    }
    if tag == "input" {
        let cw = 8 * 2;
        let h = 8 * 2 + 8;
        let w = (16 * cw).min(l.width / 2);
        let val = attr(node, "value").unwrap_or("").to_string();
        l.push_box(w, h, 0xffffff, val);
        return;
    }

    let is_block = block_tag(tag);
    if is_block { l.flush_line(); l.margin = child_st.indent; l.x = PAD + l.margin; if heading_scale(tag).is_some() { l.y += 6; } }

    let saved_align = l.align;
    l.align = child_st.align;

    // fond de bloc : on retient l'index + y de depart pour inserer un Rect dessous.
    let bg_start_y = l.y;
    let bg_insert = l.items.len();

    if tag == "li" { l.x = PAD + l.margin; let b = Style { color: 0x5f6368, ..child_st.clone() }; l.push_word("*", &b); }

    for &c in &node.children { walk(dom, c, l, &child_st, css, depth + 1); }

    if is_block { l.flush_line(); }

    if let Some(bgc) = block_bg {
        let h = (l.y - bg_start_y).max(0);
        if h > 0 {
            l.items.insert(bg_insert, Item::Rect { x: PAD + child_st.indent, y: bg_start_y, w: (l.width - child_st.indent).max(0), h, color: bgc });
        }
    }

    l.align = saved_align;
    if is_block {
        if matches!(tag, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "blockquote" | "table" | "form" | "div") { l.y += 6; }
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
                    fb::draw_text_rgb(xx as usize, sy as usize, s, *color, *scale);
                    if *bold { fb::draw_text_rgb((xx + 1) as usize, sy as usize, s, *color, *scale); }
                }
            }
            Item::Image { x, y, w: _w, h, idx } => {
                let sy = byi + y - scroll;
                if sy + h <= byi || sy >= byi + bhi { continue; }
                if let Some(img) = page.images.get(*idx) {
                    let xx = bxi + x;
                    if xx >= bxi && xx < bxi + bwi {
                        let top = sy.max(byi);
                        let bottom = (sy + h).min(byi + bhi);
                        let src_y = (top - sy).max(0) as usize;
                        let draw_h = (bottom - top).max(0) as usize;
                        if draw_h == 0 || src_y >= img.h { continue; }
                        let draw_h = draw_h.min(img.h - src_y);
                        let start = src_y.saturating_mul(img.w).min(img.pix.len());
                        fb::blit_rgb(
                            xx as usize,
                            top as usize,
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
