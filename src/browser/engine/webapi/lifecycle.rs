//! Cycle de vie document + ordonnanceur de ressources.
//!
//! Cette couche est volontairement petite et no_std-friendly : elle ne rend pas
//! la page elle-meme, elle porte le contrat navigateur qui manquait entre
//! HTML/JS/CSS/layout. Un document passe par Loading -> Interactive -> Complete,
//! les ressources critiques sont classees, et les mutations marquent le pipeline
//! style/layout/paint comme sale.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReadyState { Loading, Interactive, Complete }

impl ReadyState {
    pub fn as_str(self) -> &'static str {
        match self { ReadyState::Loading => "loading", ReadyState::Interactive => "interactive", ReadyState::Complete => "complete" }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScriptMode { Classic, Module, Json, Other }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScriptTiming { ParserBlocking, Defer, Async }

#[derive(Clone)]
pub struct ScriptResource {
    pub src: Option<String>,
    pub typ: String,
    pub mode: ScriptMode,
    pub timing: ScriptTiming,
    pub nonce: Option<String>,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Clone)]
pub struct StyleResource { pub href: String, pub blocking: bool }
#[derive(Clone)]
pub struct PreloadResource { pub href: String, pub as_attr: String, pub module: bool }

#[derive(Clone, Default)]
pub struct ResourcePlan {
    pub styles: Vec<StyleResource>,
    pub scripts: Vec<ScriptResource>,
    pub preloads: Vec<PreloadResource>,
}

impl ResourcePlan {
    pub fn blocking_styles(&self) -> usize { self.styles.iter().filter(|s| s.blocking).count() }
    pub fn defer_scripts(&self) -> usize { self.scripts.iter().filter(|s| s.timing == ScriptTiming::Defer).count() }
    pub fn module_scripts(&self) -> usize { self.scripts.iter().filter(|s| s.mode == ScriptMode::Module).count() }
}

#[derive(Clone)]
pub struct DocumentLifecycle {
    pub ready_state: ReadyState,
    pub dom_content_loaded_fired: bool,
    pub load_fired: bool,
    pub style_dirty: bool,
    pub layout_dirty: bool,
    pub paint_dirty: bool,
    pub resources: ResourcePlan,
}

impl Default for DocumentLifecycle {
    fn default() -> Self { Self::new() }
}

impl DocumentLifecycle {
    pub fn new() -> DocumentLifecycle {
        DocumentLifecycle {
            ready_state: ReadyState::Loading,
            dom_content_loaded_fired: false,
            load_fired: false,
            style_dirty: true,
            layout_dirty: true,
            paint_dirty: true,
            resources: ResourcePlan::default(),
        }
    }
    pub fn reset_for_navigation(&mut self, html: &[u8]) {
        self.ready_state = ReadyState::Loading;
        self.dom_content_loaded_fired = false;
        self.load_fired = false;
        self.style_dirty = true;
        self.layout_dirty = true;
        self.paint_dirty = true;
        self.resources = scan_resources(html);
    }
    pub fn mark_style_dirty(&mut self) { self.style_dirty = true; self.layout_dirty = true; self.paint_dirty = true; }
    pub fn mark_layout_dirty(&mut self) { self.layout_dirty = true; self.paint_dirty = true; }
    pub fn mark_paint_dirty(&mut self) { self.paint_dirty = true; }
    pub fn mark_interactive(&mut self) { self.ready_state = ReadyState::Interactive; }
    pub fn mark_complete(&mut self) { self.ready_state = ReadyState::Complete; self.dom_content_loaded_fired = true; self.load_fired = true; }
    pub fn clear_render_dirty(&mut self) { self.style_dirty = false; self.layout_dirty = false; self.paint_dirty = false; }
}

pub fn scan_resources(html: &[u8]) -> ResourcePlan {
    let mut plan = ResourcePlan::default();
    let mut i = 0usize;
    while i < html.len() {
        if html[i] == b'<' {
            if starts_ci(&html[i..], b"<link") {
                let end = find_ci(html, b">", i).map(|p| p + 1).unwrap_or(html.len());
                let header = core::str::from_utf8(&html[i..end]).unwrap_or("");
                let rel = attr(header, "rel").unwrap_or_default().to_ascii_lowercase();
                let as_attr = attr(header, "as").unwrap_or_default().to_ascii_lowercase();
                if let Some(href) = attr(header, "href") {
                    if rel.split_whitespace().any(|r| r == "stylesheet") || as_attr == "style" {
                        plan.styles.push(StyleResource { href, blocking: !rel.contains("preload") });
                    } else if rel.contains("preload") || rel.contains("modulepreload") || rel.contains("preconnect") {
                        plan.preloads.push(PreloadResource { href, as_attr, module: rel.contains("modulepreload") });
                    }
                }
                i = end; continue;
            }
            if starts_ci(&html[i..], b"<script") {
                let header_end = find_ci(html, b">", i).map(|p| p + 1).unwrap_or(html.len());
                let header = core::str::from_utf8(&html[i..header_end]).unwrap_or("");
                let typ = attr(header, "type").unwrap_or_default().to_ascii_lowercase();
                let mode = if typ.is_empty() || typ.contains("javascript") || typ == "text/babel" { ScriptMode::Classic }
                    else if typ == "module" { ScriptMode::Module }
                    else if typ.contains("json") { ScriptMode::Json }
                    else { ScriptMode::Other };
                let timing = if header.to_ascii_lowercase().contains("async") { ScriptTiming::Async }
                    else if header.to_ascii_lowercase().contains("defer") || mode == ScriptMode::Module { ScriptTiming::Defer }
                    else { ScriptTiming::ParserBlocking };
                let content_end = find_ci(html, b"</script", header_end).unwrap_or(header_end);
                let outer_end = find_ci(html, b">", content_end).map(|p| p + 1).unwrap_or(content_end);
                plan.scripts.push(ScriptResource {
                    src: attr(header, "src"), typ, mode, timing,
                    nonce: attr(header, "nonce"), byte_start: i, byte_end: outer_end,
                });
                i = outer_end; continue;
            }
        }
        i += 1;
    }
    plan
}

fn lc(b: u8) -> u8 { b.to_ascii_lowercase() }
fn starts_ci(hay: &[u8], needle: &[u8]) -> bool { hay.len() >= needle.len() && hay[..needle.len()].iter().zip(needle).all(|(a,b)| lc(*a) == lc(*b)) }
fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() { return None; }
    let mut i = from;
    while i + needle.len() <= hay.len() {
        let mut k = 0; while k < needle.len() && lc(hay[i+k]) == lc(needle[k]) { k += 1; }
        if k == needle.len() { return Some(i); }
        i += 1;
    }
    None
}

fn attr(header: &str, name: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let key = alloc::format!("{}=", name);
    let pos = lower.find(&key)? + key.len();
    let rest = header[pos..].trim_start();
    match rest.as_bytes().first() {
        Some(b'"') => rest[1..].split('"').next().filter(|s| !s.is_empty()).map(|s| s.to_string()),
        Some(b'\'') => rest[1..].split('\'').next().filter(|s| !s.is_empty()).map(|s| s.to_string()),
        _ => {
            let v = rest.split(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '>').next().unwrap_or("");
            let v = v.trim_end_matches('/');
            if v.is_empty() { None } else { Some(v.to_string()) }
        }
    }
}
