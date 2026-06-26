//! Interpréteur Rust minimal embarqué dans Bouchaud OS.
//!
//! Supporte : fn, let/let mut, if/else, while, loop, for..in 0..n,
//! println!/print!/format!/assert!, opérateurs arithmétiques et logiques,
//! méthodes de base sur str et i64, appels de fonctions utilisateur.
//! Usage : tester des snippets Rust directement dans l'OS sans compilateur.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::boxed::Box;

fn isqrt(n: i64) -> i64 {
    if n <= 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}

// ─── Valeurs ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum Val {
    Int(i64),
    Bool(bool),
    Str(String),
    Range(i64, i64),
    Unit,
}

impl Val {
    fn truthy(&self) -> bool {
        match self {
            Val::Bool(b) => *b,
            Val::Int(n) => *n != 0,
            Val::Str(s) => !s.is_empty(),
            Val::Unit | Val::Range(_, _) => false,
        }
    }
    fn as_int(&self) -> i64 {
        match self { Val::Int(n) => *n, Val::Bool(b) => *b as i64, _ => 0 }
    }
    pub fn display(&self) -> String {
        match self {
            Val::Int(n)    => format!("{}", n),
            Val::Bool(b)   => if *b { "true".into() } else { "false".into() },
            Val::Str(s)    => s.clone(),
            Val::Range(a, b) => format!("{}..{}", a, b),
            Val::Unit      => String::new(),
        }
    }
}

// ─── Tokens ───────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum T {
    // Littéraux
    Int(i64), Str(String), Bool(bool),
    // Identifiant et macro (avec !)
    Id(String), Mac(String),
    // Opérateurs composés
    Eq2, Ne, Le, Ge, And, Or, Arrow, DotDot, PlusEq, MinusEq, MulEq, DivEq,
    // Opérateur simple (octet ASCII)
    Op(u8),
    // Ponctuation
    P(u8),
    // Mots-clés
    KFn, KLet, KMut, KIf, KElse, KWhile, KFor, KIn, KReturn,
    KBreak, KContinue, KLoop, KPub, KUse, KStruct, KImpl, KMod,
    Eof,
}

fn lex(src: &str) -> Vec<T> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut out: Vec<T> = Vec::new();
    while i < b.len() {
        // Espace
        if b[i] <= b' ' { i += 1; continue; }
        // Commentaire //
        if i + 1 < b.len() && b[i] == b'/' && b[i+1] == b'/' {
            while i < b.len() && b[i] != b'\n' { i += 1; }
            continue;
        }
        // Commentaire /* */
        if i + 1 < b.len() && b[i] == b'/' && b[i+1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i+1] == b'/') { i += 1; }
            i += 2; continue;
        }
        // Entier / flottant (converti en entier)
        if b[i].is_ascii_digit() {
            let s = i;
            while i < b.len() && b[i].is_ascii_digit() { i += 1; }
            let has_dot = i < b.len() && b[i] == b'.' && i+1 < b.len() && b[i+1].is_ascii_digit();
            if has_dot { i += 1; while i < b.len() && b[i].is_ascii_digit() { i += 1; } }
            // Skip type suffix (u32, i64, usize, f64...)
            if i < b.len() && (b[i] == b'u' || b[i] == b'i' || b[i] == b'f') {
                while i < b.len() && b[i].is_ascii_alphanumeric() { i += 1; }
            }
            let w = unsafe { core::str::from_utf8_unchecked(&b[s..i]) };
            let v: i64 = if has_dot { w.split('.').next().and_then(|p| p.parse().ok()).unwrap_or(0) }
                         else { w.parse().unwrap_or(0) };
            out.push(T::Int(v)); continue;
        }
        // Chaîne littérale
        if b[i] == b'"' {
            i += 1; let mut s = String::new();
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' && i+1 < b.len() {
                    i += 1;
                    match b[i] {
                        b'n' => s.push('\n'), b't' => s.push('\t'), b'r' => s.push('\r'),
                        b'"' => s.push('"'),  b'\\' => s.push('\\'), b'0' => s.push('\0'),
                        c    => { s.push('\\'); s.push(c as char); }
                    }
                } else { s.push(b[i] as char); }
                i += 1;
            }
            if i < b.len() { i += 1; }
            out.push(T::Str(s)); continue;
        }
        // Caractère littéral → entier
        if b[i] == b'\'' && i+2 < b.len() && b[i+2] == b'\'' {
            let v = b[i+1] as i64; i += 3; out.push(T::Int(v)); continue;
        }
        // Identifiant ou mot-clé
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
            let word = unsafe { core::str::from_utf8_unchecked(&b[s..i]) };
            if i < b.len() && b[i] == b'!' { i += 1; out.push(T::Mac(word.into())); continue; }
            out.push(match word {
                "fn"     => T::KFn,   "let"      => T::KLet,   "mut"      => T::KMut,
                "if"     => T::KIf,   "else"     => T::KElse,  "while"    => T::KWhile,
                "for"    => T::KFor,  "in"       => T::KIn,    "return"   => T::KReturn,
                "break"  => T::KBreak,"continue" => T::KContinue,"loop"   => T::KLoop,
                "pub"    => T::KPub,  "use"      => T::KUse,   "struct"   => T::KStruct,
                "impl"   => T::KImpl, "mod"      => T::KMod,
                "true"   => T::Bool(true),        "false"    => T::Bool(false),
                _        => T::Id(word.into()),
            });
            continue;
        }
        let c = b[i]; i += 1;
        let nx = if i < b.len() { b[i] } else { 0 };
        macro_rules! two { ($tok:expr) => { { i += 1; out.push($tok); } } }
        match (c, nx) {
            (b'-', b'>') => two!(T::Arrow),
            (b'.', b'.') => two!(T::DotDot),
            (b'=', b'=') => two!(T::Eq2),
            (b'!', b'=') => two!(T::Ne),
            (b'<', b'=') => two!(T::Le),
            (b'>', b'=') => two!(T::Ge),
            (b'&', b'&') => two!(T::And),
            (b'|', b'|') => two!(T::Or),
            (b'+', b'=') => two!(T::PlusEq),
            (b'-', b'=') => two!(T::MinusEq),
            (b'*', b'=') => two!(T::MulEq),
            (b'/', b'=') => two!(T::DivEq),
            (b'+',_)|(b'-',_)|(b'*',_)|(b'/',_)|(b'%',_)|
            (b'<',_)|(b'>',_)|(b'!',_)|(b'&',_)|(b'|',_)|(b'^',_) => out.push(T::Op(c)),
            _ => if b"(){},;:.=[]@#~".contains(&c) { out.push(T::P(c)); }
        }
    }
    out.push(T::Eof);
    out
}

// ─── AST ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
enum E {
    Int(i64), Bool(bool), Str(String), Var(String),
    BinOp(Box<E>, &'static str, Box<E>),
    UnOp(&'static str, Box<E>),
    Call(String, Vec<E>),
    Method(Box<E>, String, Vec<E>),
    Range(Box<E>, Box<E>),
    Fmt(String, String, Vec<E>),   // macro_name, format_string, args
}

#[derive(Clone)]
enum S {
    Let(String, Option<E>),
    Assign(String, &'static str, E),  // name, op("" "+=" …), value
    Expr(E),
    If(E, Vec<S>, Option<Vec<S>>),
    While(E, Vec<S>),
    Loop(Vec<S>),
    For(String, E, Vec<S>),
    Return(Option<E>),
    Break,
    Continue,
}

#[derive(Clone)]
struct FnDef { params: Vec<String>, body: Vec<S> }

// ─── Parser ───────────────────────────────────────────────────────────────────

struct Pr { t: Vec<T>, i: usize }

impl Pr {
    fn pk(&self) -> &T { self.t.get(self.i).unwrap_or(&T::Eof) }
    fn adv(&mut self) -> T { let r = self.t.get(self.i).cloned().unwrap_or(T::Eof); if self.i < self.t.len() { self.i += 1; } r }
    fn eat(&mut self, tok: &T) -> bool { if self.pk() == tok { self.i += 1; true } else { false } }
    fn ep(&mut self, ch: u8) { self.eat(&T::P(ch)); }
    fn ep_bool(&mut self, ch: u8) -> bool { self.eat(&T::P(ch)) }
    fn eat_id(&mut self) -> Option<String> {
        if let T::Id(s) = self.pk().clone() { self.i += 1; Some(s) } else { None }
    }

    // Saute `: Type` ou `-> Type`
    fn skip_type(&mut self) {
        if self.ep_bool(b':') {
            let mut d = 0i32;
            loop {
                match self.pk() {
                    T::P(b'<') => { d += 1; self.i += 1; }
                    T::P(b'>') if d > 0 => { d -= 1; self.i += 1; }
                    T::P(b',') | T::P(b')') | T::P(b'{') | T::P(b';') | T::Arrow | T::Eof if d == 0 => break,
                    _ => { self.i += 1; }
                }
            }
        }
    }
    fn skip_ret_type(&mut self) {
        if self.eat(&T::Arrow) {
            let mut d = 0i32;
            loop {
                match self.pk() {
                    T::P(b'<') => { d += 1; self.i += 1; }
                    T::P(b'>') if d > 0 => { d -= 1; self.i += 1; }
                    T::P(b'{') | T::Eof => break,
                    _ => { self.i += 1; }
                }
            }
        }
    }

    fn parse_params(&mut self) -> Vec<String> {
        self.ep(b'(');
        let mut ps = Vec::new();
        while self.pk() != &T::P(b')') && self.pk() != &T::Eof {
            while matches!(self.pk(), T::KMut | T::Op(b'&')) { self.i += 1; }
            if let T::Id(n) = self.pk().clone() {
                self.i += 1;
                if n != "self" { ps.push(n); }
            }
            self.skip_type();
            self.ep_bool(b',');
        }
        self.ep(b')');
        ps
    }

    fn parse_block(&mut self) -> Vec<S> {
        self.ep(b'{');
        let mut body = Vec::new();
        while self.pk() != &T::P(b'}') && self.pk() != &T::Eof {
            if let Some(s) = self.parse_stmt() { body.push(s); }
        }
        self.ep(b'}');
        body
    }

    // Saute un bloc externe (struct, impl, fn, use …) jusqu'à {…} ou ;
    fn skip_item(&mut self) {
        let mut d = 0i32;
        loop {
            match self.pk() {
                T::P(b'{') => { d += 1; self.i += 1; }
                T::P(b'}') => { self.i += 1; d -= 1; if d <= 0 { break; } }
                T::P(b';') if d == 0 => { self.i += 1; break; }
                T::Eof => break,
                _ => { self.i += 1; }
            }
        }
    }

    fn parse_stmt(&mut self) -> Option<S> {
        match self.pk().clone() {
            T::KPub | T::KUse | T::KStruct | T::KImpl | T::KMod => { self.skip_item(); None }
            T::KFn => { self.skip_item(); None }
            T::KLet => {
                self.i += 1;
                if self.eat(&T::KMut) {} // consomme mut
                let nm = self.eat_id().unwrap_or_default();
                self.skip_type(); // : Type optionnel
                let init = if self.ep_bool(b'=') { Some(self.expr()) } else { None };
                self.ep(b';');
                Some(S::Let(nm, init))
            }
            T::KReturn => {
                self.i += 1;
                if self.ep_bool(b';') { Some(S::Return(None)) }
                else { let e = self.expr(); self.ep(b';'); Some(S::Return(Some(e))) }
            }
            T::KBreak    => { self.i += 1; self.ep(b';'); Some(S::Break) }
            T::KContinue => { self.i += 1; self.ep(b';'); Some(S::Continue) }
            T::KIf => {
                self.i += 1;
                let cond = self.expr();
                let then = self.parse_block();
                let else_ = if self.eat(&T::KElse) {
                    if self.pk() == &T::KIf { self.parse_stmt().map(|s| alloc::vec![s]) }
                    else { Some(self.parse_block()) }
                } else { None };
                Some(S::If(cond, then, else_))
            }
            T::KWhile => { self.i += 1; let c = self.expr(); let b = self.parse_block(); Some(S::While(c, b)) }
            T::KLoop  => { self.i += 1; let b = self.parse_block(); Some(S::Loop(b)) }
            T::KFor   => {
                self.i += 1;
                let v = self.eat_id().unwrap_or_default();
                self.eat(&T::KIn);
                let r = self.expr();
                let b = self.parse_block();
                Some(S::For(v, r, b))
            }
            T::P(b';') => { self.i += 1; None }
            T::P(b'}') | T::Eof => None,
            _ => {
                let e = self.expr();
                match self.pk() {
                    T::P(b'=') => {
                        self.i += 1;
                        if let E::Var(n) = e {
                            let v = self.expr(); self.ep(b';');
                            return Some(S::Assign(n, "", v));
                        }
                        self.ep(b';'); None
                    }
                    T::PlusEq  => { self.i += 1; if let E::Var(n) = e { let v = self.expr(); self.ep(b';'); return Some(S::Assign(n, "+=", v)); } self.ep(b';'); None }
                    T::MinusEq => { self.i += 1; if let E::Var(n) = e { let v = self.expr(); self.ep(b';'); return Some(S::Assign(n, "-=", v)); } self.ep(b';'); None }
                    T::MulEq   => { self.i += 1; if let E::Var(n) = e { let v = self.expr(); self.ep(b';'); return Some(S::Assign(n, "*=", v)); } self.ep(b';'); None }
                    T::DivEq   => { self.i += 1; if let E::Var(n) = e { let v = self.expr(); self.ep(b';'); return Some(S::Assign(n, "/=", v)); } self.ep(b';'); None }
                    _ => { self.ep(b';'); Some(S::Expr(e)) }
                }
            }
        }
    }

    fn expr(&mut self) -> E { self.or() }

    fn or(&mut self) -> E {
        let mut l = self.and();
        while self.eat(&T::Or) { let r = self.and(); l = E::BinOp(l.into(), "||", r.into()); }
        l
    }
    fn and(&mut self) -> E {
        let mut l = self.cmp();
        while self.eat(&T::And) { let r = self.cmp(); l = E::BinOp(l.into(), "&&", r.into()); }
        l
    }
    fn cmp(&mut self) -> E {
        let l = self.add();
        let op: &'static str = match self.pk() {
            T::Eq2 => "==", T::Ne => "!=", T::Le => "<=", T::Ge => ">=",
            T::Op(b'<') => "<", T::Op(b'>') => ">", _ => return l,
        };
        self.i += 1;
        let r = self.add();
        E::BinOp(l.into(), op, r.into())
    }
    fn add(&mut self) -> E {
        let mut l = self.mul();
        loop {
            let op: &'static str = match self.pk() { T::Op(b'+') => "+", T::Op(b'-') => "-", _ => break };
            self.i += 1; let r = self.mul(); l = E::BinOp(l.into(), op, r.into());
        }
        l
    }
    fn mul(&mut self) -> E {
        let mut l = self.range();
        loop {
            let op: &'static str = match self.pk() { T::Op(b'*') => "*", T::Op(b'/') => "/", T::Op(b'%') => "%", _ => break };
            self.i += 1; let r = self.range(); l = E::BinOp(l.into(), op, r.into());
        }
        l
    }
    fn range(&mut self) -> E {
        let l = self.unary();
        if self.eat(&T::DotDot) { let r = self.unary(); return E::Range(l.into(), r.into()); }
        l
    }
    fn unary(&mut self) -> E {
        match self.pk() {
            T::Op(b'!') => { self.i += 1; E::UnOp("!", self.unary().into()) }
            T::Op(b'-') => { self.i += 1; E::UnOp("-", self.unary().into()) }
            T::Op(b'&') => { self.i += 1; if self.eat(&T::KMut) {} self.unary() }
            T::Op(b'*') => { self.i += 1; self.unary() }
            _ => self.postfix(),
        }
    }
    fn postfix(&mut self) -> E {
        let mut e = self.primary();
        loop {
            match self.pk() {
                T::P(b'.') => {
                    self.i += 1;
                    // Saute ::
                    if self.pk() == &T::P(b':') && self.t.get(self.i+1) == Some(&T::P(b':')) { self.i += 2; }
                    let m = self.eat_id().unwrap_or_default();
                    if self.ep_bool(b'(') {
                        let args = self.call_args_open();
                        e = E::Method(e.into(), m, args);
                    }
                    // sinon: accès champ, ignoré
                }
                T::P(b'[') => {
                    self.i += 1;
                    let idx = self.expr();
                    self.ep(b']');
                    e = E::Method(e.into(), "__idx__".into(), alloc::vec![idx]);
                }
                // Saute `as Type`
                T::Id(s) if s == "as" => {
                    self.i += 1;
                    // Consomme le nom de type jusqu'à un opérateur ou ponctuation de fin
                    while !matches!(self.pk(),
                        T::P(b';') | T::P(b')') | T::P(b',') | T::P(b'{') | T::P(b'}') | T::P(b']') |
                        T::Op(b'+') | T::Op(b'-') | T::Op(b'*') | T::Op(b'/') | T::Op(b'%') |
                        T::Op(b'<') | T::Op(b'>') | T::Eq2 | T::Ne | T::Le | T::Ge |
                        T::And | T::Or | T::DotDot | T::Eof) { self.i += 1; }
                }
                _ => break,
            }
        }
        e
    }
    fn call_args(&mut self) -> Vec<E> {
        self.ep(b'(');
        self.call_args_open()
    }
    fn call_args_open(&mut self) -> Vec<E> {
        let mut args = Vec::new();
        while self.pk() != &T::P(b')') && self.pk() != &T::Eof {
            args.push(self.expr());
            self.ep_bool(b',');
        }
        self.ep(b')');
        args
    }

    fn primary(&mut self) -> E {
        match self.adv() {
            T::Int(n)  => E::Int(n),
            T::Bool(b) => E::Bool(b),
            T::Str(s)  => E::Str(s),
            T::Mac(name) => {
                self.ep(b'(');
                let (fmt, args) = self.macro_args(&name);
                self.ep(b')');
                E::Fmt(name, fmt, args)
            }
            T::Id(name) => {
                // Chemin Nom::assoc
                if self.pk() == &T::P(b':') && self.t.get(self.i+1) == Some(&T::P(b':')) {
                    self.i += 2;
                    let _assoc = self.eat_id();
                    if self.pk() == &T::P(b'(') {
                        let _ = self.call_args();
                        return E::Fmt("__unit__".into(), String::new(), Vec::new());
                    }
                    return E::Var(name);
                }
                if self.pk() == &T::P(b'(') { E::Call(name, self.call_args()) }
                else { E::Var(name) }
            }
            T::P(b'(') => { let e = self.expr(); self.ep(b')'); e }
            T::P(b'{') => {
                // Expression bloc : exécute et renvoie la dernière valeur
                while self.pk() != &T::P(b'}') && self.pk() != &T::Eof { self.i += 1; }
                self.ep(b'}');
                E::Fmt("__unit__".into(), String::new(), Vec::new())
            }
            T::P(b'[') => {
                // Tableau littéral
                let mut elems = Vec::new();
                while self.pk() != &T::P(b']') && self.pk() != &T::Eof {
                    elems.push(self.expr()); self.ep_bool(b',');
                }
                self.ep(b']');
                if elems.is_empty() { E::Str("[]".into()) } else { elems.remove(0) }
            }
            _ => E::Int(0),
        }
    }

    fn macro_args(&mut self, name: &str) -> (String, Vec<E>) {
        match name {
            "assert" | "assert_eq" | "assert_ne" | "panic" | "todo" |
            "unimplemented" | "eprintln" | "dbg" => {
                let mut args = Vec::new();
                while self.pk() != &T::P(b')') && self.pk() != &T::Eof {
                    args.push(self.expr()); self.ep_bool(b',');
                }
                (String::new(), args)
            }
            _ => {
                if self.pk() == &T::P(b')') { return (String::new(), Vec::new()); }
                // Premier argument = chaîne de format
                let fmt = if let T::Str(s) = self.pk().clone() { self.i += 1; s }
                          else { return (String::new(), alloc::vec![self.expr()]); };
                let mut args = Vec::new();
                while self.ep_bool(b',') && self.pk() != &T::P(b')') && self.pk() != &T::Eof {
                    args.push(self.expr());
                }
                (fmt, args)
            }
        }
    }
}

// ─── Flux de contrôle ────────────────────────────────────────────────────────

enum Flow { Ok(Val), Ret(Val), Brk, Cnt, Err(String) }

// ─── Contexte d'exécution ────────────────────────────────────────────────────

struct Ctx<'a> {
    fns:    &'a BTreeMap<String, FnDef>,
    scopes: Vec<BTreeMap<String, Val>>,
    out:    &'a mut String,
    steps:  u32,
}

const STEP_LIMIT: u32 = 500_000;

impl<'a> Ctx<'a> {
    fn get(&self, name: &str) -> Val {
        for sc in self.scopes.iter().rev() {
            if let Some(v) = sc.get(name) { return v.clone(); }
        }
        Val::Unit
    }
    fn set(&mut self, name: &str, v: Val) {
        for sc in self.scopes.iter_mut().rev() {
            if sc.contains_key(name) { sc.insert(name.into(), v); return; }
        }
        if let Some(sc) = self.scopes.last_mut() { sc.insert(name.into(), v); }
    }
    fn def(&mut self, name: &str, v: Val) {
        if let Some(sc) = self.scopes.last_mut() { sc.insert(name.into(), v); }
    }
    fn push(&mut self) { self.scopes.push(BTreeMap::new()); }
    fn pop(&mut self)  { self.scopes.pop(); }

    fn step(&mut self) -> bool { self.steps += 1; self.steps < STEP_LIMIT }

    fn run(&mut self, stmts: &[S]) -> Flow {
        for s in stmts {
            match self.stmt(s) { Flow::Ok(_) => {} f => return f }
        }
        Flow::Ok(Val::Unit)
    }

    fn stmt(&mut self, s: &S) -> Flow {
        if !self.step() { return Flow::Err("timeout (100k steps)".into()); }
        match s {
            S::Let(name, init) => {
                let v = if let Some(e) = init { match self.eval(e) { Flow::Ok(v) => v, f => return f } }
                        else { Val::Unit };
                self.def(name, v);
            }
            S::Assign(name, op, e) => {
                let v = match self.eval(e) { Flow::Ok(v) => v, f => return f };
                let cur = self.get(name);
                let nv = match *op {
                    "+=" => match (&cur, &v) {
                        (Val::Str(a), _) => Val::Str(format!("{}{}", a, v.display())),
                        _                => Val::Int(cur.as_int().wrapping_add(v.as_int())),
                    },
                    "-=" => Val::Int(cur.as_int().wrapping_sub(v.as_int())),
                    "*=" => Val::Int(cur.as_int().wrapping_mul(v.as_int())),
                    "/=" => { let d = v.as_int(); if d == 0 { return Flow::Err("division par zéro".into()); } Val::Int(cur.as_int() / d) }
                    _    => v,
                };
                self.set(name, nv);
            }
            S::Expr(e) => { match self.eval(e) { Flow::Ok(_) => {} f => return f } }
            S::Return(e) => {
                let v = match e { Some(e) => match self.eval(e) { Flow::Ok(v) => v, f => return f }, None => Val::Unit };
                return Flow::Ret(v);
            }
            S::Break    => return Flow::Brk,
            S::Continue => return Flow::Cnt,
            S::If(cond, then, else_) => {
                let cv = match self.eval(cond) { Flow::Ok(v) => v, f => return f };
                self.push();
                let r = if cv.truthy() { self.run(then) }
                        else if let Some(eb) = else_ { self.run(eb) }
                        else { Flow::Ok(Val::Unit) };
                self.pop();
                match r { Flow::Ok(_) => {} other => return other }
            }
            S::While(cond, body) => loop {
                if !self.step() { return Flow::Err("timeout".into()); }
                let cv = match self.eval(cond) { Flow::Ok(v) => v, f => return f };
                if !cv.truthy() { break; }
                self.push(); let r = self.run(body); self.pop();
                match r { Flow::Ok(_) => {} Flow::Brk => break, Flow::Cnt => continue, other => return other }
            },
            S::Loop(body) => loop {
                if !self.step() { return Flow::Err("timeout".into()); }
                self.push(); let r = self.run(body); self.pop();
                match r { Flow::Ok(_) => {} Flow::Brk => break, Flow::Cnt => continue, other => return other }
            },
            S::For(var, range_e, body) => {
                let rv = match self.eval(range_e) { Flow::Ok(v) => v, f => return f };
                let (lo, hi) = match rv {
                    Val::Range(a, b) => (a, b),
                    Val::Int(n) => (0, n),
                    _ => (0, 0),
                };
                let mut n = lo;
                while n < hi {
                    if !self.step() { return Flow::Err("timeout".into()); }
                    self.push();
                    self.def(var, Val::Int(n));
                    let r = self.run(body);
                    self.pop();
                    match r { Flow::Ok(_) => {} Flow::Brk => break, Flow::Cnt => {} other => return other }
                    n += 1;
                }
            }
        }
        Flow::Ok(Val::Unit)
    }

    fn eval(&mut self, e: &E) -> Flow {
        if !self.step() { return Flow::Err("timeout".into()); }
        match e {
            E::Int(n)  => Flow::Ok(Val::Int(*n)),
            E::Bool(b) => Flow::Ok(Val::Bool(*b)),
            E::Str(s)  => Flow::Ok(Val::Str(s.clone())),
            E::Var(n)  => Flow::Ok(self.get(n)),
            E::Range(lo, hi) => {
                let a = match self.eval(lo) { Flow::Ok(v) => v.as_int(), f => return f };
                let b = match self.eval(hi) { Flow::Ok(v) => v.as_int(), f => return f };
                Flow::Ok(Val::Range(a, b))
            }
            E::UnOp(op, inner) => {
                let v = match self.eval(inner) { Flow::Ok(v) => v, f => return f };
                Flow::Ok(match *op {
                    "-" => Val::Int(-v.as_int()),
                    "!" => Val::Bool(!v.truthy()),
                    _   => v,
                })
            }
            E::BinOp(l, op, r) => {
                let lv = match self.eval(l) { Flow::Ok(v) => v, f => return f };
                // Court-circuit
                if *op == "&&" && !lv.truthy() { return Flow::Ok(Val::Bool(false)); }
                if *op == "||" &&  lv.truthy() { return Flow::Ok(Val::Bool(true)); }
                let rv = match self.eval(r) { Flow::Ok(v) => v, f => return f };
                Flow::Ok(match (*op, &lv, &rv) {
                    ("+", Val::Str(a), _) => Val::Str(format!("{}{}", a, rv.display())),
                    ("+", _, Val::Str(b)) => Val::Str(format!("{}{}", lv.display(), b)),
                    ("+", _, _) => Val::Int(lv.as_int().wrapping_add(rv.as_int())),
                    ("-", _, _) => Val::Int(lv.as_int().wrapping_sub(rv.as_int())),
                    ("*", _, _) => Val::Int(lv.as_int().wrapping_mul(rv.as_int())),
                    ("/", _, _) => { let d = rv.as_int(); if d == 0 { return Flow::Err("division par zéro".into()); } Val::Int(lv.as_int() / d) }
                    ("%", _, _) => { let d = rv.as_int(); if d == 0 { return Flow::Err("modulo par zéro".into()); } Val::Int(lv.as_int() % d) }
                    ("==", Val::Str(a), Val::Str(b)) => Val::Bool(a == b),
                    ("==", _, _) => Val::Bool(lv.as_int() == rv.as_int()),
                    ("!=", Val::Str(a), Val::Str(b)) => Val::Bool(a != b),
                    ("!=", _, _) => Val::Bool(lv.as_int() != rv.as_int()),
                    ("<",  _, _) => Val::Bool(lv.as_int() <  rv.as_int()),
                    (">",  _, _) => Val::Bool(lv.as_int() >  rv.as_int()),
                    ("<=", _, _) => Val::Bool(lv.as_int() <= rv.as_int()),
                    (">=", _, _) => Val::Bool(lv.as_int() >= rv.as_int()),
                    ("||", _, _) => Val::Bool(lv.truthy() || rv.truthy()),
                    ("&&", _, _) => Val::Bool(lv.truthy() && rv.truthy()),
                    _ => Val::Unit,
                })
            }
            E::Fmt(mac, fmt, args) => {
                let mut vals = Vec::new();
                for a in args { match self.eval(a) { Flow::Ok(v) => vals.push(v), f => return f } }
                self.exec_macro(mac, fmt, vals)
            }
            E::Call(name, args) => {
                let mut vals = Vec::new();
                for a in args { match self.eval(a) { Flow::Ok(v) => vals.push(v), f => return f } }
                self.call(name, vals)
            }
            E::Method(obj, method, args) => {
                let ov = match self.eval(obj) { Flow::Ok(v) => v, f => return f };
                let mut vals = Vec::new();
                for a in args { match self.eval(a) { Flow::Ok(v) => vals.push(v), f => return f } }
                Flow::Ok(self.method(ov, method, vals))
            }
        }
    }

    fn exec_macro(&mut self, mac: &str, fmt: &str, vals: Vec<Val>) -> Flow {
        match mac {
            "println" => {
                let s = fmt_apply(fmt, &vals);
                self.out.push_str(&s); self.out.push('\n');
            }
            "print" => {
                let s = fmt_apply(fmt, &vals);
                self.out.push_str(&s);
            }
            "eprint" | "eprintln" => {
                let s = fmt_apply(fmt, &vals);
                self.out.push_str(&s);
                if mac == "eprintln" { self.out.push('\n'); }
            }
            "format" => {
                return Flow::Ok(Val::Str(fmt_apply(fmt, &vals)));
            }
            "assert" => {
                if vals.is_empty() || !vals[0].truthy() {
                    let msg = if fmt.is_empty() { "assertion échouée".into() }
                              else { format!("assertion échouée: {}", fmt_apply(fmt, &vals[1..])) };
                    return Flow::Err(msg);
                }
            }
            "assert_eq" => {
                if vals.len() >= 2 {
                    let eq = match (&vals[0], &vals[1]) {
                        (Val::Str(a), Val::Str(b)) => a == b,
                        (a, b) => a.as_int() == b.as_int(),
                    };
                    if !eq { return Flow::Err(format!("assert_eq: {} != {}", vals[0].display(), vals[1].display())); }
                }
            }
            "assert_ne" => {
                if vals.len() >= 2 {
                    let eq = match (&vals[0], &vals[1]) {
                        (Val::Str(a), Val::Str(b)) => a == b,
                        (a, b) => a.as_int() == b.as_int(),
                    };
                    if eq { return Flow::Err(format!("assert_ne: {} == {}", vals[0].display(), vals[1].display())); }
                }
            }
            "panic"         => return Flow::Err(format!("panic: {}", fmt_apply(fmt, &vals))),
            "todo"          => return Flow::Err("non implémenté (todo!)".into()),
            "unimplemented" => return Flow::Err("non implémenté".into()),
            "dbg" => {
                for v in &vals { self.out.push_str(&format!("[dbg] {}\n", v.display())); }
            }
            "vec" => {
                return Flow::Ok(vals.into_iter().next().unwrap_or(Val::Unit));
            }
            "__unit__" => {}
            _ => {}
        }
        Flow::Ok(Val::Unit)
    }

    fn method(&self, obj: Val, m: &str, args: Vec<Val>) -> Val {
        match m {
            "len"        => Val::Int(match &obj { Val::Str(s) => s.len() as i64, _ => 0 }),
            "is_empty"   => Val::Bool(match &obj { Val::Str(s) => s.is_empty(), _ => true }),
            "abs"        => Val::Int(obj.as_int().abs()),
            "to_string"  | "to_owned" | "clone" => obj,
            "to_uppercase" => Val::Str(match obj { Val::Str(s) => s.chars().map(|c| c.to_ascii_uppercase()).collect(), _ => String::new() }),
            "to_lowercase" => Val::Str(match obj { Val::Str(s) => s.chars().map(|c| c.to_ascii_lowercase()).collect(), _ => String::new() }),
            "trim"       => Val::Str(match obj { Val::Str(s) => s.trim().to_string(), _ => String::new() }),
            "contains"   => Val::Bool(match &obj { Val::Str(s) => s.contains(args.get(0).map(Val::display).as_deref().unwrap_or("")), _ => false }),
            "starts_with"=> Val::Bool(match &obj { Val::Str(s) => s.starts_with(args.get(0).map(Val::display).as_deref().unwrap_or("")), _ => false }),
            "ends_with"  => Val::Bool(match &obj { Val::Str(s) => s.ends_with(args.get(0).map(Val::display).as_deref().unwrap_or("")), _ => false }),
            "repeat"     => Val::Str(match obj { Val::Str(s) => { let n = args.get(0).map(|v| v.as_int()).unwrap_or(0) as usize; s.repeat(n) }, _ => String::new() }),
            "min"        => Val::Int(obj.as_int().min(args.get(0).map(|v| v.as_int()).unwrap_or(i64::MAX))),
            "max"        => Val::Int(obj.as_int().max(args.get(0).map(|v| v.as_int()).unwrap_or(i64::MIN))),
            "pow"        => Val::Int(obj.as_int().wrapping_pow(args.get(0).map(|v| v.as_int()).unwrap_or(0) as u32)),
            "sqrt"       => Val::Int(isqrt(obj.as_int().max(0))),
            "count" | "sum" | "product" | "collect" => obj,
            "__idx__"    => match &obj { Val::Str(s) => Val::Int(s.as_bytes().get(args.get(0).map(|v| v.as_int()).unwrap_or(0) as usize).copied().unwrap_or(0) as i64), _ => Val::Unit },
            "new" | "default" | "iter" | "into_iter" | "chars" | "bytes" | "rev" | "enumerate" => obj,
            _ => Val::Unit,
        }
    }

    fn call(&mut self, name: &str, args: Vec<Val>) -> Flow {
        // Fonctions intégrées
        match name {
            "__unit__" | "__assoc__" => return Flow::Ok(Val::Unit),
            "abs"      => return Flow::Ok(Val::Int(args.get(0).map(|v| v.as_int()).unwrap_or(0).abs())),
            "min"      => return Flow::Ok(Val::Int(args.get(0).map(|v| v.as_int()).unwrap_or(0).min(args.get(1).map(|v| v.as_int()).unwrap_or(0)))),
            "max"      => return Flow::Ok(Val::Int(args.get(0).map(|v| v.as_int()).unwrap_or(0).max(args.get(1).map(|v| v.as_int()).unwrap_or(0)))),
            "sqrt"     => return Flow::Ok(Val::Int(isqrt(args.get(0).map(|v| v.as_int()).unwrap_or(0).max(0)))),
            _ => {}
        }
        // Fonction utilisateur
        if let Some(func) = self.fns.get(name).cloned() {
            if self.scopes.len() > 64 { return Flow::Err("débordement de pile".into()); }
            self.push();
            for (i, param) in func.params.iter().enumerate() {
                let v = args.get(i).cloned().unwrap_or(Val::Unit);
                self.def(param, v);
            }
            let r = self.run(&func.body);
            self.pop();
            return match r { Flow::Ret(v) | Flow::Ok(v) => Flow::Ok(v), other => other };
        }
        Flow::Ok(Val::Unit)
    }
}

// ─── Application de la chaîne de format ──────────────────────────────────────

fn fmt_apply(fmt: &str, args: &[Val]) -> String {
    if fmt.is_empty() {
        return args.iter().map(|v| v.display()).collect::<Vec<_>>().join(", ");
    }
    let mut out = String::new();
    let mut ai = 0usize;
    let b = fmt.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'{' {
            if i+1 < b.len() && b[i+1] == b'{' { out.push('{'); i += 2; continue; }
            let start = i + 1;
            while i < b.len() && b[i] != b'}' { i += 1; }
            let _spec = &fmt[start..i];
            i += 1;
            if let Some(v) = args.get(ai) { out.push_str(&v.display()); ai += 1; }
        } else if b[i] == b'}' && i+1 < b.len() && b[i+1] == b'}' {
            out.push('}'); i += 2;
        } else {
            out.push(b[i] as char); i += 1;
        }
    }
    out
}

// ─── Collecte des fonctions (1er passage) ────────────────────────────────────

fn collect_fns(src: &str) -> BTreeMap<String, FnDef> {
    let toks = lex(src);
    let mut p = Pr { t: toks, i: 0 };
    let mut fns = BTreeMap::new();
    while p.pk() != &T::Eof {
        while matches!(p.pk(), T::KPub) { p.i += 1; }
        if p.pk() == &T::KFn {
            p.i += 1;
            let name = p.eat_id().unwrap_or_default();
            // Saute paramètres génériques <T…>
            if p.pk() == &T::P(b'<') {
                let mut d = 0i32;
                loop { match p.pk() { T::P(b'<') => { d += 1; p.i += 1; } T::P(b'>') => { p.i += 1; d -= 1; if d <= 0 { break; } } T::Eof => break, _ => { p.i += 1; } } }
            }
            let params = p.parse_params();
            p.skip_ret_type();
            let body = p.parse_block();
            fns.insert(name, FnDef { params, body });
        } else {
            p.skip_item();
        }
    }
    fns
}

// ─── API publique ─────────────────────────────────────────────────────────────

/// Exécute un programme Rust (sous-ensemble). Renvoie (sortie, erreur_éventuelle).
pub fn run(src: &str) -> (String, Option<String>) {
    let fns = collect_fns(src);
    let mut out = String::new();
    if let Some(main) = fns.get("main").cloned() {
        let mut ctx = Ctx { fns: &fns, scopes: alloc::vec![BTreeMap::new()], out: &mut out, steps: 0 };
        let r = ctx.run(&main.body);
        let err = match r { Flow::Err(e) => Some(e), _ => None };
        (out, err)
    } else {
        (out, Some("pas de fn main() trouvée".into()))
    }
}

/// Évalue une expression simple et renvoie le résultat sous forme de chaîne.
pub fn eval_expr(src: &str) -> String {
    let toks = lex(src);
    let mut p = Pr { t: toks, i: 0 };
    let e = p.expr();
    let fns = BTreeMap::new();
    let mut out = String::new();
    let mut ctx = Ctx { fns: &fns, scopes: alloc::vec![BTreeMap::new()], out: &mut out, steps: 0 };
    match ctx.eval(&e) { Flow::Ok(v) => v.display(), _ => "erreur".into() }
}
