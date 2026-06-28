//! Journal d'activite du navigateur (devlog).
//!
//! Instrumente toute la pile — reseau, cache, CSS, JS, DOM, layout, peinture —
//! pour voir ce qui marche, ce qui echoue et ce qui est lent. Chaque evenement
//! est horodate (ticks PIT) et echo sur la sortie serie (COM1, visible dans
//! QEMU). Le tampon circulaire est consultable dans l'interface via `about:log`.
//!
//! Acces mono-thread (boucle GUI / navigateur) : `static mut` protege par le
//! contexte d'execution cooperatif.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq)]
pub enum Cat { Net, Cache, Css, Js, Dom, Layout, Paint, Info, Warn, Err }

impl Cat {
    pub fn tag(self) -> &'static str {
        match self {
            Cat::Net => "NET", Cat::Cache => "CACHE", Cat::Css => "CSS",
            Cat::Js => "JS", Cat::Dom => "DOM", Cat::Layout => "LAYOUT",
            Cat::Paint => "PAINT", Cat::Info => "INFO", Cat::Warn => "WARN",
            Cat::Err => "ERR",
        }
    }
    // Couleur d'affichage HTML pour `about:log`.
    fn color(self) -> &'static str {
        match self {
            Cat::Net => "#4fa8f0", Cat::Cache => "#9a7bd0", Cat::Css => "#34c26a",
            Cat::Js => "#f5c040", Cat::Dom => "#7fb4ee", Cat::Layout => "#e09040",
            Cat::Paint => "#5fd0c0", Cat::Info => "#9aa0a6", Cat::Warn => "#e0a030",
            Cat::Err => "#e0483a",
        }
    }
}

struct Entry { tick: u64, cat: Cat, msg: String }

static mut LOG: Option<Vec<Entry>> = None;
static mut ENABLED: bool = true;
const MAX_ENTRIES: usize = 600;

fn buf() -> &'static mut Vec<Entry> {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(LOG);
        slot.get_or_insert_with(Vec::new)
    }
}

/// Enregistre un evenement de la pile navigateur (+ echo serie COM1).
pub fn log(cat: Cat, msg: String) {
    if unsafe { !ENABLED } { return; }
    let tick = crate::kernel::timer::ticks();
    crate::serial_println!("[nautile/{}] {}", cat.tag(), msg);
    let v = buf();
    if v.len() >= MAX_ENTRIES { v.remove(0); }
    v.push(Entry { tick, cat, msg });
}

/// Active/desactive la journalisation.
pub fn set_enabled(on: bool) { unsafe { ENABLED = on; } }
pub fn enabled() -> bool { unsafe { ENABLED } }

/// Vide le journal.
pub fn clear() { buf().clear(); }

/// Nombre d'evenements journalises.
pub fn count() -> usize { buf().len() }

/// Compteur leger (erreurs, avert., requetes, hits cache) sur un journal.
fn stats_of(v: &[Entry]) -> (usize, usize, usize, usize) {
    let mut errs = 0; let mut warns = 0; let mut net = 0; let mut cache = 0;
    for e in v.iter() {
        match e.cat { Cat::Err => errs += 1, Cat::Warn => warns += 1,
            Cat::Net => net += 1, Cat::Cache => cache += 1, _ => {} }
    }
    (errs, warns, net, cache)
}

/// Macro ergonomique : `dlog!(Cat::Net, "GET {} -> {}", url, status)`.
#[macro_export]
macro_rules! dlog {
    ($cat:expr, $($arg:tt)*) => {
        $crate::diag::log($cat, alloc::format!($($arg)*))
    };
}

// ── Rendu HTML pour `about:log` ───────────────────────────────────────────────

/// Construit la page HTML du journal (consultee via `about:log`).
pub fn render_html() -> String {
    use alloc::format;
    let v = buf();
    let (errs, warns, net, cache) = stats_of(v);
    let mut rows = String::new();
    // Affiche les 300 dernieres entrees (plus recentes en bas).
    let start = v.len().saturating_sub(300);
    for e in &v[start..] {
        rows.push_str(&format!(
            "<tr><td class=\"t\">{tick}</td>\
             <td class=\"c\" style=\"color:{col}\">{tag}</td>\
             <td class=\"m\">{msg}</td></tr>",
            tick = e.tick, col = e.cat.color(), tag = e.cat.tag(),
            msg = esc(&e.msg),
        ));
    }
    format!(
        "<!doctype html><html><head><title>Nautile — Journal</title><style>\
         body{{background:#0b1220;color:#cdd9e5;font-family:monospace;margin:0;padding:0}}\
         .hd{{position:sticky;top:0;background:#10243f;padding:10px 14px;\
              border-bottom:2px solid #1e63b0}}\
         .hd h1{{margin:0;font-size:15px;color:#f5c040}}\
         .sum{{font-size:12px;color:#9fc2e8;margin-top:4px}}\
         .sum b{{color:#fff}} .err{{color:#e0483a}} .warn{{color:#e0a030}}\
         table{{border-collapse:collapse;width:100%;font-size:12px}}\
         td{{padding:2px 8px;border-bottom:1px solid #16243a;vertical-align:top}}\
         td.t{{color:#5f7da0;text-align:right;width:64px}}\
         td.c{{font-weight:bold;width:64px}}\
         td.m{{color:#cdd9e5;white-space:pre-wrap}}\
         </style></head><body>\
         <div class=\"hd\"><h1>&#x1f50e; Journal de la pile Nautile</h1>\
         <div class=\"sum\">{n} evenements &bull; <span class=\"err\">{errs} erreurs</span> \
         &bull; <span class=\"warn\">{warns} avert.</span> &bull; {net} requetes \
         &bull; {cache} hits cache &bull; <a href=\"about:bouchaud\" style=\"color:#4fa8f0\">accueil</a></div></div>\
         <table><tr><td class=\"t\">tick</td><td class=\"c\">cat</td><td class=\"m\">message</td></tr>\
         {rows}</table></body></html>",
        n = v.len(), errs = errs, warns = warns, net = net, cache = cache, rows = rows,
    )
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c { '&' => o.push_str("&amp;"), '<' => o.push_str("&lt;"),
                  '>' => o.push_str("&gt;"), _ => o.push(c) }
    }
    o
}
