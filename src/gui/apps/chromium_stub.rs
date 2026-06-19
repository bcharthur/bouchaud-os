//! Bouchaud Browser : navigateur graphique.
//!
//! Recupere le HTML (HTTP/1.1 + TLS, gzip/deflate/brotli), le parse en DOM, le
//! met en page (moteur `gui::web`) et le peint dans le framebuffer HD avec
//! defilement. Pages internes : `about:bouchaud`, `about:system`, `file:/...`.

use crate::gui::framebuffer as fb;
use crate::gui::web::{self, Page, Session};
use crate::gui::window::clip;
use crate::arch::x86_64::rtc;
use crate::fs::ramfs;
use crate::kernel::timer;
use crate::users;
use alloc::format;
use alloc::string::{String, ToString};

const ADDR_H: usize = 11;
const SCROLL_W: usize = 6;

// Echappe le texte brut pour l'injecter dans du HTML synthetise.
fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c { '&' => o.push_str("&amp;"), '<' => o.push_str("&lt;"), '>' => o.push_str("&gt;"), _ => o.push(c) }
    }
    o
}

fn page_from_html(html: &[u8], base: &str, width: i32) -> (Session, Page) {
    // Borne la taille du document analyse (les pages enormes type YouTube
    // depasseraient la memoire). Le rendu reste correct sur l'entete.
    let capped = &html[..html.len().min(4_000_000)];
    Session::open(capped, base, width)
}

/// Charge une URL et renvoie la session interactive + la page mise en page.
pub(crate) fn open(url: &str, width: i32) -> (Session, Page) {
    let width = width.max(80);
    if url == "about:bouchaud" {
        let html = format!(
            "<!doctype html><html><head><title>Bouchaud OS</title><style>\
             body{{background:#f5f7fb;color:#202124}}\
             .hero{{background:#1a73e8;color:#ffffff;padding:18px;text-align:center}}\
             .hero h1{{color:#ffffff}}\
             .v{{color:#d2e3fc}}\
             .card{{background:#ffffff;border:1px solid #dadce0;padding:10px;margin:8px}}\
             .card h3{{color:#1a73e8}}\
             a{{color:#1a73e8}}\
             </style></head><body>\
             <div class=\"hero\"><h1>Bouchaud OS</h1>\
             <p class=\"v\">Systeme souverain experimental &mdash; version {}</p></div>\
             <div class=\"card\"><h3>Applications</h3><ul>\
             <li><a href=\"about:calc\">Calculatrice</a> &mdash; appli native (moteur JS)</li>\
             <li><a href=\"about:system\">Informations systeme</a></li>\
             <li><a href=\"file:/readme.txt\">Lecteur de fichiers (readme.txt)</a></li>\
             </ul></div>\
             <div class=\"card\"><h3>Le Web, en vrai</h3>\
             <p>Noyau Rust no_std, pile reseau TLS 1.3, moteur de rendu HTML/CSS et\
             interpreteur JavaScript integres. Essaie une vraie page :</p><ul>\
             <li><a href=\"https://example.com/\">https://example.com/</a></li>\
             <li><a href=\"https://www.google.com/\">https://www.google.com/</a></li></ul></div>\
             <div class=\"card\"><h3>Astuce</h3><p>Tape une URL dans la barre d'adresse,\
             ou un numero seul pour suivre le lien correspondant.</p></div>\
             </body></html>",
            crate::VERSION);
        return page_from_html(html.as_bytes(), url, width);
    }
    if url == "about:calc" {
        return page_from_html(CALC_APP.as_bytes(), url, width);
    }
    if url == "about:system" {
        let dt = rtc::now();
        let (used, free, total) = crate::kernel::heap::stats();
        let html = format!(
            "<h1>Systeme</h1><ul><li>Heure : {:02}:{:02}:{:02}</li><li>Uptime : {} s</li>\
             <li>Heap : {}/{} o (libre {})</li><li>PCI : {} peripheriques</li>\
             <li>User : {}</li></ul>",
            dt.hour, dt.minute, dt.second, timer::seconds(), used, total, free,
            crate::arch::x86_64::pci::count(), users::session().username());
        return page_from_html(html.as_bytes(), url, width);
    }
    if let Some(path) = url.strip_prefix("file:") {
        let p = path.trim_start_matches('/');
        let full = format!("/{}", p);
        let fs = ramfs::fs();
        let body = match fs.resolve_checked(&full, 0) {
            Ok(idx) if fs.nodes[idx].kind == ramfs::NodeKind::File && fs.can(idx, ramfs::PERM_R) => {
                let mut s = String::new();
                for k in 0..fs.nodes[idx].content_len { s.push(fs.nodes[idx].content[k] as char); }
                format!("<h2>file:{}</h2><pre>{}</pre>", full, esc(&s))
            }
            Ok(_) => format!("<h2>Erreur</h2><p>permission refusee : {}</p>", full),
            _ => format!("<h2>Erreur</h2><p>introuvable : {}</p>", full),
        };
        return page_from_html(body.as_bytes(), url, width);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let doc = crate::net::fetch_document(url);
        if doc.ok && !doc.body.is_empty() {
            if doc.is_html {
                return page_from_html(&doc.body, &doc.final_url, width);
            }
            // Contenu non-HTML : affichage texte brut.
            let mut s = String::new();
            for &b in doc.body.iter().take(40_000) {
                match b { b'\n' | b'\r' | b'\t' => s.push(b as char), 0x20..=0x7e => s.push(b as char), _ => s.push('.') }
            }
            let html = format!("<h3>{} ({} o)</h3><pre>{}</pre>", esc(&doc.content_type), doc.body.len(), esc(&s));
            return page_from_html(html.as_bytes(), &doc.final_url, width);
        }
        // Echec : affiche le diagnostic (TLS, statut, erreurs).
        let mut html = String::from("<h2>Echec du chargement</h2><pre>");
        for line in &doc.banner { html.push_str(&esc(line)); html.push('\n'); }
        html.push_str("</pre>");
        return page_from_html(html.as_bytes(), url, width);
    }
    let html = format!("<h2>Page inconnue</h2><p>{}</p><p>Essaie about:bouchaud, https://example.com/</p>", esc(url));
    page_from_html(html.as_bytes(), url, width)
}

// Application calculatrice embarquee (HTML+CSS+JS) : demonstration d'une appli
// interactive tournant sur le moteur JS/DOM. Les boutons `onclick` rejouent du
// code dans le contexte JS persistant de la page (voir web::Session).
const CALC_APP: &str = r#"<!doctype html><html><head><title>Calculatrice</title>
<style>
body{background:#202124;color:#e8eaed;text-align:center}
#disp{background:#111;color:#8ab4f8;font-size:26px;max-width:260px;margin:8px auto;padding:8px}
.row{display:flex;max-width:260px;margin:0 auto}
button{background:#3c4043;color:#e8eaed;font-size:20px}
.op button{color:#fdd663}
</style></head><body>
<h2>Calculatrice</h2>
<div id="disp">0</div>
<div class="row"><button onclick="press('7')">7</button><button onclick="press('8')">8</button><button onclick="press('9')">9</button><div class="op"><button onclick="press('/')">/</button></div></div>
<div class="row"><button onclick="press('4')">4</button><button onclick="press('5')">5</button><button onclick="press('6')">6</button><div class="op"><button onclick="press('*')">*</button></div></div>
<div class="row"><button onclick="press('1')">1</button><button onclick="press('2')">2</button><button onclick="press('3')">3</button><div class="op"><button onclick="press('-')">-</button></div></div>
<div class="row"><button onclick="press('0')">0</button><button onclick="press('.')">.</button><div class="op"><button onclick="equals()">=</button></div><div class="op"><button onclick="press('+')">+</button></div></div>
<div class="row"><button onclick="clr()">C</button></div>
<script>
var cur='';
function press(c){ if(cur==='0')cur=''; cur+=c; show(); }
function clr(){ cur=''; show0(); }
function show(){ document.getElementById('disp').textContent = cur; }
function show0(){ document.getElementById('disp').textContent = '0'; }
function equals(){ cur = String(evalExpr(cur)); show(); }
// Evaluateur d'expression: tokenise, applique */ puis +-.
function evalExpr(s){
  var toks=[], num='';
  for(var i=0;i<s.length;i++){ var ch=s[i];
    if(ch>='0'&&ch<='9'||ch==='.'){ num+=ch; }
    else { if(num!==''){toks.push(parseFloat(num));num='';} toks.push(ch); } }
  if(num!=='')toks.push(parseFloat(num));
  var p=[], i=0;
  while(i<toks.length){ var t=toks[i];
    if(t==='*'||t==='/'){ var a=p.pop(), b=toks[i+1]; p.push(t==='*'?a*b:a/b); i+=2; }
    else { p.push(t); i++; } }
  var r=p[0]||0;
  for(i=1;i<p.length;i+=2){ if(p[i]==='+')r+=p[i+1]; else if(p[i]==='-')r-=p[i+1]; }
  return Math.round(r*1e6)/1e6;
}
</script></body></html>"#;

/// Normalise l'entree de la barre d'adresse en URL. Un numero seul suit le lien
/// correspondant de la page courante.
pub(crate) fn resolve_input(input: &str, page: &Page) -> String {
    let t = input.trim();
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = t.parse::<usize>() {
            if n >= 1 && n <= page.links.len() {
                return page.links[n - 1].href.clone();
            }
        }
    }
    if t.contains("://") || t.starts_with("about:") || t.starts_with("file:") {
        return t.to_string();
    }
    if t.contains('.') && !t.contains(' ') {
        return format!("https://{}", t);
    }
    t.to_string()
}

/// Hauteur maximale de defilement pour la zone de contenu donnee.
pub(crate) fn max_scroll(page: &Page, bh: usize) -> i32 {
    let view = bh.saturating_sub(ADDR_H) as i32;
    (page.height - view).max(0)
}

/// Convertit un clic dans la barre de defilement en position de scroll.
pub(crate) fn scroll_at(page: &Page, rel_x: i32, rel_y: i32, bw: usize, bh: usize) -> Option<i32> {
    let content_h = bh.saturating_sub(ADDR_H);
    let max = max_scroll(page, bh);
    if max <= 0 || content_h < 12 {
        return None;
    }
    let sx = bw.saturating_sub(SCROLL_W) as i32;
    if rel_x < sx || rel_x >= bw as i32 || rel_y < ADDR_H as i32 {
        return None;
    }
    let track_h = content_h as i32;
    let y = (rel_y - ADDR_H as i32).clamp(0, track_h - 1);
    let thumb_h = ((track_h * track_h) / page.height.max(track_h)).clamp(10, track_h);
    let travel = (track_h - thumb_h).max(1);
    Some(((y - thumb_h / 2).clamp(0, travel) * max) / travel)
}

/// Suit un clic dans la zone de contenu : renvoie l'URL du lien touche.
pub(crate) fn link_at(page: &Page, scroll: i32, rel_x: i32, rel_y: i32) -> Option<String> {
    let cy = rel_y - ADDR_H as i32 + scroll; // coordonnee dans le contenu
    let cx = rel_x;
    for lnk in &page.links {
        if cx >= lnk.x && cx < lnk.x + lnk.w && cy >= lnk.y && cy < lnk.y + lnk.h {
            return Some(lnk.href.clone());
        }
    }
    None
}

/// Affiche un ecran "Chargement..." (a peindre + present() avant le fetch
/// bloquant, pour un retour visuel immediat).
pub(crate) fn draw_loading(url: &str, bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect(bx, by, bw, bh, fb::C_WHITE);
    fb::draw_text_scaled(bx + 8, by + 10, "Chargement...", fb::C_BLACK, 2);
    fb::draw_text(bx + 8, by + 34, clip(url, bw / 8), fb::C_DKGRAY);
}

/// Dessine le navigateur : barre d'adresse + contenu peint.
pub(crate) fn draw(url: &str, input: &str, page: &Page, scroll: i32, bx: usize, by: usize, bw: usize, bh: usize) {
    // Barre d'adresse.
    fb::fill_rect(bx, by, bw, ADDR_H - 1, fb::C_WHITE);
    let shown = if input == url { input.to_string() } else { format!("{}_", input) };
    fb::draw_text(bx + 1, by + 1, clip(&shown, bw / 8), fb::C_BLACK);
    // Contenu.
    let cy = by + ADDR_H;
    let ch = bh.saturating_sub(ADDR_H);
    let content_w = if max_scroll(page, bh) > 0 { bw.saturating_sub(SCROLL_W) } else { bw };
    web::paint(page, scroll, bx, cy, content_w, ch);
    draw_scrollbar(page, scroll, bx, cy, bw, ch);
}

fn draw_scrollbar(page: &Page, scroll: i32, bx: usize, cy: usize, bw: usize, ch: usize) {
    let max = (page.height - ch as i32).max(0);
    if max <= 0 || ch < 12 || bw < SCROLL_W {
        return;
    }
    let sx = bx + bw - SCROLL_W;
    fb::fill_rect(sx, cy, SCROLL_W, ch, fb::C_WHITE);
    fb::rect(sx, cy, SCROLL_W, ch, fb::C_DKGRAY);
    let track_h = ch as i32;
    let thumb_h = ((track_h * track_h) / page.height.max(track_h)).clamp(10, track_h);
    let travel = (track_h - thumb_h).max(1);
    let thumb_y = cy as i32 + (scroll.clamp(0, max) * travel) / max;
    fb::fill_rect(sx + 1, thumb_y as usize, SCROLL_W - 2, thumb_h as usize, fb::C_GRAY);
}
