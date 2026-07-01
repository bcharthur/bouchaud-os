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
            let mut val = part[c + 1..].trim();
            // Retire le marqueur `!important` : sinon la valeur ("red !important")
            // ne se parse plus. La priorite de cascade reste approchee par la
            // specificite/ordre (les moteurs reels ajoutent un palier dedie).
            if let Some(bang) = val.rfind('!') {
                if val[bang + 1..].trim_start().to_ascii_lowercase().starts_with("important") {
                    val = val[..bang].trim_end();
                }
            }
            if !prop.is_empty() && !val.is_empty() { v.push((prop, val.to_string())); }
        }
    }
    v
}

fn is_name(c: u8) -> bool { c.is_ascii_alphanumeric() || c == b'-' || c == b'_' }

// Parse un composant simple compose : `div`, `.x`, `#y`, `a.b.c`, `input#q.big`.
// Les pseudo-classes/elements (`:hover`, `::before`) et selecteurs d'attribut
// (`[type=text]`) sont reconnus et ignores pour le matching mais consommes
// proprement (ils ne cassent plus l'analyse du reste du composant).
fn parse_simple(comp: &str) -> (Sel, u32) {
    let comp = comp.trim();
    let mut sel = Sel::any();
    let mut spec = 0u32;
    if comp.is_empty() || comp == "*" { return (sel, 0); }
    let b = comp.as_bytes();
    let mut i = 0;
    // Tag optionnel en tete.
    let s0 = i;
    while i < b.len() && is_name(b[i]) { i += 1; }
    if i > s0 { sel.tag = Some(comp[s0..i].to_ascii_lowercase()); spec += 1; }
    // Suffixes .classe / #id / :pseudo / [attr].
    while i < b.len() {
        match b[i] {
            b'.' => {
                i += 1; let s = i;
                while i < b.len() && is_name(b[i]) { i += 1; }
                if i > s { sel.classes.push(comp[s..i].to_string()); spec += 10; }
            }
            b'#' => {
                i += 1; let s = i;
                while i < b.len() && is_name(b[i]) { i += 1; }
                if i > s { sel.id = Some(comp[s..i].to_ascii_lowercase()); spec += 100; }
            }
            b':' => {
                // Pseudo-classe/element : saute le nom et un eventuel (...).
                i += 1; if i < b.len() && b[i] == b':' { i += 1; }
                while i < b.len() && is_name(b[i]) { i += 1; }
                if i < b.len() && b[i] == b'(' {
                    let mut depth = 0i32;
                    while i < b.len() {
                        if b[i] == b'(' { depth += 1; }
                        else if b[i] == b')' { depth -= 1; if depth == 0 { i += 1; break; } }
                        i += 1;
                    }
                }
                spec += 10; // une pseudo-classe compte comme une classe (specificite)
            }
            b'[' => {
                while i < b.len() && b[i] != b']' { i += 1; }
                if i < b.len() { i += 1; }
                spec += 10;
            }
            _ => { i += 1; }
        }
    }
    (sel, spec)
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
    if chain.is_empty() { chain.push(Sel::any()); }
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

