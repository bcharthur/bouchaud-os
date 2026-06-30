//! Lexer JavaScript : transforme la source en suite de jetons `(Tok, nl)` où
//! `nl` indique qu'un saut de ligne précède le jeton (pour l'insertion
//! automatique de point-virgule côté parseur). Extrait de l'ancien `js.rs`
//! monolithique — première étape de fragmentation du moteur JS.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::hexv;

#[derive(Clone, PartialEq)]
pub(super) enum Tok { Num(f64), Str(String), Tpl(Vec<TplPart>), Ident(String), Keyword(String), Punct(String), Regex, Eof }
#[derive(Clone, PartialEq)]
pub(super) enum TplPart { Str(String), Expr(Vec<Tok>) }

pub(super) struct Lexer<'a> { s: &'a [u8], i: usize, toks: Vec<(Tok, bool)> }

const KEYWORDS: &[&str] = &["var","let","const","function","return","if","else","for","while","do",
    "break","continue","new","typeof","instanceof","in","of","this","null","true","false",
    "undefined","void","delete","throw","try","catch","finally","switch","case","default","class","extends","super",
    "async","await","yield","static","get","set","from"];

fn is_id_start(c: u8) -> bool { c == b'_' || c == b'$' || c.is_ascii_alphabetic() }
fn is_id_part(c: u8) -> bool { c == b'_' || c == b'$' || c.is_ascii_alphanumeric() }

impl<'a> Lexer<'a> {
    pub(super) fn new(s: &'a [u8]) -> Lexer<'a> {
        // Les gros bundles modernes (Google sert facilement >1 Mio de JS)
        // declenchaient une croissance geometrique de Vec pendant le lexing :
        // quand le tampon approchait ~10 Mio, le realloc suivant demandait
        // ~20 Mio contigus et pouvait OOM sur le tas noyau de 48 Mio. On
        // pre-dimensionne d'apres la taille source pour eviter ce pic de
        // double-buffer tout en gardant le moteur JS actif.
        let cap = (s.len() / 4).clamp(256, 300_000);
        Lexer { s, i: 0, toks: Vec::with_capacity(cap) }
    }

    fn prev_allows_regex(&self) -> bool {
        match self.toks.last() {
            None => true,
            Some((t, _)) => match t {
                Tok::Num(_) | Tok::Str(_) | Tok::Tpl(_) | Tok::Regex => false,
                Tok::Ident(_) => false,
                Tok::Keyword(k) => k != "this",
                Tok::Punct(p) => p != ")" && p != "]" && p != "}",
                Tok::Eof => true,
            },
        }
    }

    pub(super) fn lex_all(mut self) -> Result<Vec<(Tok, bool)>, String> {
        let mut nl = false;
        loop {
            loop {
                while self.i < self.s.len() && (self.s[self.i] == b' ' || self.s[self.i] == b'\t' || self.s[self.i] == b'\r' || self.s[self.i] == b'\n') {
                    if self.s[self.i] == b'\n' { nl = true; }
                    self.i += 1;
                }
                if self.i + 1 < self.s.len() && self.s[self.i] == b'/' && self.s[self.i + 1] == b'/' {
                    while self.i < self.s.len() && self.s[self.i] != b'\n' { self.i += 1; }
                    continue;
                }
                if self.i + 1 < self.s.len() && self.s[self.i] == b'/' && self.s[self.i + 1] == b'*' {
                    self.i += 2;
                    while self.i + 1 < self.s.len() && !(self.s[self.i] == b'*' && self.s[self.i + 1] == b'/') { if self.s[self.i] == b'\n' { nl = true; } self.i += 1; }
                    self.i += 2;
                    continue;
                }
                break;
            }
            if self.i >= self.s.len() { self.toks.push((Tok::Eof, nl)); break; }
            let c = self.s[self.i];
            let tok = if c.is_ascii_digit() || (c == b'.' && self.i + 1 < self.s.len() && self.s[self.i + 1].is_ascii_digit()) {
                self.lex_number()
            } else if c == b'"' || c == b'\'' { self.lex_string(c)? }
            else if c == b'`' { self.lex_template()? }
            else if is_id_start(c) { self.lex_ident() }
            else if c == b'/' && self.prev_allows_regex() { self.lex_regex() }
            else { self.lex_punct()? };
            self.toks.push((tok, nl));
            nl = false;
            if self.toks.len() > 1_000_000 { return Err("script trop long".into()); }
        }
        Ok(self.toks)
    }

    fn lex_number(&mut self) -> Tok {
        let start = self.i;
        if self.s[self.i] == b'0' && self.i + 1 < self.s.len() && (self.s[self.i + 1] | 32 == b'x') {
            self.i += 2; let hs = self.i;
            while self.i < self.s.len() && self.s[self.i].is_ascii_hexdigit() { self.i += 1; }
            let v = u64::from_str_radix(core::str::from_utf8(&self.s[hs..self.i]).unwrap_or("0"), 16).unwrap_or(0);
            return Tok::Num(v as f64);
        }
        let (mut seen_dot, mut seen_e) = (false, false);
        while self.i < self.s.len() {
            let ch = self.s[self.i];
            if ch.is_ascii_digit() { self.i += 1; }
            else if ch == b'.' && !seen_dot && !seen_e { seen_dot = true; self.i += 1; } // un seul point : `10..x` -> Num(10.) puis .x
            else if (ch | 32) == b'e' && !seen_e { seen_e = true; self.i += 1; }
            else if (ch == b'+' || ch == b'-') && self.i > start && (self.s[self.i - 1] | 32) == b'e' { self.i += 1; }
            else { break; }
        }
        Tok::Num(core::str::from_utf8(&self.s[start..self.i]).unwrap_or("0").parse::<f64>().unwrap_or(f64::NAN))
    }

    // Decode un caractere UTF-8 a la position courante (les pages web non-ASCII,
    // ex. accents francais, doivent etre lues correctement et non octet par octet).
    fn take_char(&mut self) -> char {
        let b0 = self.s[self.i];
        if b0 < 0x80 { self.i += 1; return b0 as char; }
        let len = if b0 >= 0xF0 { 4 } else if b0 >= 0xE0 { 3 } else { 2 };
        let end = (self.i + len).min(self.s.len());
        match core::str::from_utf8(&self.s[self.i..end]).ok().and_then(|s| s.chars().next()) {
            Some(c) => { self.i += c.len_utf8(); c }
            None => { self.i += 1; '\u{FFFD}' }
        }
    }

    fn lex_string(&mut self, q: u8) -> Result<Tok, String> {
        self.i += 1; let mut out = String::new();
        while self.i < self.s.len() && self.s[self.i] != q {
            if self.s[self.i] == b'\\' { self.i += 1; if self.i >= self.s.len() { break; } let ch = self.escape(); if ch != '\0' { out.push(ch); } }
            else { out.push(self.take_char()); }
        }
        self.i += 1;
        Ok(Tok::Str(out))
    }

    fn escape(&mut self) -> char {
        let e = self.s[self.i]; self.i += 1;
        match e {
            b'n' => '\n', b't' => '\t', b'r' => '\r', b'b' => '\u{8}', b'f' => '\u{c}', b'0' => '\0',
            b'\\' => '\\', b'\'' => '\'', b'"' => '"', b'`' => '`', b'/' => '/',
            b'x' => char::from_u32(self.take_hex(2)).unwrap_or('?'),
            b'u' => {
                if self.i < self.s.len() && self.s[self.i] == b'{' { self.i += 1; let mut v = 0u32; while self.i < self.s.len() && self.s[self.i] != b'}' { v = v * 16 + hexv(self.s[self.i]); self.i += 1; } self.i += 1; char::from_u32(v).unwrap_or('?') }
                else { char::from_u32(self.take_hex(4)).unwrap_or('?') }
            }
            b'\n' => '\0',
            other => other as char,
        }
    }
    fn take_hex(&mut self, n: usize) -> u32 { let mut v = 0u32; for _ in 0..n { if self.i < self.s.len() && self.s[self.i].is_ascii_hexdigit() { v = v * 16 + hexv(self.s[self.i]); self.i += 1; } } v }

    fn lex_template(&mut self) -> Result<Tok, String> {
        self.i += 1; let mut parts: Vec<TplPart> = Vec::new(); let mut cur = String::new();
        while self.i < self.s.len() && self.s[self.i] != b'`' {
            let c = self.s[self.i];
            if c == b'\\' { self.i += 1; if self.i < self.s.len() { let ch = self.escape(); if ch != '\0' { cur.push(ch); } } continue; }
            if c == b'$' && self.i + 1 < self.s.len() && self.s[self.i + 1] == b'{' {
                parts.push(TplPart::Str(core::mem::take(&mut cur)));
                self.i += 2; let mut depth = 1; let estart = self.i;
                while self.i < self.s.len() && depth > 0 { match self.s[self.i] { b'{' => depth += 1, b'}' => depth -= 1, _ => {} } if depth == 0 { break; } self.i += 1; }
                let sub = &self.s[estart..self.i]; self.i += 1;
                let subtoks = Lexer::new(sub).lex_all()?;
                parts.push(TplPart::Expr(subtoks.into_iter().map(|(t, _)| t).collect()));
            } else { cur.push(self.take_char()); }
        }
        self.i += 1; parts.push(TplPart::Str(cur));
        Ok(Tok::Tpl(parts))
    }

    fn lex_ident(&mut self) -> Tok {
        let start = self.i; while self.i < self.s.len() && is_id_part(self.s[self.i]) { self.i += 1; }
        let w = core::str::from_utf8(&self.s[start..self.i]).unwrap_or("").to_string();
        if KEYWORDS.contains(&w.as_str()) { Tok::Keyword(w) } else { Tok::Ident(w) }
    }

    fn lex_regex(&mut self) -> Tok {
        self.i += 1; let mut in_class = false;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == b'\\' { self.i += 2; continue; }
            if c == b'[' { in_class = true; } else if c == b']' { in_class = false; }
            else if c == b'/' && !in_class { self.i += 1; break; } else if c == b'\n' { break; }
            self.i += 1;
        }
        while self.i < self.s.len() && is_id_part(self.s[self.i]) { self.i += 1; }
        Tok::Regex
    }

    fn lex_punct(&mut self) -> Result<Tok, String> {
        let four: &[&str] = &[">>>="];
        let three: &[&str] = &["===","!==","**=","...","<<=",">>=","&&=","||=","??=",">>>"];
        let two: &[&str] = &["==","!=","<=",">=","&&","||","=>","+=","-=","*=","/=","%=","**","++","--","<<",">>","?.","??","&=","|=","^="];
        for p in four { if self.s[self.i..].starts_with(p.as_bytes()) { self.i += 4; return Ok(Tok::Punct((*p).to_string())); } }
        for p in three { if self.s[self.i..].starts_with(p.as_bytes()) { self.i += 3; return Ok(Tok::Punct((*p).to_string())); } }
        for p in two { if self.s[self.i..].starts_with(p.as_bytes()) { self.i += 2; return Ok(Tok::Punct((*p).to_string())); } }
        let c = self.s[self.i]; self.i += 1; Ok(Tok::Punct((c as char).to_string()))
    }
}
