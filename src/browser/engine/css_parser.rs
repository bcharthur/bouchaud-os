//! Parseur CSS minimal : declarations, selecteurs et feuilles de style.
//!
//! Le style resolver (`style.rs`) contient les structures et l'index ; ce module
//! transforme le texte CSS en `Rule`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::style::{Rule, Sel, Comb, AttrOp, AttrSel, Pseudo};

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

fn unquote(s: &str) -> &str {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] { &s[1..s.len() - 1] } else { s }
}

// Parse un formule `an+b` de :nth-child : `2n+1`, `odd`, `even`, `3`, `-n+3`.
fn parse_anb(arg: &str) -> (i32, i32) {
    let s: String = arg.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    match s.as_str() {
        "odd" => return (2, 1),
        "even" => return (2, 0),
        _ => {}
    }
    if let Some(npos) = s.find('n') {
        // Coefficient a (avant 'n').
        let a_str = &s[..npos];
        let a = match a_str { "" | "+" => 1, "-" => -1, x => x.parse::<i32>().unwrap_or(0) };
        // Constante b (apres 'n', signe inclus).
        let b = s[npos + 1..].parse::<i32>().unwrap_or(0);
        (a, b)
    } else {
        (0, s.parse::<i32>().unwrap_or(0))
    }
}

// Parse un selecteur d'attribut `[name op val]` (le contenu entre crochets).
fn parse_attr(inner: &str) -> Option<AttrSel> {
    let inner = inner.trim();
    if inner.is_empty() { return None; }
    // Cherche l'operateur : ^= $= *= ~= |= puis =.
    for (pat, op) in [("^=", AttrOp::Prefix), ("$=", AttrOp::Suffix), ("*=", AttrOp::Substr), ("~=", AttrOp::Word), ("|=", AttrOp::Dash)] {
        if let Some(p) = inner.find(pat) {
            let name = inner[..p].trim().to_ascii_lowercase();
            let val = unquote(&inner[p + 2..]).to_string();
            if !name.is_empty() { return Some(AttrSel { name, op, val }); }
        }
    }
    if let Some(p) = inner.find('=') {
        let name = inner[..p].trim().to_ascii_lowercase();
        let val = unquote(&inner[p + 1..]).to_string();
        if !name.is_empty() { return Some(AttrSel { name, op: AttrOp::Eq, val }); }
    }
    Some(AttrSel { name: inner.to_ascii_lowercase(), op: AttrOp::Exists, val: String::new() })
}

// Parse un composant simple compose : `div`, `.x`, `#y`, `a.b.c`, `input[type=text]:checked`.
// Gere maintenant reellement les selecteurs d'attribut et les pseudo-classes
// structurelles (:first-child, :nth-child, :not(...)…). Les pseudos d'etat non
// decidables (:hover, :focus...) sont neutres (n'empechent pas le match).
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
                i += 1; if i < b.len() && b[i] == b':' { i += 1; } // ::before -> pseudo-element (neutre)
                let ns = i;
                while i < b.len() && is_name(b[i]) { i += 1; }
                let name = comp[ns..i].to_ascii_lowercase();
                // Argument optionnel entre parentheses.
                let mut arg = "";
                if i < b.len() && b[i] == b'(' {
                    let astart = i + 1; let mut depth = 0i32;
                    while i < b.len() {
                        if b[i] == b'(' { depth += 1; }
                        else if b[i] == b')' { depth -= 1; if depth == 0 { break; } }
                        i += 1;
                    }
                    arg = &comp[astart..i.min(comp.len())];
                    if i < b.len() { i += 1; } // consomme ')'
                }
                match name.as_str() {
                    "first-child" => sel.pseudos.push(Pseudo::FirstChild),
                    "last-child" => sel.pseudos.push(Pseudo::LastChild),
                    "only-child" => sel.pseudos.push(Pseudo::OnlyChild),
                    "empty" => sel.pseudos.push(Pseudo::Empty),
                    "root" => sel.pseudos.push(Pseudo::Root),
                    "nth-child" => { let (a, bb) = parse_anb(arg); sel.pseudos.push(Pseudo::NthChild(a, bb)); }
                    "nth-last-child" => { let (a, bb) = parse_anb(arg); sel.pseudos.push(Pseudo::NthLastChild(a, bb)); }
                    "not" => {
                        let mut inner = Vec::new();
                        for part in arg.split(',') { let (s, _) = parse_simple(part); if !s.is_any() { inner.push(s); } }
                        if !inner.is_empty() { sel.pseudos.push(Pseudo::Not(inner)); }
                    }
                    // Pseudos d'etat/element non decidables : ignorees (neutres).
                    _ => {}
                }
                spec += 10;
            }
            b'[' => {
                let s = i + 1;
                while i < b.len() && b[i] != b']' { i += 1; }
                let inner = &comp[s..i.min(comp.len())];
                if i < b.len() { i += 1; } // consomme ']'
                if let Some(a) = parse_attr(inner) { sel.attrs.push(a); spec += 10; }
            }
            _ => { i += 1; }
        }
    }
    (sel, spec)
}

// Parse un selecteur complet en chaine ancetre→cible. Le combinateur `>` est
// Parse un selecteur complet en chaine ancetre→cible, en conservant les vrais
// combinateurs (descendant, enfant `>`, frere adjacent `+`, frere general `~`).
// Chaque compound porte le combinateur qui le relie a celui de sa gauche. Le
// decoupage respecte la profondeur `[...]`/`(...)` pour ne pas confondre les
// espaces internes (`[data-x = y]`, `:not(a b)`) avec des combinateurs.
// Limite a 4 compounds pour borner le cout du matching.
fn parse_selector(s: &str) -> (Vec<Sel>, u32) {
    let s = s.trim();
    let b = s.as_bytes();
    let mut chain: Vec<Sel> = Vec::new();
    let mut spec = 0u32;
    let mut comb = Comb::Descendant; // pour le prochain compound
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_ascii_whitespace() { i += 1; }
        if i >= b.len() { break; }
        match b[i] {
            b'>' => { comb = Comb::Child; i += 1; continue; }
            b'+' => { comb = Comb::Adjacent; i += 1; continue; }
            b'~' => { comb = Comb::General; i += 1; continue; }
            _ => {}
        }
        // Lit un compound en respectant les niveaux [] et ().
        let start = i; let mut depth = 0i32;
        while i < b.len() {
            let c = b[i];
            if c == b'[' || c == b'(' { depth += 1; }
            else if c == b']' || c == b')' { if depth > 0 { depth -= 1; } }
            else if depth == 0 && ((c as char).is_ascii_whitespace() || c == b'>' || c == b'+' || c == b'~') { break; }
            i += 1;
        }
        let (mut sel, sp) = parse_simple(&s[start..i]);
        sel.comb = comb;
        spec += sp;
        chain.push(sel);
        comb = Comb::Descendant;
    }
    if chain.is_empty() { chain.push(Sel::any()); }
    // Ne garde que les 4 derniers compounds (cible + 3 ancetres) pour le cout.
    if chain.len() > 4 { let start = chain.len() - 4; chain.drain(0..start); chain[0].comb = Comb::Descendant; }
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

