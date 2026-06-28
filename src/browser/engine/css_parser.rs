//! Parseur CSS minimal : declarations, selecteurs et feuilles de style.
//!
//! Le style resolver (`style.rs`) contient les structures et l'index ; ce module
//! transforme le texte CSS en `Rule`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::style::{Rule, Sel};

fn lc(c: u8) -> u8 { if c >= b'A' && c <= b'Z' { c + 32 } else { c } }

fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() || from >= hay.len() { return None; }
    let last = hay.len().saturating_sub(needle.len());
    let mut i = from;
    while i <= last {
        if hay[i..i+needle.len()].iter().zip(needle).all(|(a, b)| lc(*a) == lc(*b)) { return Some(i); }
        i += 1;
    }
    None
}

pub(super) fn parse_decls(body: &str) -> Vec<(String, String)> {
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

// Parse un selecteur simple (un seul composant : `div`, `.x`, `#y`, `a.b`).
fn parse_simple(comp: &str) -> (Sel, u32) {
    let last = comp.trim();
    if last == "*" || last.is_empty() { return (Sel::Any, 0); }
    if let Some(id) = last.strip_prefix('#') { return (Sel::Id(id.to_ascii_lowercase()), 100); }
    if let Some(cl) = last.strip_prefix('.') {
        // `.a.b` -> garde la premiere classe (matching simple).
        let cl = cl.split('.').next().unwrap_or(cl);
        let cl = cl.split(|c: char| c == ':' || c == '[').next().unwrap_or(cl);
        return (Sel::Class(cl.to_string()), 10);
    }
    if let Some(dot) = last.find('.') {
        let tag = last[..dot].to_ascii_lowercase();
        let cl = last[dot + 1..].split('.').next().unwrap_or(&last[dot + 1..]);
        let cl = cl.split(|c: char| c == ':' || c == '[').next().unwrap_or(cl);
        return (Sel::TagClass(tag, cl.to_string()), 11);
    }
    // pseudo/attr non geres -> match par balise si alphanumerique
    let tag: String = last.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>().to_ascii_lowercase();
    if tag.is_empty() { (Sel::Any, 0) } else { (Sel::Tag(tag), 1) }
}

// Parse un selecteur complet en chaine ancetre→cible. Le combinateur `>` est
// traite comme un descendant (approximation), les pseudo-elements ignores.
// Limite la profondeur a 4 composants pour borner le cout du matching.
fn parse_selector(s: &str) -> (Vec<Sel>, u32) {
    let s = s.trim();
    let mut chain: Vec<Sel> = Vec::new();
    let mut spec = 0u32;
    for comp in s.split(|c: char| c == ' ' || c == '>' || c == '+' || c == '~').filter(|x| !x.is_empty()) {
        let (sel, sp) = parse_simple(comp);
        chain.push(sel);
        spec += sp;
    }
    if chain.is_empty() { chain.push(Sel::Any); }
    // Ne garde que les 4 derniers composants (cible + 3 ancetres) pour le cout.
    if chain.len() > 4 { let start = chain.len() - 4; chain.drain(0..start); }
    (chain, spec)
}

pub(super) fn parse_stylesheet(text: &str, out: &mut Vec<Rule>) {
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
            let (chain, spec) = parse_selector(sel);
            out.push(Rule { chain, decls: decls.clone(), spec });
        }
        if out.len() > 4000 { break; }
    }
}

