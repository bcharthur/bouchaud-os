//! Moteur de rendu web : HTML -> DOM -> CSS (subset) -> layout flux blocs/inline
//! -> liste d'affichage truecolor peinte dans le framebuffer HD.
//!
//! Pas un navigateur complet (JS volontairement minimal, CSS partiel), mais un vrai moteur :
//! arbre DOM, feuilles de style (`<style>` + `style=""`), cascade avec
//! selecteurs simples (balise/.classe/#id), couleurs reelles, tailles de
//! police, gras, alignement, fonds de blocs, masquage (`display:none`), liens
//! cliquables, **mini-JS inline** (`document.write`, `innerHTML`) et images
//! (PNG / data:URI / fetch reseau) downscalees.

use crate::gui::image::{self, Image};
use super::display_list::{apply_box_transform, translate_item};
pub use super::display_list::{Item, Layer, Link, Page};
pub use super::paint::paint;
use super::style::{CssIndex, Rule, Sel, Comb, AttrOp, AttrSel, Pseudo};
use super::css_values::parse_transform;
use super::css_parser::{parse_decls, parse_stylesheet};
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

#[inline]
fn is_void(tag: &str) -> bool { super::html::tags::is_void(tag) }

// Liste minimale des balises de bloc (pour la fermeture implicite des <p>).
fn is_block_name(t: &str) -> bool {
    matches!(t, "address"|"article"|"aside"|"blockquote"|"details"|"div"|"dl"|"fieldset"|
        "figcaption"|"figure"|"footer"|"form"|"h1"|"h2"|"h3"|"h4"|"h5"|"h6"|"header"|"hr"|
        "main"|"nav"|"ol"|"p"|"pre"|"section"|"table"|"ul")
}

/// Fermetures implicites de l'algorithme de construction d'arbre HTML5 : à
/// l'ouverture d'un élément, dépile les éléments ouverts qu'il referme
/// automatiquement (`<li>`, `<p>`, `<td>`, `<tr>`, `<option>`, `<dt>/<dd>`…),
/// ce qui corrige l'imbrication des pages réelles au balisage relâché.
fn auto_close(stack: &mut Vec<usize>, dom: &Dom, opening: &str) {
    loop {
        let top = match stack.last() {
            Some(&n) if n != 0 => dom.nodes[n].tag.as_deref().unwrap_or(""),
            _ => break,
        };
        let should_close = match opening {
            "li" => top == "li",
            "dt" | "dd" => top == "dt" || top == "dd",
            "option" => top == "option" || top == "optgroup",
            "td" | "th" => top == "td" || top == "th",
            "tr" => top == "td" || top == "th" || top == "tr",
            "thead" | "tbody" | "tfoot" => top == "td" || top == "th" || top == "tr",
            "p" => top == "p",
            // L'ouverture de tout bloc ferme un <p> resté ouvert.
            _ if is_block_name(opening) => top == "p",
            _ => false,
        };
        if should_close { stack.pop(); } else { break; }
    }
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
            // Fermetures implicites HTML5 : un <li>/<p>/<td>/<tr>/<option>...
            // non ferme est clos par l'ouverture d'un element incompatible.
            auto_close(&mut stack, &dom, &name);
            let parent = *stack.last().unwrap_or(&0);
            let id = dom.push(parent, Node { tag: Some(name.clone()), text: String::new(), attrs, children: Vec::new() });
            if !is_void(&name) && !self_closing { stack.push(id); }
        } else {
            let start = i;
            while i < html.len() && html[i] != b'<' { i += 1; }
            // Decodage UTF-8 tolerant : on ne jette plus tout le run si un octet
            // est invalide (pages en encodage mixte) — remplacement byte a byte.
            let frag = String::from_utf8_lossy(&html[start..i]);
            let decoded = decode_entities(&frag);
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

// Premiere URL d'un attribut `srcset` (`url 1x, url2 2x` ou `url 480w, ...`).
fn srcset_first(v: &str) -> Option<&str> {
    v.split(',').next()
        .and_then(|first| first.trim().split_whitespace().next())
        .filter(|u| !u.is_empty())
}

/// Resout la source reelle d'une `<img>` du web moderne : gere le lazy-loading
/// (`data-src`, `data-original`, `data-lazy-src`) ou `src` n'est qu'un
/// placeholder transparent, et `srcset`/`data-srcset` (premiere URL). Renvoie
/// l'URL a charger, ou None.
fn img_src(node: &Node) -> Option<&str> {
    let src = attr(node, "src").map(str::trim).filter(|s| !s.is_empty());
    let src_is_placeholder = src.map_or(true, |s| {
        s.starts_with("data:image/svg") || s.starts_with("data:image/gif")
            || s.contains("blank.") || s.contains("spacer") || s.contains("placeholder")
    });
    // Sources lazy explicites : prioritaires si `src` est absent/placeholder.
    if src_is_placeholder {
        for k in ["data-src", "data-original", "data-lazy-src", "data-lazy"] {
            if let Some(v) = attr(node, k) {
                let v = v.trim();
                if !v.is_empty() { return Some(v); }
            }
        }
        for k in ["srcset", "data-srcset"] {
            if let Some(v) = attr(node, k) {
                if let Some(u) = srcset_first(v) { return Some(u); }
            }
        }
    }
    if let Some(s) = src { return Some(s); }
    // Dernier recours : srcset meme si src etait un vrai (mais vide) chemin.
    attr(node, "srcset").and_then(srcset_first)
}

// ----------------------------------------------------------------------------
// CSS (subset)
// ----------------------------------------------------------------------------

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
            // <noscript> : on execute le JS, donc son contenu (souvent un
            // `<style>table,div,span,p{display:none}</style>` anti-no-JS) ne doit
            // JAMAIS s'appliquer, sinon il masque toute la page.
            if starts_ci(&html[i..], b"<noscript") {
                i = find_ci(html, b"</noscript", i + 1)
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
    let scripted = crate::gui::js::execute_inline(html, base_url);
    render_scripted(&scripted, base_url, width)
}

// Met en page un HTML deja enrichi par le JS (DOM applique).
fn render_scripted(scripted: &[u8], base_url: &str, width: i32) -> Page {
    use crate::diag::Cat;
    let mc = |a: u64, b: u64| -> u64 { b.wrapping_sub(a) / 1_000_000 };

    // ── Phase 1 : tokenizer/tree builder (HTML -> DOM) ──
    let t0 = crate::kernel::timer::cycles_since_boot();
    let (clean, inline_css) = extract_and_strip(scripted, 1_500_000);
    let dom = parse(&clean);
    let t1 = crate::kernel::timer::cycles_since_boot();
    crate::dlog!(Cat::Dom, "parse HTML: {} noeuds, {}o -> {}Mc", dom.nodes.len(), clean.len(), mc(t0, t1));

    // ── Phase 2 : style (CSS externe + inline -> regles + index) ──
    // Feuilles externes (<link rel=stylesheet>) d'abord (priorite plus faible sur
    // egalite), puis le CSS inline (<style>/style="") par-dessus.
    let mut css: Vec<Rule> = Vec::new();
    load_external_css(&dom, base_url, &mut css);
    let ext_rules = css.len();
    css.extend(inline_css);
    let t2 = crate::kernel::timer::cycles_since_boot();
    crate::dlog!(Cat::Css, "cascade: {} regles ({} externes) -> {}Mc", css.len(), ext_rules, mc(t1, t2));

    // ── Phase 3 : layout (cascade + box model + flex/grid/position -> items) ──
    let mut page = layout(&dom, base_url, width, &css, false);
    let t3 = crate::kernel::timer::cycles_since_boot();
    // Fallback anti-FOUC : si aucun item n'est rendu (JS n'a pas pu retirer
    // les classes display:none), relancer le layout en ignorant display:none.
    if page.items.is_empty() {
        crate::dlog!(Cat::Layout, "layout: 0 items -> fallback sans display:none");
        page = layout(&dom, base_url, width, &css, true);
    }
    let capped = if page.layers.len() >= MAX_LAYERS { " [PLAFOND couches atteint]" } else { "" };
    crate::dlog!(Cat::Layout, "layout: {} items, {} couches{}, h={}px -> {}Mc",
        page.items.len(), page.layers.len(), capped, page.height, mc(t2, t3));
    page
}

/// Telecharge et applique les feuilles de style externes referencees par
/// `<link rel="stylesheet" href="...">` (la cause n°1 des pages "sans style" :
/// les sites modernes externalisent leur CSS). Resout les URL relatives, passe
/// par le cache reseau, et gere `@import` de premier niveau.
fn load_external_css(dom: &Dom, base_url: &str, css: &mut Vec<Rule>) {
    let (scheme, host) = scheme_host(base_url);
    let mut count = 0u32;
    for node in &dom.nodes {
        if count >= 12 { break; }
        if node.tag.as_deref() != Some("link") { continue; }
        let rel = attr(node, "rel").unwrap_or("");
        if !rel.split_whitespace().any(|r| r.eq_ignore_ascii_case("stylesheet")) {
            // <link rel="preload" as="style"> est aussi une feuille de style.
            let as_attr = attr(node, "as").unwrap_or("");
            if !as_attr.eq_ignore_ascii_case("style") { continue; }
        }
        let href = match attr(node, "href") { Some(h) if !h.trim().is_empty() => h.trim(), _ => continue };
        let abs = resolve_location(&scheme, &host, href);
        let before = css.len();
        if let Some(bytes) = crate::net::fetch_cached(&abs) {
            let text = String::from_utf8_lossy(&bytes);
            parse_stylesheet_imports(&text, &scheme, &host, css, 0);
            crate::dlog!(crate::diag::Cat::Css, "feuille {} -> +{} regles", abs, css.len() - before);
            count += 1;
        } else {
            crate::dlog!(crate::diag::Cat::Warn, "feuille CSS injoignable: {}", abs);
        }
    }
}

// Parse une feuille externe en suivant ses `@import url(...)` (1 niveau de
// recursion) avant ses propres regles.
fn parse_stylesheet_imports(text: &str, scheme: &str, host: &str, css: &mut Vec<Rule>, depth: u32) {
    if depth < 2 {
        let mut search = 0usize;
        let mut imports = 0u32;
        while let Some(pos) = text[search..].find("@import") {
            let abs_pos = search + pos;
            let rest = &text[abs_pos..];
            let end = rest.find(';').map(|e| abs_pos + e).unwrap_or(text.len());
            let decl = &text[abs_pos..end];
            // @import url("x") | @import "x"
            if let Some(u) = extract_import_url(decl) {
                let abs = resolve_location(scheme, host, &u);
                if let Some(bytes) = crate::net::fetch_cached(&abs) {
                    let sub = String::from_utf8_lossy(&bytes);
                    parse_stylesheet_imports(&sub, scheme, host, css, depth + 1);
                }
            }
            search = end + 1;
            imports += 1;
            if imports >= 8 || search >= text.len() { break; }
        }
    }
    parse_stylesheet(text, css);
}

// Extrait l'URL d'une declaration `@import` (`url("x")`, `url(x)`, ou `"x"`).
fn extract_import_url(decl: &str) -> Option<String> {
    let d = decl.trim_start_matches("@import").trim();
    let d = if let Some(rest) = d.strip_prefix("url(") {
        rest.split(')').next().unwrap_or("")
    } else { d };
    let d = d.trim().trim_matches('"').trim_matches('\'').trim();
    // Ignore les requetes media (`@import "x" screen`).
    let url = d.split_whitespace().next().unwrap_or("").trim_matches('"').trim_matches('\'');
    if url.is_empty() { None } else { Some(url.to_string()) }
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
        let (ctx, scripted) = crate::gui::js::open_page(html, base_url);
        let page = render_scripted(&scripted, base_url, width);
        (Session { ctx, base: base_url.to_string(), width }, page)
    }
    /// Ouvre une page reseau en mode souverain : DOM + CSS + images, SANS executer
    /// le JS de la page (les SPA modernes ne peuvent pas tourner ici et ne
    /// produisent que du texte parasite). Rendu propre et lisible.
    pub fn open_static(html: &[u8], base_url: &str, width: i32) -> (Session, Page) {
        let (ctx, scripted) = crate::gui::js::open_page_static(html, base_url);
        let page = render_scripted(&scripted, base_url, width);
        (Session { ctx, base: base_url.to_string(), width }, page)
    }
    /// Rejoue un gestionnaire (code d'un lien `javascript:`) et re-rend la page.
    pub fn dispatch(&mut self, code: &str) -> Page {
        let scripted = self.ctx.dispatch(code);
        render_scripted(&scripted, &self.base, self.width)
    }
    /// Largeur de mise en page courante (pour detecter un redimensionnement).
    pub fn width(&self) -> i32 { self.width }
    /// Re-rend a une nouvelle largeur sans rejouer de code.
    pub fn relayout(&mut self, width: i32) -> Page {
        self.width = width;
        let html = self.ctx.html();
        render_scripted(&html, &self.base, self.width)
    }
}

// --- Matching selecteur ↔ element, avec acces DOM complet -------------------
// `path` = chemin d'index DOM racine→element courant (le dernier est l'element
// teste). Donne acces aux attributs, aux ancetres (path[..len-1]) et a la
// fratrie (enfants du parent path[len-2]), comme un vrai moteur.

fn attr_sel_matches(node: &Node, a: &AttrSel) -> bool {
    let v = match attr(node, &a.name) { Some(v) => v, None => return false };
    match a.op {
        AttrOp::Exists => true,
        AttrOp::Eq => v == a.val,
        AttrOp::Prefix => !a.val.is_empty() && v.starts_with(&a.val),
        AttrOp::Suffix => !a.val.is_empty() && v.ends_with(&a.val),
        AttrOp::Substr => !a.val.is_empty() && v.contains(&a.val),
        AttrOp::Word => v.split(char::is_whitespace).any(|w| w == a.val),
        AttrOp::Dash => v == a.val || (v.len() > a.val.len() && v.starts_with(&a.val) && v.as_bytes()[a.val.len()] == b'-'),
    }
}

// Position (1-based) et total de l'element parmi ses FRERES elements.
fn elem_pos(dom: &Dom, path: &[usize]) -> (usize, usize) {
    if path.len() < 2 { return (1, 1); }
    let parent = &dom.nodes[path[path.len() - 2]];
    let me = path[path.len() - 1];
    let mut count = 0usize; let mut pos = 0usize;
    for &c in &parent.children {
        if dom.nodes[c].tag.is_some() { count += 1; if c == me { pos = count; } }
    }
    if pos == 0 { (1, count.max(1)) } else { (pos, count) }
}

// Verifie e == a*k + b pour un entier k >= 0 (semantique :nth-child).
fn anb_match(a: i32, b: i32, e: i32) -> bool {
    if a == 0 { return e == b; }
    let d = e - b;
    d % a == 0 && d / a >= 0
}

fn pseudo_matches(p: &Pseudo, dom: &Dom, path: &[usize]) -> bool {
    match p {
        Pseudo::Root => path.len() <= 1 || matches!(dom.nodes[path[path.len() - 1]].tag.as_deref(), Some("html")),
        Pseudo::FirstChild => { let (pos, _) = elem_pos(dom, path); pos == 1 }
        Pseudo::LastChild => { let (pos, cnt) = elem_pos(dom, path); pos == cnt }
        Pseudo::OnlyChild => { let (_, cnt) = elem_pos(dom, path); cnt == 1 }
        Pseudo::NthChild(a, b) => { let (pos, _) = elem_pos(dom, path); anb_match(*a, *b, pos as i32) }
        Pseudo::NthLastChild(a, b) => { let (pos, cnt) = elem_pos(dom, path); anb_match(*a, *b, (cnt - pos + 1) as i32) }
        Pseudo::Empty => {
            let node = &dom.nodes[path[path.len() - 1]];
            node.children.iter().all(|&c| { let n = &dom.nodes[c]; n.tag.is_none() && n.text.trim().is_empty() })
        }
        Pseudo::Not(inner) => !inner.iter().any(|s| compound_matches(s, dom, path)),
    }
}

// Un compound (tag+id+classes+attrs+pseudos) matche-t-il l'element en fin de path.
fn compound_matches(sel: &Sel, dom: &Dom, path: &[usize]) -> bool {
    let node = &dom.nodes[path[path.len() - 1]];
    let tag = node.tag.as_deref().unwrap_or("");
    if let Some(t) = &sel.tag { if !t.eq_ignore_ascii_case(tag) { return false; } }
    if let Some(x) = &sel.id {
        match attr(node, "id") { Some(v) if v.eq_ignore_ascii_case(x) => {}, _ => return false }
    }
    if !sel.classes.is_empty() {
        let classes = attr(node, "class").unwrap_or("");
        for c in &sel.classes { if !classes.split(char::is_whitespace).any(|cl| cl == c) { return false; } }
    }
    for a in &sel.attrs { if !attr_sel_matches(node, a) { return false; } }
    for p in &sel.pseudos { if !pseudo_matches(p, dom, path) { return false; } }
    true
}

// Frere element immediatement precedent (pour `+`).
fn prev_elem_sibling(dom: &Dom, path: &[usize]) -> Option<usize> {
    if path.len() < 2 { return None; }
    let parent = &dom.nodes[path[path.len() - 2]];
    let me = path[path.len() - 1];
    let mut prev = None;
    for &c in &parent.children {
        if c == me { break; }
        if dom.nodes[c].tag.is_some() { prev = Some(c); }
    }
    prev
}

// Matche chain[0..=ci] en terminant sur l'element decrit par `path`.
fn match_chain(chain: &[Sel], ci: usize, dom: &Dom, path: &[usize]) -> bool {
    if !compound_matches(&chain[ci], dom, path) { return false; }
    if ci == 0 { return true; }
    match chain[ci].comb {
        Comb::Descendant => {
            let mut d = path.len();
            while d > 1 { d -= 1; if match_chain(chain, ci - 1, dom, &path[..d]) { return true; } }
            false
        }
        Comb::Child => path.len() >= 2 && match_chain(chain, ci - 1, dom, &path[..path.len() - 1]),
        Comb::Adjacent => {
            match prev_elem_sibling(dom, path) {
                Some(prev) => { let mut np = path[..path.len() - 1].to_vec(); np.push(prev); match_chain(chain, ci - 1, dom, &np) }
                None => false,
            }
        }
        Comb::General => {
            if path.len() < 2 { return false; }
            let parent = &dom.nodes[path[path.len() - 2]];
            let me = path[path.len() - 1];
            for &c in &parent.children {
                if c == me { break; }
                if dom.nodes[c].tag.is_some() {
                    let mut np = path[..path.len() - 1].to_vec(); np.push(c);
                    if match_chain(chain, ci - 1, dom, &np) { return true; }
                }
            }
            false
        }
    }
}

fn rule_matches(chain: &[Sel], dom: &Dom, path: &[usize]) -> bool {
    !chain.is_empty() && match_chain(chain, chain.len() - 1, dom, path)
}

// Couleurs --------------------------------------------------------------------

// Interpolation lineaire entre deux couleurs RGB (t/max dans [0,1]).
fn lerp_rgb(c1: u32, c2: u32, t: i32, max: i32) -> u32 {
    let m = max.max(1);
    let ch = |sh: u32| -> u32 {
        let a = ((c1 >> sh) & 0xff) as i32;
        let b = ((c2 >> sh) & 0xff) as i32;
        (a + (b - a) * t / m).clamp(0, 255) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

// Decoupe une liste CSS par virgules de premier niveau (respecte les parentheses
// de rgb()/rgba()/hsl()).
fn split_top_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' => { if depth > 0 { depth -= 1; } }
            b',' if depth == 0 => { out.push(s[start..i].trim()); start = i + 1; }
            _ => {}
        }
    }
    if start < s.len() { out.push(s[start..].trim()); }
    out
}

// Couleur d'un stop de gradient ("#163870 60%" / "rgba(0,0,0,.4) 50%" / "red").
fn color_of_stop(part: &str) -> Option<u32> {
    let p = part.trim();
    if let Some(c) = parse_color(p) { return Some(c); }
    if let Some(idx) = p.rfind(char::is_whitespace) { return parse_color(p[..idx].trim()); }
    None
}

/// Parse `linear-gradient(...)` en (couleur_debut, couleur_fin, vertical).
/// La direction (`to bottom`, `160deg`...) determine l'axe ; les stops
/// intermediaires sont reduits a leurs extremites (gradient 2 teintes).
fn parse_gradient(s: &str) -> Option<(u32, u32, bool)> {
    let key = "linear-gradient(";
    let start = s.find(key)? + key.len();
    // Trouve la parenthese fermante correspondante.
    let bytes = s.as_bytes();
    let mut depth = 1i32;
    let mut end = start;
    while end < bytes.len() && depth > 0 {
        match bytes[end] { b'(' => depth += 1, b')' => depth -= 1, _ => {} }
        if depth == 0 { break; }
        end += 1;
    }
    let inner = &s[start..end.min(s.len())];
    let parts = split_top_commas(inner);
    if parts.is_empty() { return None; }

    let mut vertical = true;
    let mut colors: Vec<u32> = Vec::new();
    for (idx, p) in parts.iter().enumerate() {
        let pt = p.trim();
        // Premier element : direction eventuelle.
        if idx == 0 && (pt.ends_with("deg") || pt.starts_with("to ") || pt.ends_with("turn")) {
            vertical = if let Some(a) = pt.strip_suffix("deg").and_then(|x| x.trim().parse::<f32>().ok()) {
                let m = ((a % 180.0) + 180.0) % 180.0;
                m < 45.0 || m > 135.0
            } else if pt.contains("left") || pt.contains("right") {
                false
            } else {
                true // to bottom / to top / défaut
            };
            continue;
        }
        if let Some(c) = color_of_stop(pt) { colors.push(c); }
    }
    if colors.is_empty() { return None; }
    Some((colors[0], *colors.last().unwrap(), vertical))
}

// Couleurs CSS : déléguées au module fragmenté `engine::css::color`.
#[inline]
fn parse_color(s: &str) -> Option<u32> { super::css::color::parse(s) }

fn font_px(s: &str) -> Option<i32> {
    let s = s.trim();
    match s {
        "xx-small" => return Some(10), "x-small" => return Some(12), "small" => return Some(13),
        "medium" => return Some(16), "large" => return Some(20), "x-large" => return Some(26),
        "xx-large" => return Some(33), _ => {}
    }
    if let Some(px) = s.strip_suffix("px") { return px.trim().parse::<f32>().ok().map(|v| v as i32); }
    // `rem` (root em) AVANT `em` car "1rem" se termine aussi par "em".
    // Taille de police racine = 16px (defaut navigateur).
    if let Some(rem) = s.strip_suffix("rem") { return rem.trim().parse::<f32>().ok().map(|v| (v * 16.0) as i32); }
    if let Some(em) = s.strip_suffix("em") { return em.trim().parse::<f32>().ok().map(|v| (v * 16.0) as i32); }
    if let Some(pt) = s.strip_suffix("pt") { return pt.trim().parse::<f32>().ok().map(|v| (v * 4.0 / 3.0) as i32); }
    // `vw` / `vh` : approximation viewport (fenetre Nautile ~ 600x420) pour les
    // tailles relatives au viewport, courantes sur le web moderne.
    if let Some(vw) = s.strip_suffix("vw") { return vw.trim().parse::<f32>().ok().map(|v| (v * 6.0) as i32); }
    if let Some(vh) = s.strip_suffix("vh") { return vh.trim().parse::<f32>().ok().map(|v| (v * 4.2) as i32); }
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
    transform: u8,    // text-transform: 0 none, 1 uppercase, 2 lowercase, 3 capitalize
    nowrap: bool,     // white-space: nowrap (pas de retour a la ligne automatique)
    line_h: Option<i32>, // line-height explicite (px) ; None = defaut
    va: u8,           // vertical-align: 0 baseline, 1 middle, 2 top, 3 bottom
}

fn default_style() -> Style { Style { color: 0x202124, scale: 2, bold: false, align: 0, href: None, pre: false, transform: 0, nowrap: false, line_h: None, va: 0 } }

// Marqueur de l'element de liste courant + avance le compteur. `<ol>` -> numero
// (1. 2. 3.), `<ul>` -> puce (• / rien si list-style:none). None hors liste connue.
fn li_marker(ctx: &mut Ctx) -> Option<String> {
    let top = ctx.list_stack.last_mut()?;
    if top.0 {
        let i = top.1;
        top.1 += 1;
        Some(alloc::format!("{}.", i))
    } else if top.2 == 1 {
        None // list-style: none
    } else {
        Some("\u{2022}".to_string()) // puce •
    }
}

// Applique text-transform a un mot.
fn apply_transform(s: &str, t: u8) -> String {
    match t {
        1 => s.chars().flat_map(|c| c.to_uppercase()).collect(),
        2 => s.chars().flat_map(|c| c.to_lowercase()).collect(),
        3 => {
            let mut out = String::new();
            let mut first = true;
            for c in s.chars() {
                if first { for u in c.to_uppercase() { out.push(u); } first = false; }
                else { out.push(c); }
            }
            out
        }
        _ => s.to_string(),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Disp { Block, Inline, InlineBlock, Flex, Grid, None }
#[derive(Clone, Copy, PartialEq)]
enum FloatK { None, Left, Right }

// Direction d'un conteneur flex (row par defaut, column si empile verticalement).
#[derive(Clone, Copy, PartialEq)]
enum FlexDir { Row, Column }
// Alignement principal (justify-content) : repartition de l'espace libre.
#[derive(Clone, Copy, PartialEq)]
enum Justify { Start, Center, End, Between, Around }
// Alignement transversal (align-items) : position dans la hauteur de ligne.
#[derive(Clone, Copy, PartialEq)]
enum AlignI { Start, Center, End, Stretch }
// Schema de positionnement CSS (position:).
#[derive(Clone, Copy, PartialEq)]
enum Pos { Static, Relative, Absolute, Fixed, Sticky }
#[derive(Clone, Copy)]
enum Len { Px(i32), Pct(i32), Calc { pct: i32, px: i32 } }
impl Len {
    fn resolve(self, avail: i32) -> i32 {
        match self {
            Len::Px(p) => p,
            Len::Pct(p) => (avail * p / 100).max(0),
            // calc(pct% +/- px) — composante relative + absolue, comme un vrai moteur.
            Len::Calc { pct, px } => (avail * pct / 100 + px).max(0),
        }
    }
}

// Evalue un `calc(...)` de longueurs : termes en % et px combines par + et -
// (les operateurs * et / ne sont pas geres -> None, repli sur auto). Tolere
// l'absence d'espaces autour des operateurs (CSS minifie).
fn parse_calc(expr: &str) -> Option<Len> {
    if expr.contains('*') || expr.contains('/') { return None; }
    // Normalise : insere des espaces autour de + et - pour tokeniser simplement.
    let norm = expr.replace('+', " + ").replace('-', " - ");
    let mut pct = 0i32;
    let mut px = 0i32;
    let mut sign = 1i32;
    let mut seen = false;
    for tok in norm.split_whitespace() {
        match tok {
            "+" => sign = 1,
            "-" => sign = -1,
            _ => {
                if let Some(p) = tok.strip_suffix('%') {
                    pct += sign * p.trim().parse::<f32>().ok()? as i32;
                } else if let Some(v) = font_px(tok) {
                    px += sign * v;
                } else {
                    return None;
                }
                seen = true;
                sign = 1;
            }
        }
    }
    if !seen { return None; }
    if pct == 0 { Some(Len::Px(px)) } else { Some(Len::Calc { pct, px }) }
}

fn parse_len(s: &str) -> Option<Len> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) {
        return parse_calc(inner);
    }
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
    // Box model par cote (px) — espace interieur (padding) et exterieur (margin).
    pad_t: i32, pad_r: i32, pad_b: i32, pad_l: i32,
    mar_t: i32, mar_b: i32,
    border_w: i32,         // epaisseur de bordure (px)
    border_color: u32,     // couleur de bordure
    radius: i32,           // border-radius (px) — purement indicatif au rendu
    // Conteneur flex / grid.
    flex_dir: FlexDir,
    justify: Justify,
    align_items: AlignI,
    gap: i32,              // espacement entre items flex/grid (px)
    grid_cols: u8,         // nombre de colonnes (grid-template-columns) ; 0 = auto
    // Enfant flex : facteur de croissance (flex-grow / flex:N).
    flex_grow: i32,
    // Enfant grid : nombre de colonnes occupees (grid-column span) ; 0 = 1, 255 = pleine rangee.
    grid_span: u8,
    // list-style-type : 0 auto (disc/decimal selon ul/ol), 1 none, 2 disc,
    // 3 circle, 4 square, 5 decimal.
    list_style: u8,
    // Positionnement (P1) : schema + decalages + ordre d'empilement + clipping.
    position: Pos,
    top: Option<Len>, right: Option<Len>, bottom: Option<Len>, left: Option<Len>,
    z_index: Option<i32>,
    overflow_clip: bool,   // overflow: hidden/clip/auto/scroll -> clippe le contenu
    shadow: Option<u32>,   // box-shadow : couleur de l'ombre portee (offset fixe)
    grad: Option<(u32, u32, bool)>, // linear-gradient : (debut, fin, vertical)
    tx: i32, ty: i32,       // transform: translate(...) simplifie
    scale_pct: i32,         // transform: scale(...) en pourcentage (100 = identite)
}
fn default_box() -> BoxProps {
    BoxProps { hidden: false, bg: None, width: None, height: None, max_width: None, min_width: None, min_height: None,
        center: false, disp: None, float: FloatK::None,
        pad_t: 0, pad_r: 0, pad_b: 0, pad_l: 0, mar_t: 0, mar_b: 0,
        border_w: 0, border_color: 0x000000, radius: 0,
        flex_dir: FlexDir::Row, justify: Justify::Start, align_items: AlignI::Stretch,
        gap: 0, grid_cols: 0, flex_grow: 0, grid_span: 0, list_style: 0,
        position: Pos::Static, top: None, right: None, bottom: None, left: None,
        z_index: None, overflow_clip: false, shadow: None, grad: None, tx: 0, ty: 0, scale_pct: 100 }
}

// Premiere longueur d'une valeur raccourcie (`10px 20px` -> 10).
fn first_len(val: &str) -> Option<i32> {
    val.split_whitespace().find_map(|t| parse_len(t).map(|l| l.resolve(0)))
}

// Decompose un raccourci CSS 1-4 valeurs en (top, right, bottom, left).
// `10px` -> tous ; `10px 20px` -> v/h ; `1 2 3` -> t/h/b ; `1 2 3 4` -> t/r/b/l.
fn parse_sides(val: &str) -> (i32, i32, i32, i32) {
    let parts: Vec<i32> = val.split_whitespace()
        .map(|t| parse_len(t).map(|l| l.resolve(0)).unwrap_or(0).max(0))
        .collect();
    match parts.len() {
        1 => (parts[0], parts[0], parts[0], parts[0]),
        2 => (parts[0], parts[1], parts[0], parts[1]),
        3 => (parts[0], parts[1], parts[2], parts[1]),
        n if n >= 4 => (parts[0], parts[1], parts[2], parts[3]),
        _ => (0, 0, 0, 0),
    }
}

// Compte les "pistes" (tracks) de premier niveau d'une liste CSS, en traitant
// `fonction(...)` (ex. minmax(0, 1fr)) comme une seule piste malgre ses espaces.
fn count_tracks(s: &str) -> u32 {
    let mut n = 0u32;
    let mut depth = 0i32;
    let mut in_tok = false;
    for c in s.chars() {
        match c {
            '(' => { depth += 1; in_tok = true; }
            ')' => { if depth > 0 { depth -= 1; } }
            ' ' | '\t' | ',' if depth == 0 => { if in_tok { n += 1; in_tok = false; } }
            _ => { in_tok = true; }
        }
    }
    if in_tok { n += 1; }
    n
}

// Compte les colonnes declarees par `grid-template-columns`, en developpant
// `repeat(n, ...)`. Ex : `repeat(3, 1fr)` -> 3 ; `1fr 200px 1fr` -> 3 ;
// `repeat(4, minmax(0,1fr))` -> 4.
fn count_grid_cols(val: &str) -> u8 {
    let v = val.trim();
    // Grilles responsives : `repeat(auto-fill|auto-fit, ...)` -> heuristique
    // adaptee aux fenetres etroites de Nautile (2 colonnes lisibles).
    if v.contains("auto-fill") || v.contains("auto-fit") { return 2; }
    if let Some(rest) = v.strip_prefix("repeat(") {
        if let Some(comma) = rest.find(',') {
            if let Ok(n) = rest[..comma].trim().parse::<u32>() {
                let inner = &rest[comma + 1..];
                let inner = inner.strip_suffix(')').unwrap_or(inner);
                let per = count_tracks(inner).max(1);
                return (n * per).min(12) as u8;
            }
        }
    }
    count_tracks(v).min(12) as u8
}

// ----------------------------------------------------------------------------
// Liste d'affichage
// ----------------------------------------------------------------------------

const PAD: i32 = 8;
// Hauteur approximative du viewport pour `position: fixed` et le bloc conteneur
// initial (la fenetre Nautile par defaut fait ~420px, moins le chrome).
const VIEWPORT_H: i32 = 380;
// Plafond de couches d'empilement. Les sites complexes (Wikipedia) declarent des
// milliers d'elements positionnes : sans plafond, `paint` itere toutes les
// couches a CHAQUE frame -> scroll fige. Au-dela, on rend l'element EN FLUX
// (contenu visible et defilable, au prix d'un positionnement approximatif).
const MAX_LAYERS: usize = 256;

// Element d'une ligne en cours (positions relatives au debut de ligne).
enum LineItem {
    Word { dx: i32, w: i32, s: String, color: u32, scale: usize, bold: bool, href: Option<String>, va: u8 },
    Img { dx: i32, w: i32, h: i32, idx: usize, va: u8 },
    Box { dx: i32, w: i32, h: i32, fill: u32, value: String },
    Frag { dx: i32, w: i32, h: i32, items: Vec<Item>, links: Vec<Link> },
}

// Decalage vertical d'un element inline de hauteur `ih` dans une ligne de
// hauteur `lh`, selon vertical-align (0 baseline≈bas, 1 middle, 2 top, 3 bottom).
fn va_offset(va: u8, ih: i32, lh: i32) -> i32 {
    match va {
        1 => ((lh - ih) / 2).max(0),     // middle
        2 => 0,                          // top
        _ => (lh - ih).max(0),           // baseline / bottom
    }
}

// Fragment mis en page dans son propre espace (coordonnees relatives a 0,0).
struct Frag { items: Vec<Item>, links: Vec<Link>, width: i32, height: i32, nat: i32 }

// Etat partage entre la page et tous les sous-fragments (images, budget, etc.).
struct Ctx<'a> {
    css: &'a [Rule],
    css_index: CssIndex,
    css_vars: Vec<(String, String)>,
    images: Vec<Image>,
    img_cache: Vec<(String, usize)>,
    img_budget: u32,
    scheme: String,
    host: String,
    title: String,
    visited: usize,
    // Pile des index DOM des ancetres de l'element courant (racine→parent),
    // partagee entre tous les sous-fragments. Permet au matcher d'acceder aux
    // attributs, a la fratrie et aux combinateurs via le DOM.
    ancestors: Vec<usize>,
    // Pile de contexte de liste : (ordonnee, prochain index, style de marqueur).
    list_stack: Vec<(bool, i32, u8)>,
    // --- Positionnement (P1) ---
    // Couches positionnees collectees pendant le layout (absolute/fixed/relative+z).
    layers: Vec<Layer>,
    // Pile des blocs conteneurs (content box en coords document) etablis par les
    // ancetres positionnes ; le sommet sert de reference aux enfants `absolute`.
    cb_stack: Vec<(i32, i32, i32, i32)>,
    // Hauteur approximative du viewport (fenetre) pour `position: fixed` et le
    // bloc conteneur initial.
    viewport_h: i32,
    // Si vrai, display:none est ignoré (fallback quand le JS n'a pas pu retirer
    // les classes anti-FOUC — ex : Google cache le body jusqu'à JS prêt).
    no_display_none: bool,
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
            match crate::net::fetch_cached(&abs) {
                Some(b) => { self.img_budget -= 1; raw = b; }
                None => { crate::dlog!(crate::diag::Cat::Warn, "image injoignable: {}", abs); return None; }
            }
        }
        let img = match image::decode(&raw) {
            Some(im) => im,
            None => {
                crate::dlog!(crate::diag::Cat::Warn, "image non decodee ({}o, format non supporte?): {}", raw.len(), src);
                return None;
            }
        };
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
                LineItem::Word { dx, w, s, color, scale, bold, href, va } => {
                    let tx = base_x + dx;
                    let ty = y + va_offset(va, 8 * scale as i32, lh);
                    if let Some(h) = href { self.links.push(Link { x: tx, y, w, h: lh, href: h }); }
                    self.items.push(Item::Text { x: tx, y: ty, s, color, scale, bold });
                }
                LineItem::Img { dx, w, h, idx, va } => {
                    // Image inline : par defaut alignee sur la ligne de base (bas).
                    self.items.push(Item::Image { x: base_x + dx, y: y + va_offset(va, h, lh), w, h, idx });
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
        // text-transform (uppercase/lowercase/capitalize) applique au mot.
        let owned;
        let s: &str = if st.transform != 0 { owned = apply_transform(s, st.transform); &owned } else { s };
        // Largeurs PROPORTIONNELLES via la police vectorielle (repli monospace).
        let px = 8 * st.scale as i32;
        let cw = super::font_ttf::char_width(' ', px).max(1); // espace inter-mots
        let wpx = super::font_ttf::text_width(s, px);
        // Hauteur de ligne : line-height explicite, sinon taille + interligne.
        let lh = st.line_h.map(|l| l.max(px)).unwrap_or(px + LINE_GAP);
        if lh > self.line_h { self.line_h = lh; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += cw; }
        // white-space: nowrap -> jamais de retour a la ligne automatique.
        if !st.nowrap && cur + wpx > self.avail && cur > 0 { self.flush_line(); cur = 0; if lh > self.line_h { self.line_h = lh; } }
        self.line.push(LineItem::Word { dx: cur, w: wpx, s: s.to_string(), color: st.color, scale: st.scale, bold: st.bold, href: st.href.clone(), va: st.va });
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

    fn push_image(&mut self, idx: usize, va: u8) {
        let (iw, ih) = { let im = &self.ctx.images[idx]; (im.w as i32, im.h as i32) };
        let lh = ih + 4;
        if lh > self.line_h { self.line_h = lh; }
        let mut cur = self.line_cursor();
        if cur > 0 { cur += 8; }
        if cur + iw > self.avail && cur > 0 { self.flush_line(); cur = 0; if lh > self.line_h { self.line_h = lh; } }
        self.line.push(LineItem::Img { dx: cur, w: iw, h: ih, idx, va });
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

// Genere les rectangles d'un degrade lineaire 2 teintes sur la boite (x,y,w,h).
// `vertical` => variation haut→bas, sinon gauche→droite. ~16 bandes suffisent
// visuellement pour les fenetres Nautile.
fn paint_gradient_bands(x: i32, y: i32, w: i32, h: i32, c1: u32, c2: u32, vertical: bool) -> Vec<Item> {
    let span = if vertical { h } else { w };
    let n = span.clamp(1, 16);
    let mut out = Vec::with_capacity(n as usize);
    for k in 0..n {
        let col = lerp_rgb(c1, c2, k, n - 1);
        if vertical {
            let y0 = y + span * k / n;
            let y1 = y + span * (k + 1) / n;
            out.push(Item::Rect { x, y: y0, w, h: (y1 - y0).max(1), color: col });
        } else {
            let x0 = x + span * k / n;
            let x1 = x + span * (k + 1) / n;
            out.push(Item::Rect { x: x0, y, w: (x1 - x0).max(1), h, color: col });
        }
    }
    out
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

#[inline]
fn block_tag(t: &str) -> bool { super::html::tags::is_block(t) }

#[inline]
fn heading_scale(t: &str) -> Option<usize> { super::html::tags::heading_scale(t) }

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
fn layout(dom: &Dom, base_url: &str, width: i32, css: &[Rule], no_display_none: bool) -> Page {
    let (scheme, host) = scheme_host(base_url);
    let mut bg = 0xffffff_u32;
    let mut css_vars: Vec<(String, String)> = Vec::new();
    for r in css {
        // Fond global : regles ciblant body/html ou :root/* (dernier composant).
        let target = r.chain.last();
        let hits_root = matches!(target, Some(s) if s.is_root_tag() || s.is_any());
        if hits_root {
            for (p, v) in &r.decls {
                if p == "background" || p == "background-color" {
                    if let Some(c) = parse_color(v.split(' ').next().unwrap_or(v)) { bg = c; }
                }
                if p.starts_with("--") { css_vars.push((p.clone(), v.clone())); }
            }
        }
    }
    let content_w = (width - 2 * PAD).max(40);
    let mut ctx = Ctx {
        css, css_index: CssIndex::new(css), css_vars, images: Vec::new(), img_cache: Vec::new(), img_budget: 96,
        scheme: scheme.to_string(), host: host.to_string(), title: String::new(), visited: 0,
        ancestors: Vec::new(),
        list_stack: Vec::new(),
        layers: Vec::new(),
        // Bloc conteneur initial = viewport (origine du contenu, largeur, hauteur approx).
        cb_stack: alloc::vec![(PAD, PAD, content_w, VIEWPORT_H)],
        viewport_h: VIEWPORT_H,
        no_display_none,
    };
    let mut f = Flow::new(&mut ctx, PAD, content_w);
    f.y = PAD;
    walk(&mut f, dom, 0, &default_style(), 0);
    f.flush_line();
    let height = f.y + PAD;
    let items = core::mem::take(&mut f.items);
    let mut links = core::mem::take(&mut f.links);
    drop(f);
    // Recupere les couches positionnees, triees par z (stable = ordre document).
    let mut layers = core::mem::take(&mut ctx.layers);
    layers.sort_by_key(|l| l.z);
    // Les liens des couches participent au hit-testing (coords document).
    for l in &layers { for lk in &l.links { links.push(Link { x: lk.x, y: lk.y, w: lk.w, h: lk.h, href: lk.href.clone() }); } }
    Page { title: ctx.title, items, links, images: ctx.images, height, bg, layers }
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


fn apply_decls(decls: &[(String, String)], st: &mut Style, bx: &mut BoxProps, css_vars: &[(String, String)], no_display_none: bool) {
    for (p, v) in decls {
        if p.starts_with("--") { continue; } // proprietes CSS custom (deja collectees)
        let resolved;
        let val = if v.contains("var(") { resolved = resolve_var(v, css_vars); resolved.trim() } else { v.trim() };
        match p.as_str() {
            "color" => { if let Some(c) = parse_color(val) { st.color = c; } }
            "background" | "background-color" | "background-image" => {
                if val.contains("linear-gradient(") {
                    if let Some((c1, c2, vert)) = parse_gradient(val) {
                        bx.grad = Some((c1, c2, vert));
                        bx.bg = Some(c1); // couleur de repli (et fond sous le degrade)
                    }
                } else if let Some(c) = parse_color(val.split(',').next().unwrap_or(val).split(' ').next().unwrap_or(val)) {
                    bx.bg = Some(c);
                }
            }
            "font-size" => { if let Some(px) = font_px(val) { st.scale = px_to_scale(px); } }
            "font-weight" => { if val == "bold" || val == "bolder" || val == "700" || val == "800" || val == "900" { st.bold = true; } else if val == "normal" || val == "400" { st.bold = false; } }
            "text-align" => { st.align = match val { "center" => 1, "right" => 2, _ => 0 }; }
            "white-space" => {
                if val == "pre" || val == "pre-wrap" { st.pre = true; }
                if val == "nowrap" || val == "pre" { st.nowrap = true; }
            }
            "line-height" => {
                let v = val.trim();
                let base = 8 * st.scale as i32;
                if v == "normal" { st.line_h = None; }
                else if let Some(px) = v.strip_suffix("px") { st.line_h = px.trim().parse::<f32>().ok().map(|x| x as i32); }
                else if v.ends_with('%') { st.line_h = v.trim_end_matches('%').trim().parse::<f32>().ok().map(|x| (base as f32 * x / 100.0) as i32); }
                else if let Some(em) = v.strip_suffix("em") { st.line_h = em.trim().parse::<f32>().ok().map(|x| (base as f32 * x) as i32); }
                else if let Ok(mult) = v.parse::<f32>() { st.line_h = Some((base as f32 * mult) as i32); } // unitless
            }
            "text-transform" => {
                st.transform = match val { "uppercase" => 1, "lowercase" => 2, "capitalize" => 3, _ => 0 };
            }
            "vertical-align" => {
                st.va = match val { "middle" => 1, "top" | "text-top" => 2, "bottom" | "text-bottom" => 3, _ => 0 };
            }
            "list-style-type" | "list-style" => {
                // `list-style: none` ou un type ; on lit le premier mot reconnu.
                for tok in val.split_whitespace() {
                    let s = match tok {
                        "none" => 1, "disc" => 2, "circle" => 3, "square" => 4,
                        "decimal" => 5, _ => 0,
                    };
                    if s != 0 { bx.list_style = s; break; }
                }
            }
            // --- Positionnement (P1) ---
            "position" => {
                bx.position = match val {
                    "relative" => Pos::Relative,
                    "absolute" => Pos::Absolute,
                    "fixed" => Pos::Fixed,
                    "sticky" | "-webkit-sticky" => Pos::Sticky,
                    _ => Pos::Static,
                };
            }
            "top" => { bx.top = parse_len(val); }
            "right" => { bx.right = parse_len(val); }
            "bottom" => { bx.bottom = parse_len(val); }
            "left" => { bx.left = parse_len(val); }
            "z-index" => { bx.z_index = val.trim().parse::<i32>().ok(); }
            "overflow" | "overflow-x" | "overflow-y" => {
                if matches!(val, "hidden" | "clip" | "auto" | "scroll") { bx.overflow_clip = true; }
            }
            // box-shadow : on garde uniquement la couleur (ombre portee discrete).
            "box-shadow" => {
                if val != "none" {
                    let col = val.split(',').next().unwrap_or(val)
                        .split_whitespace().find_map(parse_color)
                        .unwrap_or(0x30000000 & 0xffffff);
                    bx.shadow = Some(col);
                }
            }
            "display" => {
                let d = match val { "none" => Disp::None, "inline" => Disp::Inline, "inline-block" => Disp::InlineBlock,
                    "flex" | "inline-flex" => Disp::Flex, "grid" | "inline-grid" => Disp::Grid, _ => Disp::Block };
                if d == Disp::None && !no_display_none { bx.hidden = true; }
                bx.disp = Some(d);
            }
            "flex-direction" => { if val.starts_with("column") { bx.flex_dir = FlexDir::Column; } else { bx.flex_dir = FlexDir::Row; } }
            "justify-content" => {
                bx.justify = match val {
                    "center" => Justify::Center,
                    "flex-end" | "end" | "right" => Justify::End,
                    "space-between" => Justify::Between,
                    "space-around" | "space-evenly" => Justify::Around,
                    _ => Justify::Start,
                };
            }
            "align-items" => {
                bx.align_items = match val {
                    "center" => AlignI::Center,
                    "flex-end" | "end" => AlignI::End,
                    "flex-start" | "start" => AlignI::Start,
                    _ => AlignI::Stretch,
                };
            }
            "gap" | "grid-gap" | "row-gap" | "column-gap" | "grid-column-gap" | "grid-row-gap" => {
                if let Some(px) = first_len(val) { bx.gap = bx.gap.max(px.max(0)); }
            }
            "grid-template-columns" => { bx.grid_cols = count_grid_cols(val); }
            "grid-column" => {
                // `1 / -1` = pleine rangee ; `span N` = N colonnes.
                if val.contains("-1") { bx.grid_span = 255; }
                else if let Some(rest) = val.split("span").nth(1) {
                    if let Ok(nn) = rest.trim().split(|c: char| c == '/' || c == ' ').next().unwrap_or("").trim().parse::<u8>() { bx.grid_span = nn; }
                }
            }
            "flex" | "flex-grow" => {
                // `flex: 1`, `flex: 1 1 0`, `flex-grow: 2` -> facteur de croissance.
                if let Some(tok) = val.split_whitespace().next() {
                    if let Ok(g) = tok.parse::<f32>() { bx.flex_grow = g as i32; }
                    else if tok == "auto" { bx.flex_grow = 1; }
                }
            }
            "visibility" => { /* non implementee : on ne masque pas (anti-FOUC) */ }
            "width" => { bx.width = parse_len(val); }
            "height" => { bx.height = parse_len(val); }
            "max-width" => { bx.max_width = parse_len(val); }
            "min-width" => { bx.min_width = parse_len(val); }
            "min-height" => { bx.min_height = parse_len(val); }
            "border-radius" => { if let Some(px) = first_len(val) { bx.radius = px.max(0); } }
            "transform" => {
                let (tx, ty, sc) = parse_transform(val);
                bx.tx += tx; bx.ty += ty; bx.scale_pct = bx.scale_pct * sc / 100;
            }
            "float" => { bx.float = match val { "left" => FloatK::Left, "right" => FloatK::Right, _ => FloatK::None }; }
            "margin" => {
                if val.contains("auto") { bx.center = true; }
                let (t, _r, b, _l) = parse_sides(val);
                bx.mar_t = t; bx.mar_b = b;
            }
            "margin-left" | "margin-right" => { if val.contains("auto") { bx.center = true; } }
            "margin-top" => { if let Some(px) = first_len(val) { bx.mar_t = px.max(0); } }
            "margin-bottom" => { if let Some(px) = first_len(val) { bx.mar_b = px.max(0); } }
            "padding" => {
                let (t, r, b, l) = parse_sides(val);
                bx.pad_t = t; bx.pad_r = r; bx.pad_b = b; bx.pad_l = l;
            }
            "padding-top" => { if let Some(px) = first_len(val) { bx.pad_t = px.max(0); } }
            "padding-right" => { if let Some(px) = first_len(val) { bx.pad_r = px.max(0); } }
            "padding-bottom" => { if let Some(px) = first_len(val) { bx.pad_b = px.max(0); } }
            "padding-left" => { if let Some(px) = first_len(val) { bx.pad_l = px.max(0); } }
            // `border: 1px solid #ccc` -> epaisseur + couleur ; le style (solid...) est ignore.
            "border" | "border-width" | "border-top" | "border-bottom" | "border-left" | "border-right" => {
                for tok in val.split_whitespace() {
                    if let Some(l) = parse_len(tok) { let w = l.resolve(0); if w > 0 { bx.border_w = w; } }
                    else if let Some(c) = parse_color(tok) { bx.border_color = c; if bx.border_w == 0 { bx.border_w = 1; } }
                }
            }
            "border-color" => { if let Some(c) = parse_color(val) { bx.border_color = c; if bx.border_w == 0 { bx.border_w = 1; } } }
            "border-style" => { if val != "none" && bx.border_w == 0 { bx.border_w = 1; } }
            "opacity" => { /* opacity non implémentée ; on ne masque pas le contenu (Google anti-FOUC) */ }
            _ => {}
        }
    }
}

// Calcule le style herite + la boite de l'element (cascade CSS + style inline).
fn compute(f: &Flow, dom: &Dom, idx: usize, node: &Node, tag: &str, st: &Style) -> (Style, BoxProps) {
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
    // Chemin DOM racine→element courant pour le matcher (attributs + fratrie).
    let mut path: Vec<usize> = f.ctx.ancestors.clone();
    path.push(idx);
    let mut matched: Vec<(u32, usize)> = Vec::new();
    let consider = |ri: usize, matched: &mut Vec<(u32, usize)>| {
        let r = &f.ctx.css[ri];
        if rule_matches(&r.chain, dom, &path) {
            matched.push((r.spec, ri));
        }
    };
    for &ri in &f.ctx.css_index.any { consider(ri, &mut matched); }
    for &ri in f.ctx.css_index.tags(tag) { consider(ri, &mut matched); }
    if !id.is_empty() {
        for &ri in f.ctx.css_index.ids(&id) { consider(ri, &mut matched); }
    }
    for cl in classes.split(' ').filter(|c| !c.is_empty()) {
        for &ri in f.ctx.css_index.classes(cl) { consider(ri, &mut matched); }
    }
    // Cascade CSS : specificite croissante puis ordre source croissant.
    matched.sort_by_key(|&(spec, ri)| (spec, ri));
    let ndn = f.ctx.no_display_none;
    for (_, ri) in matched { apply_decls(&f.ctx.css[ri].decls, &mut cst, &mut bx, &f.ctx.css_vars, ndn); }
    if let Some(style) = attr(node, "style") { apply_decls(&parse_decls(style), &mut cst, &mut bx, &f.ctx.css_vars, false); }
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
    // SVG inline (icones) : non rendable utilement, et son <title> clobberait le
    // titre de page. On ignore tout le sous-arbre, comme une image sans alt.
    if tag == "svg" || tag == "template" || tag == "noscript" { return; }
    if tag == "head" { for &c in &node.children { walk(f, dom, c, st, depth + 1); } return; }
    if tag == "title" {
        let mut t = String::new();
        for &c in &node.children { if dom.nodes[c].tag.is_none() { t.push_str(&dom.nodes[c].text); } }
        f.ctx.title = t.trim().to_string();
        return;
    }

    let (cst, bx) = compute(f, dom, idx, node, tag, st);
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
        // Resolution lazy-load / srcset (web moderne).
        if let Some(src) = img_src(node) {
            if let Some(i) = f.ctx.load_image(src, maxw, maxh) { f.push_image(i, cst.va); return; }
        }
        // Echec de chargement / format non supporte (SVG, WebP...) :
        // comportement d'un vrai navigateur — on affiche le texte `alt` discret
        // (pas de grosse boite grise). Les images decoratives (alt vide, icones,
        // pixels de tracking) sont ignorees pour ne pas encombrer la page.
        let alt = attr(node, "alt").unwrap_or("").trim();
        if !alt.is_empty() {
            let muted = Style { color: 0x9aa0a6, scale: cst.scale.min(2), ..cst.clone() };
            f.push_word(alt, &muted);
        }
        return;
    }
    if tag == "video" || tag == "audio" {
        // Pas de pile de décodage A/V : on rend un substitut cohérent.
        // <video poster="..."> -> on charge l'affiche comme image.
        if tag == "video" {
            if let Some(poster) = attr(node, "poster") {
                let maxw = bx.width.map(|l| l.resolve(f.avail)).unwrap_or(f.avail / 2).clamp(32, f.avail) as usize;
                let maxh = bx.height.map(|l| l.resolve(0)).unwrap_or(360).max(32) as usize;
                if let Some(i) = f.ctx.load_image(poster, maxw, maxh) { f.push_image(i, cst.va); return; }
            }
        }
        // Libellé déduit du type des <source> ou de l'attribut src.
        let mut label = if tag == "video" { "▶ Vidéo".to_string() } else { "♪ Audio".to_string() };
        let kid_src = node.children.iter().find_map(|&c| {
            let k = &dom.nodes[c];
            if k.tag.as_deref() == Some("source") { attr(k, "type").or(attr(k, "src")) } else { None }
        });
        if let Some(t) = kid_src.or_else(|| attr(node, "type")).or_else(|| attr(node, "src")) {
            if t.contains('/') && !t.contains('.') {
                label = alloc::format!("{} — {}", if tag == "video" { "▶" } else { "♪" }, crate::browser::engine::media::label_for_mime(t));
            }
        }
        let w = bx.width.map(|l| l.resolve(f.avail)).unwrap_or((label.len() as i32 * 8 + 32).min(f.avail)).clamp(48, f.avail);
        let h = bx.height.map(|l| l.resolve(0)).unwrap_or(if tag == "video" { 180 } else { 36 }).clamp(28, 720);
        f.push_box(w, h, 0xeeeeee, label);
        return;
    }
    if tag == "input" || tag == "textarea" || tag == "select" {
        let input_type = attr(node, "type").unwrap_or("text").to_ascii_lowercase();
        // inputs cachés : aucun rendu
        if input_type == "hidden" { return; }
        let cw = 8 * 2;
        let h = bx.height.map(|l| l.resolve(0)).unwrap_or(cw + 8).clamp(cw, 60);
        let default_w = if tag == "textarea" { f.avail } else { (20 * cw).min(f.avail * 3 / 4) };
        let w = bx.width.map(|l| l.resolve(f.avail)).unwrap_or(default_w).clamp(cw, f.avail);
        let is_submit = input_type == "submit" || input_type == "button" || input_type == "reset" || tag == "button";
        if is_submit {
            // Bouton : fond bleu Google (ou gris) avec texte centré
            let label = attr(node, "value").unwrap_or(if input_type == "reset" { "Effacer" } else { "Rechercher" }).to_string();
            let fill = bx.bg.filter(|&c| c != 0xffffff && c != 0).unwrap_or(0xf8f9fa);
            f.push_box(w.max(label.len() as i32 * 7 + 16), h, fill, label);
        } else if input_type == "checkbox" || input_type == "radio" {
            let sz = 14;
            f.push_box(sz, sz, 0xffffff, String::new());
        } else {
            // Champ texte : fond blanc, texte de valeur ou placeholder grisé
            let val = attr(node, "value").filter(|v| !v.is_empty())
                .map(|v| v.to_string())
                .unwrap_or_else(|| attr(node, "placeholder").unwrap_or("").to_string());
            f.push_box(w, h, 0xffffff, val);
        }
        return;
    }

    // Mode d'affichage : explicite (CSS) sinon defaut par balise. tr -> flex.
    let mut disp = bx.disp.unwrap_or(if block_tag(tag) { Disp::Block } else { Disp::Inline });
    if tag == "tr" { disp = Disp::Flex; }
    if bx.float != FloatK::None && disp == Disp::Block { disp = Disp::InlineBlock; }

    // Empile l'element (index DOM) comme ancetre pour le matching de ses enfants.
    f.ctx.ancestors.push(idx);

    // --- Positionnement (P1) ---
    // absolute / fixed : hors flux, place dans une couche dediee (z-index).
    // Au-dela du plafond de couches, on retombe en flux normal (cf. MAX_LAYERS).
    if matches!(bx.position, Pos::Absolute | Pos::Fixed) && f.ctx.layers.len() < MAX_LAYERS {
        layout_positioned(f, dom, node, &cst, &bx, tag, disp, depth);
        f.ctx.ancestors.pop();
        return;
    }

    // relative / sticky : reste dans le flux, mais on capture la plage produite
    // pour la decaler (top/left...) et/ou la promouvoir en couche (z-index).
    let positioned_inflow = matches!(bx.position, Pos::Relative | Pos::Sticky);
    let it0 = f.items.len();
    let lk0 = f.links.len();
    let lay0 = f.ctx.layers.len();

    match disp {
        Disp::None => {}
        Disp::Inline => { for &c in &node.children { walk(f, dom, c, &cst, depth + 1); } }
        Disp::InlineBlock => {
            let w = bx.width.map(|l| l.resolve(f.avail)).unwrap_or(f.avail).clamp(8, f.avail);
            let frag = make_frag(f, dom, node, &cst, &bx, tag, w, !bx.width.is_some(), depth + 1);
            f.push_frag(frag, 8);
        }
        Disp::Flex => { block_layout(f, dom, node, &cst, &bx, tag, Disp::Flex, depth); }
        Disp::Grid => { block_layout(f, dom, node, &cst, &bx, tag, Disp::Grid, depth); }
        Disp::Block => { block_layout(f, dom, node, &cst, &bx, tag, Disp::Block, depth); }
    }

    if positioned_inflow {
        // Decalage relatif a la position normale (sticky approxime en relative).
        let (cbw, cbh) = f.ctx.cb_stack.last().map(|&(_, _, w, h)| (w, h)).unwrap_or((f.avail, f.ctx.viewport_h));
        let dx = bx.left.map(|l| l.resolve(cbw)).or_else(|| bx.right.map(|l| -l.resolve(cbw))).unwrap_or(0);
        let dy = bx.top.map(|l| l.resolve(cbh)).or_else(|| bx.bottom.map(|l| -l.resolve(cbh))).unwrap_or(0);
        if dx != 0 || dy != 0 {
            for it in &mut f.items[it0..] { translate_item(it, dx, dy); }
            for lk in &mut f.links[lk0..] { lk.x += dx; lk.y += dy; }
            for l in &mut f.ctx.layers[lay0..] {
                for it in &mut l.items { translate_item(it, dx, dy); }
                for lk in &mut l.links { lk.x += dx; lk.y += dy; }
                if let Some((cx, cy, cw, ch)) = l.clip { l.clip = Some((cx + dx, cy + dy, cw, ch)); }
            }
        }
        // z-index : promeut la plage produite dans une couche d'empilement, sauf
        // si le plafond est atteint (on garde alors la plage en flux a z=0).
        if let Some(z) = bx.z_index {
            if f.ctx.layers.len() < MAX_LAYERS {
                let items: Vec<Item> = f.items.drain(it0..).collect();
                let links: Vec<Link> = f.links.drain(lk0..).collect();
                f.ctx.layers.push(Layer { z, fixed: false, clip: None, items, links });
            }
        }
    }

    f.ctx.ancestors.pop();
}

// Place un element `position: absolute|fixed` dans une couche dediee. Le sous-arbre
// est mis en page dans son propre espace (origine 0,0), puis translate a la position
// calculee depuis le bloc conteneur (absolute) ou le viewport (fixed).
fn layout_positioned(f: &mut Flow, dom: &Dom, node: &Node, cst: &Style, bx: &BoxProps, tag: &str, disp: Disp, depth: u32) {
    let fixed = bx.position == Pos::Fixed;
    // Bloc conteneur : viewport pour `fixed`, sinon le plus proche ancetre positionne.
    let (cbx, cby, cbw, cbh) = if fixed {
        (PAD, 0, f.avail.max(40), f.ctx.viewport_h)
    } else {
        *f.ctx.cb_stack.last().unwrap_or(&(PAD, PAD, f.avail, f.ctx.viewport_h))
    };

    let lft = bx.left.map(|l| l.resolve(cbw));
    let rgt = bx.right.map(|l| l.resolve(cbw));
    let top = bx.top.map(|l| l.resolve(cbh));
    let bot = bx.bottom.map(|l| l.resolve(cbh));

    // Largeur : explicite, sinon left+right impose la largeur, sinon pleine CB.
    let width = if let Some(w) = bx.width { w.resolve(cbw) }
        else if let (Some(l), Some(r)) = (lft, rgt) { (cbw - l - r).max(8) }
        else { cbw };
    let width = width.clamp(8, cbw.max(8));

    // Mise en page du sous-arbre dans un sous-flux a l'origine (0,0).
    let inner_disp = match disp { Disp::Flex => Disp::Flex, Disp::Grid => Disp::Grid, _ => Disp::Block };
    let layer_start = f.ctx.layers.len();
    // block_layout etablit lui-meme le bloc conteneur local (element positionne).
    let mut sub = Flow::new(f.ctx, 0, width);
    sub.y = 0;
    block_layout(&mut sub, dom, node, cst, bx, tag, inner_disp, depth);
    sub.flush_line();
    let h = sub.y.max(1);
    let mut items = core::mem::take(&mut sub.items);
    let mut links = core::mem::take(&mut sub.links);
    drop(sub);

    // Position finale dans le document (ou le viewport pour fixed).
    let px = if let Some(l) = lft { cbx + l }
        else if let Some(r) = rgt { cbx + cbw - r - width }
        else { cbx };
    let py = if let Some(t) = top { cby + t }
        else if let Some(b) = bot { cby + cbh - b - h }
        else { cby };

    for it in &mut items { translate_item(it, px, py); }
    for lk in &mut links { lk.x += px; lk.y += py; }
    // Translate aussi les couches imbriquees (descendants absolus) generees.
    for l in &mut f.ctx.layers[layer_start..] {
        for it in &mut l.items { translate_item(it, px, py); }
        for lk in &mut l.links { lk.x += px; lk.y += py; }
        if let Some((cx, cy, cw, ch)) = l.clip { l.clip = Some((cx + px, cy + py, cw, ch)); }
    }

    let clip = if bx.overflow_clip { Some((px, py, width, h)) } else { None };
    f.ctx.layers.push(Layer { z: bx.z_index.unwrap_or(0), fixed, clip, items, links });
}

// Met en page le contenu d'un element dans son propre fragment (largeur `w`).
// `shrink` : ajuste la largeur finale au contenu (inline-block sans width).
fn make_frag(f: &mut Flow, dom: &Dom, node: &Node, cst: &Style, bx: &BoxProps, tag: &str, w: i32, shrink: bool, depth: u32) -> Frag {
    let mut sub = Flow::new(f.ctx, 0, w);
    sub.align = cst.align;
    if tag == "li" { let b = Style { color: 0x5f6368, ..cst.clone() }; sub.push_word("\u{2022}", &b); }
    for &c in &node.children { walk(&mut sub, dom, c, cst, depth + 1); }
    sub.flush_line();
    let nat = sub.used_w.max(1);
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
    Frag { items, links, width: fw, height: h, nat }
}

// Bloc en flux normal : nouvelle ligne, largeur eventuellement contrainte
// (width/max-width) et centree (margin:auto), fond, marges verticales.
fn block_layout(f: &mut Flow, dom: &Dom, node: &Node, cst: &Style, bx: &BoxProps, tag: &str, inner_disp: Disp, depth: u32) {
    f.flush_line();
    if heading_scale(tag).is_some() && f.y > PAD { f.y += LINE_GAP; }

    // Marge haute (box model).
    f.y += bx.mar_t;

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
    let (pt, pr, pb, pl) = (bx.pad_t, bx.pad_r, bx.pad_b, bx.pad_l);
    // Largeur du CONTENU = boite - bordures - paddings (gauche + droite).
    let inner = (outer - 2 * bw - pl - pr).max(8);

    let box_top = f.y;
    let bg_insert = f.items.len();
    let link_insert = f.links.len();
    let (sx0, sav, sal) = (f.x0, f.avail, f.align);
    f.x0 = sx0 + left + bw + pl;    // contenu insere par bordure + padding gauche
    f.avail = inner;
    f.align = cst.align;
    f.y += bw + pt;                 // bordure + padding du haut

    // Contexte de liste : ul/ol empilent (ordonnee, index de depart, style) pour
    // que leurs <li> produisent puces ou numeros.
    let is_list = tag == "ul" || tag == "ol";
    if is_list {
        let ordered = tag == "ol";
        let start = if ordered {
            attr(node, "start").and_then(|s| s.trim().parse::<i32>().ok()).unwrap_or(1)
        } else { 1 };
        f.ctx.list_stack.push((ordered, start, bx.list_style));
    }

    // Bloc conteneur : un element positionne (relative/absolute/fixed/sticky)
    // sert de reference aux descendants `absolute`. Content box en coords courantes.
    let establishes_cb = bx.position != Pos::Static;
    if establishes_cb { f.ctx.cb_stack.push((f.x0, f.y, inner, f.ctx.viewport_h)); }

    // Contenu : flux normal, ou disposition flex/grid si le conteneur le demande.
    match inner_disp {
        Disp::Flex => flex_inner(f, dom, node, cst, bx, depth),
        Disp::Grid => grid_inner(f, dom, node, cst, bx, depth),
        _ => {
            if tag == "li" {
                let marker = if f.ctx.list_stack.is_empty() { Some("\u{2022}".to_string()) } else { li_marker(f.ctx) };
                if let Some(m) = marker {
                    let b = Style { color: 0x5f6368, ..cst.clone() };
                    f.push_word(&m, &b);
                }
            }
            for &c in &node.children { walk(f, dom, c, cst, depth + 1); }
            f.flush_line();
        }
    }

    if establishes_cb { f.ctx.cb_stack.pop(); }
    if is_list { f.ctx.list_stack.pop(); }

    f.y += pb + bw;               // bordure + padding du bas
    // height / min-height contraignent la hauteur finale de la boite.
    if let Some(hv) = bx.height { let target = box_top + hv.resolve(0); if f.y < target { f.y = target; } }
    if let Some(mn) = bx.min_height { let target = box_top + mn.resolve(0); if f.y < target { f.y = target; } }
    let box_bottom = f.y;
    f.x0 = sx0; f.avail = sav; f.align = sal;

    let h = (box_bottom - box_top).max(0);
    // Fond : degrade lineaire (bandes interpolees) ou couleur unie, sous le contenu.
    if h > 0 {
        if let Some((c1, c2, vert)) = bx.grad {
            let bx0 = sx0 + left;
            let bands = paint_gradient_bands(bx0, box_top, outer, h, c1, c2, vert);
            f.items.splice(bg_insert..bg_insert, bands);
        } else if let Some(bgc) = bx.bg {
            f.items.insert(bg_insert, if bx.radius > 0 { Item::RoundedRect { x: sx0 + left, y: box_top, w: outer, h, radius: bx.radius, color: bgc } } else { Item::Rect { x: sx0 + left, y: box_top, w: outer, h, color: bgc } });
        }
    }
    // box-shadow : rectangle decale (+4,+4), insere ENCORE en dessous du fond.
    if let Some(sh) = bx.shadow {
        if h > 0 { f.items.insert(bg_insert, if bx.radius > 0 { Item::RoundedRect { x: sx0 + left + 4, y: box_top + 4, w: outer, h, radius: bx.radius, color: sh } } else { Item::Rect { x: sx0 + left + 4, y: box_top + 4, w: outer, h, color: sh } }); }
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

    // Transform CSS simple : applique translate/scale au fragment visuel et aux liens.
    apply_box_transform(&mut f.items[bg_insert..], &mut f.links[link_insert..], sx0 + left, box_top, bx.tx, bx.ty, bx.scale_pct);

    // Marge basse + espacement par defaut entre blocs.
    f.y += bx.mar_b;
    if matches!(tag, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "blockquote" | "table" |
        "form" | "div" | "section" | "article" | "header" | "footer" | "li" | "tr" | "figure") { f.y += LINE_GAP; }
}

// Met en page un enfant dans son propre fragment de largeur `w`.
fn child_frag(f: &mut Flow, dom: &Dom, c: usize, cst: &Style, align: u8, w: i32, depth: u32) -> Frag {
    let mut sub = Flow::new(f.ctx, 0, w.max(8));
    sub.align = align;
    walk(&mut sub, dom, c, cst, depth + 1);
    sub.flush_line();
    let h = sub.y;
    let nat = sub.used_w.max(1);
    let items = core::mem::take(&mut sub.items);
    let links = core::mem::take(&mut sub.links);
    drop(sub);
    Frag { items, links, width: w.max(8), height: h, nat }
}

// Place un fragment a (x, y) absolus dans le flux courant (translation + collecte).
fn place_frag(f: &mut Flow, frag: Frag, x: i32, y: i32) {
    for mut sub in frag.items { translate_item(&mut sub, x, y); f.items.push(sub); }
    for mut lk in frag.links { lk.x += x; lk.y += y; f.links.push(lk); }
}

// Decalage vertical d'un item selon align-items dans une bande de hauteur `band`.
fn cross_offset(align: AlignI, item_h: i32, band: i32) -> i32 {
    match align {
        AlignI::Center => (band - item_h) / 2,
        AlignI::End => band - item_h,
        _ => 0,
    }
}

// Conteneur flexbox : `flex-direction` row/column, `gap`, `justify-content`,
// `align-items` et largeurs/flex-grow des enfants. Le `tr` d'un tableau reutilise
// ce moteur en mode row.
fn flex_inner(f: &mut Flow, dom: &Dom, node: &Node, cst: &Style, bx: &BoxProps, depth: u32) {
    let kids: Vec<usize> = node.children.iter().cloned().filter(|&c| dom.nodes[c].tag.is_some()).collect();
    if kids.is_empty() {
        let saved = f.align; f.align = cst.align;
        for &c in &node.children { walk(f, dom, c, cst, depth + 1); }
        f.flush_line(); f.align = saved;
        return;
    }

    // ── Direction colonne : empilement vertical avec gap. ──
    if bx.flex_dir == FlexDir::Column {
        for (i, &c) in kids.iter().enumerate() {
            if i > 0 { f.y += bx.gap; }
            let frag = child_frag(f, dom, c, cst, cst.align, f.avail, depth);
            let h = frag.height;
            // align-items horizontal : start (defaut/stretch) | center | end.
            let dx = match bx.align_items {
                AlignI::Center => ((f.avail - frag.width) / 2).max(0),
                AlignI::End => (f.avail - frag.width).max(0),
                _ => 0,
            };
            place_frag(f, frag, f.x0 + dx, f.y);
            f.y += h;
        }
        return;
    }

    // ── Direction ligne : largeur fixe (CSS) ou NATURELLE (contenu) de chaque
    // enfant, puis distribution flex-grow si ça tient, sinon passage à la ligne
    // (flex-wrap) — c'est ce qui évite les chevauchements de texte nowrap.
    let n = kids.len();
    let gap = bx.gap;

    let mut base: Vec<i32> = Vec::with_capacity(n);
    let mut grow: Vec<i32> = Vec::with_capacity(n);
    for &c in &kids {
        let cn = &dom.nodes[c];
        let ct = cn.tag.as_deref().unwrap_or("");
        let (_ccst, cbx) = compute(f, dom, c, cn, ct, cst);
        if let Some(w) = cbx.width {
            base.push(w.resolve(f.avail).clamp(8, f.avail));
            grow.push(0);
        } else {
            // Mesure la largeur naturelle du contenu (sans le forcer à rétrécir).
            let m = child_frag(f, dom, c, cst, cst.align, f.avail, depth);
            base.push(m.nat.clamp(8, f.avail));
            grow.push(if cbx.flex_grow > 0 { cbx.flex_grow } else { 0 });
        }
    }
    let gap_total = gap * (n as i32 - 1).max(0);
    let total: i32 = base.iter().sum::<i32>() + gap_total;

    if total <= f.avail {
        // Tient sur une ligne : l'espace restant va aux enfants flex-grow.
        let mut widths = base.clone();
        let sum_grow: i32 = grow.iter().sum();
        let free = (f.avail - total).max(0);
        if sum_grow > 0 {
            for i in 0..n { if grow[i] > 0 { widths[i] += free * grow[i] / sum_grow; } }
        }
        let mut frags: Vec<Frag> = Vec::with_capacity(n);
        let mut row_h = 0;
        for (i, &c) in kids.iter().enumerate() {
            let frag = child_frag(f, dom, c, cst, cst.align, widths[i], depth);
            if frag.height > row_h { row_h = frag.height; }
            frags.push(frag);
        }
        let used: i32 = widths.iter().sum::<i32>() + gap_total;
        let leftover = (f.avail - used).max(0);
        let (start, between) = match bx.justify {
            Justify::Center => (leftover / 2, gap),
            Justify::End => (leftover, gap),
            Justify::Between => (0, gap + if n > 1 { leftover / (n as i32 - 1) } else { 0 }),
            Justify::Around => (leftover / (2 * n as i32), gap + leftover / n as i32),
            Justify::Start => (0, gap),
        };
        let mut x = f.x0 + start;
        for (i, frag) in frags.into_iter().enumerate() {
            let dy = cross_offset(bx.align_items, frag.height, row_h).max(0);
            place_frag(f, frag, x, f.y + dy);
            x += widths[i] + between;
        }
        f.y += row_h;
    } else {
        // Débordement : on passe à la ligne (flex-wrap), au lieu de superposer.
        let mut x = f.x0;
        let mut row_h = 0;
        for &c in &kids {
            let cn = &dom.nodes[c];
            let ct = cn.tag.as_deref().unwrap_or("");
            let (_ccst, cbx) = compute(f, dom, c, cn, ct, cst);
            let w = cbx.width.map(|l| l.resolve(f.avail)).unwrap_or_else(|| {
                child_frag(f, dom, c, cst, cst.align, f.avail, depth).nat
            }).clamp(8, f.avail);
            if x > f.x0 && x + w > f.x0 + f.avail {
                f.y += row_h + gap;
                x = f.x0;
                row_h = 0;
            }
            let frag = child_frag(f, dom, c, cst, cst.align, w, depth);
            if frag.height > row_h { row_h = frag.height; }
            place_frag(f, frag, x, f.y);
            x += w + gap;
        }
        f.y += row_h;
    }
}

// Conteneur CSS grid : `grid-template-columns` definit le nombre de colonnes ;
// les items sont disposes en grille avec retour a la ligne et `gap`. Le span
// `grid-column: 1/-1` (ou `span N`) fait occuper plusieurs colonnes a une cellule.
fn grid_inner(f: &mut Flow, dom: &Dom, node: &Node, cst: &Style, bx: &BoxProps, depth: u32) {
    let kids: Vec<usize> = node.children.iter().cloned().filter(|&c| dom.nodes[c].tag.is_some()).collect();
    if kids.is_empty() { return; }
    let cols = (bx.grid_cols as i32).max(1);
    let gap = bx.gap;
    let avail = f.avail;
    let colw = ((avail - gap * (cols - 1)) / cols).clamp(24, avail);
    // Largeur d'une cellule couvrant `span` colonnes (gaps internes inclus).
    let span_w = |span: i32| -> i32 { (colw * span + gap * (span - 1)).clamp(24, avail) };

    let mut col = 0i32;          // colonne courante dans la rangee
    let mut x = f.x0;            // curseur horizontal
    let mut row_h = 0;           // hauteur max de la rangee courante
    let mut i = 0usize;
    while i < kids.len() {
        // Determine le span de la cellule (compute lit la BoxProps de l'enfant).
        let cn = &dom.nodes[kids[i]];
        let ct = cn.tag.as_deref().unwrap_or("");
        let (_ccst, cbx) = compute(f, dom, kids[i], cn, ct, cst);
        let span = match cbx.grid_span { 0 => 1, 255 => cols, s => (s as i32).min(cols) };

        // Retour a la ligne si la cellule ne tient pas dans la rangee.
        if col > 0 && col + span > cols {
            f.y += row_h + gap;
            col = 0; x = f.x0; row_h = 0;
        }

        let cw = span_w(span);
        let frag = child_frag(f, dom, kids[i], cst, cst.align, cw, depth);
        if frag.height > row_h { row_h = frag.height; }
        place_frag(f, frag, x, f.y);

        x += cw + gap;
        col += span;
        i += 1;
        if col >= cols { f.y += row_h + gap; col = 0; x = f.x0; row_h = 0; }
    }
    // Termine la derniere rangee partielle (si non close ci-dessus).
    if col > 0 { f.y += row_h; } else { f.y -= gap; }
}

fn scheme_host(base: &str) -> (&str, &str) {
    let (scheme, rest) = if let Some(r) = base.strip_prefix("https://") { ("https", r) }
        else if let Some(r) = base.strip_prefix("http://") { ("http", r) }
        else { ("http", base) };
    let host = match rest.find('/') { Some(i) => &rest[..i], None => rest };
    (scheme, host)
}

