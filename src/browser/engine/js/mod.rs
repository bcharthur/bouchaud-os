//! Moteur JavaScript reel (sous-ensemble ECMAScript) pour le navigateur.
//!
//! Pipeline : lexer -> parser (AST) -> interpreteur arborescent. Couvre
//! nombres/chaines/templates/booleens/null/undefined, objets & tableaux,
//! fonctions (declarations, expressions, fleche + closures), `this`,
//! operateurs (arith/comparaison/logique/bit/ternaire/affectation/++/--),
//! var/let/const, if/else, for, for-in/of, while, do-while, break/continue,
//! return, try/catch/throw ; built-ins console/Math/JSON/Array/String/Object.
//!
//! Integration page : `execute_inline(html)` execute les `<script>` inline (sur
//! un contexte partage), injecte la sortie `document.write(...)` a la position
//! du script et applique `getElementById(id).innerHTML/textContent = ...` sur
//! le HTML. Sandbox : nombre de pas borne (anti-boucle infinie), erreurs
//! capturees (un script fautif n'empeche pas le rendu).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use core::cell::RefCell;

// ============================================================================
// Valeurs
// ============================================================================

pub type NativeFn = fn(&mut Interp, Value, &[Value]) -> Result<Value, Value>;

#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<String>),
    Obj(Rc<RefCell<Obj>>),
}

pub struct Obj {
    pub props: OrderedMap,
    pub arr: Option<Vec<Value>>,
    pub call: Option<Callable>,
    pub class: &'static str,
}

/// Table de proprietes preservant l'ordre d'insertion (semantique JS, observable
/// via Object.keys / for-in / JSON.stringify). Acces lineaire : les objets des
/// pages web restent petits.
#[derive(Clone, Default)]
pub struct OrderedMap { keys: Vec<String>, vals: Vec<Value> }
impl OrderedMap {
    pub fn new() -> OrderedMap { OrderedMap { keys: Vec::new(), vals: Vec::new() } }
    pub fn insert(&mut self, k: String, v: Value) -> Option<Value> {
        if let Some(i) = self.keys.iter().position(|x| x == &k) {
            Some(core::mem::replace(&mut self.vals[i], v))
        } else { self.keys.push(k); self.vals.push(v); None }
    }
    pub fn get(&self, k: &str) -> Option<&Value> { self.keys.iter().position(|x| x == k).map(|i| &self.vals[i]) }
    pub fn get_mut(&mut self, k: &str) -> Option<&mut Value> { let i = self.keys.iter().position(|x| x == k)?; Some(&mut self.vals[i]) }
    pub fn contains_key(&self, k: &str) -> bool { self.keys.iter().any(|x| x == k) }
    pub fn is_empty(&self) -> bool { self.keys.is_empty() }
    pub fn len(&self) -> usize { self.keys.len() }
    pub fn keys(&self) -> impl Iterator<Item = &String> { self.keys.iter() }
    pub fn values(&self) -> impl Iterator<Item = &Value> { self.vals.iter() }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> { self.keys.iter().zip(self.vals.iter()) }
}

#[derive(Clone)]
pub enum Callable {
    User { def: Rc<FuncDef>, env: Env },
    Native(NativeFn),
}

impl Obj {
    fn plain() -> Obj { Obj { props: OrderedMap::new(), arr: None, call: None, class: "Object" } }
}

fn new_obj(o: Obj) -> Value { Value::Obj(Rc::new(RefCell::new(o))) }
pub fn str_val(s: impl Into<String>) -> Value { Value::Str(Rc::new(s.into())) }
fn array_val(items: Vec<Value>) -> Value {
    new_obj(Obj { props: OrderedMap::new(), arr: Some(items), call: None, class: "Array" })
}
fn native_val(f: NativeFn) -> Value {
    new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::Native(f)), class: "Function" })
}

// ============================================================================
// AST
// ============================================================================

#[derive(Clone)]
pub struct FuncDef {
    pub params: Vec<String>,
    pub rest: Option<String>,
    pub body: Vec<Stmt>,
    pub arrow: bool,
    pub expr_body: Option<Box<Expr>>,
}

#[derive(Clone)]
pub enum Expr {
    Num(f64), Str(String), Tpl(Vec<Expr>), Bool(bool), Null, Undef,
    Ident(String), This,
    Array(Vec<Expr>), Object(Vec<(String, Expr)>),
    Unary(String, Box<Expr>), Update(String, bool, Box<Expr>),
    Bin(String, Box<Expr>, Box<Expr>), Logic(String, Box<Expr>, Box<Expr>),
    Assign(String, Box<Expr>, Box<Expr>), Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>, bool), New(Box<Expr>, Vec<Expr>),
    Member(Box<Expr>, String, bool), Index(Box<Expr>, Box<Expr>),
    Func(Rc<FuncDef>), Seq(Vec<Expr>), Spread(Box<Expr>),
}

#[derive(Clone)]
pub enum Stmt {
    Expr(Expr), Var(bool, Vec<(String, Option<Expr>)>), Func(String, Rc<FuncDef>),
    Return(Option<Expr>), If(Expr, Box<Stmt>, Option<Box<Stmt>>), Block(Vec<Stmt>),
    While(Expr, Box<Stmt>), DoWhile(Box<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    ForIn(String, Expr, Box<Stmt>, bool), Break, Continue, Throw(Expr),
    Try(Vec<Stmt>, Option<(String, Vec<Stmt>)>, Option<Vec<Stmt>>), Empty,
    // name, extends_expr, [(is_static, method_name, def)]
    Class(String, Option<Expr>, Vec<(bool, String, Rc<FuncDef>)>),
    // discriminant, [(test_or_None_for_default, body_stmts)]
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
}

// ============================================================================
// Lexer
// ============================================================================

mod lexer;
use lexer::{Tok, TplPart, Lexer};

// `hexv` est partagé (lexer + décodage URI/JSON), il reste ici.
fn hexv(c: u8) -> u32 { match c { b'0'..=b'9' => (c - b'0') as u32, b'a'..=b'f' => (c - b'a' + 10) as u32, b'A'..=b'F' => (c - b'A' + 10) as u32, _ => 0 } }

// ============================================================================
// Parser
// ============================================================================

struct Parser { toks: Vec<(Tok, bool)>, i: usize }

impl Parser {
    fn new(toks: Vec<(Tok, bool)>) -> Parser { Parser { toks, i: 0 } }
    fn peek(&self) -> &Tok { &self.toks[self.i.min(self.toks.len() - 1)].0 }
    fn nl_before(&self) -> bool { self.toks[self.i.min(self.toks.len() - 1)].1 }
    fn next(&mut self) -> Tok { let t = self.toks[self.i.min(self.toks.len() - 1)].0.clone(); if self.i < self.toks.len() { self.i += 1; } t }
    fn is_punct(&self, p: &str) -> bool { matches!(self.peek(), Tok::Punct(x) if x == p) }
    fn is_kw(&self, k: &str) -> bool { matches!(self.peek(), Tok::Keyword(x) if x == k) }
    fn eat_punct(&mut self, p: &str) -> bool { if self.is_punct(p) { self.i += 1; true } else { false } }
    fn expect_punct(&mut self, p: &str) -> Result<(), String> { if self.eat_punct(p) { Ok(()) } else { Err(format!("attendu '{}'", p)) } }
    fn semi(&mut self) { let _ = self.eat_punct(";"); }

    fn parse_program(&mut self) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::new();
        let mut recovered = 0u32;
        let mut first_err: Option<String> = None;
        while !matches!(self.peek(), Tok::Eof) {
            let start = self.i;
            match self.parse_stmt() {
                Ok(s) => out.push(s),
                Err(e) => {
                    // Récupération : une seule erreur de syntaxe ne doit pas
                    // jeter tout un bundle (Google/React font >1 Mo). On saute
                    // jusqu'à la prochaine frontière de statement et on continue.
                    if first_err.is_none() { first_err = Some(e); }
                    if self.i == start { self.i += 1; } // garantit la progression
                    self.recover_to_stmt_boundary();
                    recovered += 1;
                    if recovered > 20_000 { break; }
                }
            }
        }
        if recovered > 0 {
            crate::dlog!(crate::diag::Cat::Js, "parser: {} statements ignores (1re erreur: {})",
                recovered, first_err.as_deref().unwrap_or("?"));
        }
        Ok(out)
    }

    // Avance jusqu'au prochain `;` (profondeur 0) ou `}` fermant, en équilibrant
    // parenthèses/crochets/accolades, pour reprendre le parsing après une erreur.
    fn recover_to_stmt_boundary(&mut self) {
        let mut depth = 0i32;
        while !matches!(self.peek(), Tok::Eof) {
            let mut stop = false;
            if let Tok::Punct(p) = self.peek() {
                match p.as_str() {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" => { if depth > 0 { depth -= 1; } }
                    "}" => { if depth == 0 { self.i += 1; return; } depth -= 1; }
                    ";" => { if depth <= 0 { stop = true; } }
                    _ => {}
                }
            }
            self.i += 1;
            if stop { return; }
        }
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.peek().clone() {
            Tok::Punct(p) if p == "{" => { self.i += 1; let mut b = Vec::new(); while !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) { b.push(self.parse_stmt()?); } self.expect_punct("}")?; Ok(Stmt::Block(b)) }
            Tok::Punct(p) if p == ";" => { self.i += 1; Ok(Stmt::Empty) }
            Tok::Keyword(k) => match k.as_str() {
                "var" | "let" | "const" => { let block = k != "var"; self.i += 1; let d = self.parse_var_decls()?; self.semi(); Ok(Stmt::Var(block, d)) }
                "function" => { self.i += 1; let name = self.ident()?; let def = self.parse_func_rest()?; Ok(Stmt::Func(name, Rc::new(def))) }
                "return" => { self.i += 1; if self.is_punct(";") || self.is_punct("}") || self.nl_before() || matches!(self.peek(), Tok::Eof) { self.semi(); Ok(Stmt::Return(None)) } else { let e = self.parse_expr()?; self.semi(); Ok(Stmt::Return(Some(e))) } }
                "if" => { self.i += 1; self.expect_punct("(")?; let c = self.parse_expr()?; self.expect_punct(")")?; let t = Box::new(self.parse_stmt()?); let e = if self.is_kw("else") { self.i += 1; Some(Box::new(self.parse_stmt()?)) } else { None }; Ok(Stmt::If(c, t, e)) }
                "while" => { self.i += 1; self.expect_punct("(")?; let c = self.parse_expr()?; self.expect_punct(")")?; Ok(Stmt::While(c, Box::new(self.parse_stmt()?))) }
                "do" => { self.i += 1; let body = Box::new(self.parse_stmt()?); if !self.is_kw("while") { return Err("attendu while".into()); } self.i += 1; self.expect_punct("(")?; let c = self.parse_expr()?; self.expect_punct(")")?; self.semi(); Ok(Stmt::DoWhile(body, c)) }
                "for" => self.parse_for(),
                "break" => { self.i += 1; self.semi(); Ok(Stmt::Break) }
                "continue" => { self.i += 1; self.semi(); Ok(Stmt::Continue) }
                "throw" => { self.i += 1; let e = self.parse_expr()?; self.semi(); Ok(Stmt::Throw(e)) }
                "try" => self.parse_try(),
                "class" => self.parse_class(),
                "switch" => self.parse_switch(),
                // async function / async arrow (traitement synchrone : await/async ignorés sémantiquement)
                "async" => {
                    self.i += 1;
                    if self.is_kw("function") {
                        self.i += 1;
                        // function anonyme ou nommée
                        let name_opt = if let Tok::Ident(_) = self.peek() { Some(self.ident()?) } else { None };
                        let def = self.parse_func_rest()?;
                        if let Some(n) = name_opt { Ok(Stmt::Func(n, Rc::new(def))) }
                        else { Ok(Stmt::Expr(Expr::Func(Rc::new(def)))) }
                    } else {
                        // async arrow ou expression  ; parse_expr gère la suite
                        let e = self.parse_expr()?; self.semi(); Ok(Stmt::Expr(e))
                    }
                }
                // export/import : on ignore le mot-clé et on parse le reste normalement
                "from" | "static" | "get" | "set" => { let e = self.parse_expr()?; self.semi(); Ok(Stmt::Expr(e)) }
                _ => { let e = self.parse_expr()?; self.semi(); Ok(Stmt::Expr(e)) }
            },
            // Statement etiquete `label: stmt` (tres frequent en JS minifie pour
            // les boucles `a:for(...){...break a}`). On consomme l'etiquette et on
            // parse le statement suivant ; break/continue retombent sur la boucle
            // la plus proche (suffisant pour la plupart des cas).
            Tok::Ident(_) if matches!(self.toks.get(self.i + 1).map(|t| &t.0), Some(Tok::Punct(p)) if p == ":") => {
                self.i += 2; // saute `label` et `:`
                self.parse_stmt()
            }
            _ => { let e = self.parse_expr()?; self.semi(); Ok(Stmt::Expr(e)) }
        }
    }

    fn skip_balanced_after_brace(&mut self) -> Result<(), String> {
        while !self.is_punct("{") && !matches!(self.peek(), Tok::Eof) { self.i += 1; }
        if self.eat_punct("{") { let mut d = 1; while d > 0 && !matches!(self.peek(), Tok::Eof) { if self.is_punct("{") { d += 1; } else if self.is_punct("}") { d -= 1; } self.i += 1; } }
        Ok(())
    }

    fn parse_try(&mut self) -> Result<Stmt, String> {
        self.i += 1; self.expect_punct("{")?;
        let mut tb = Vec::new(); while !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) { tb.push(self.parse_stmt()?); } self.expect_punct("}")?;
        let mut catch = None;
        if self.is_kw("catch") {
            self.i += 1;
            let mut param = String::from("e");
            if self.eat_punct("(") {
                if self.is_punct("{") || self.is_punct("[") {
                    self.skip_pattern(); // catch ({message}) ou catch ([a, b])
                } else {
                    param = self.ident()?;
                }
                self.expect_punct(")")?;
            }
            self.expect_punct("{")?;
            let mut cb = Vec::new();
            while !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) { cb.push(self.parse_stmt()?); }
            self.expect_punct("}")?;
            catch = Some((param, cb));
        }
        let mut fin = None;
        if self.is_kw("finally") { self.i += 1; self.expect_punct("{")?; let mut fb = Vec::new(); while !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) { fb.push(self.parse_stmt()?); } self.expect_punct("}")?; fin = Some(fb); }
        Ok(Stmt::Try(tb, catch, fin))
    }

    fn parse_class(&mut self) -> Result<Stmt, String> {
        self.i += 1; // skip 'class'
        // anonymous class expression or named
        let name = if matches!(self.peek(), Tok::Ident(_) | Tok::Keyword(_)) { self.ident()? } else { "__anon__".into() };
        let extends = if self.is_kw("extends") {
            self.i += 1;
            Some(self.parse_member_only()?)
        } else { None };
        self.expect_punct("{")?;
        let mut methods: Vec<(bool, String, Rc<FuncDef>)> = Vec::new();
        while !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) {
            if self.is_punct(";") { self.i += 1; continue; }
            let is_static = self.is_kw("static");
            if is_static { self.i += 1; }
            // static { ... } initializer block
            if is_static && self.is_punct("{") { self.skip_pattern(); continue; }
            // skip async keyword before method name
            if self.is_kw("async") {
                let next_is_name = self.toks.get(self.i + 1).map(|t| !matches!(&t.0, Tok::Punct(p) if p == "(" || p == ";" || p == "}" || p == "=")).unwrap_or(false);
                if next_is_name { self.i += 1; }
            }
            // get/set contextual keywords
            if let Tok::Keyword(k) | Tok::Ident(k) = self.peek().clone() {
                if k == "get" || k == "set" {
                    let next_is_name = self.toks.get(self.i + 1).map(|t| !matches!(&t.0, Tok::Punct(p) if p == "(" || p == ";" || p == "}" || p == "=")).unwrap_or(false);
                    if next_is_name { self.i += 1; }
                }
            }
            // generator `*`
            if self.is_punct("*") { self.i += 1; }
            let mname = match self.peek().clone() {
                Tok::Ident(n)  => { self.i += 1; n }
                Tok::Keyword(k) => { self.i += 1; k }
                Tok::Str(s)    => { self.i += 1; s }
                Tok::Num(n)    => { self.i += 1; format!("{}", n) }
                // private field: #name
                Tok::Punct(p) if p == "#" => {
                    self.i += 1;
                    if let Tok::Ident(_) | Tok::Keyword(_) = self.peek() { self.ident().unwrap_or_default() } else { "__priv__".into() }
                }
                // computed property: [expr]
                Tok::Punct(p) if p == "[" => {
                    self.skip_pattern(); "__computed__".into()
                }
                _ => {
                    // unknown token: skip to next ; or }
                    while !self.is_punct(";") && !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) {
                        if self.is_punct("{") || self.is_punct("[") { self.skip_pattern(); } else { self.i += 1; }
                    }
                    self.eat_punct(";");
                    continue;
                }
            };
            if self.is_punct("(") {
                let def = self.parse_func_rest()?;
                methods.push((is_static, mname, Rc::new(def)));
            } else {
                if self.is_punct("=") { self.i += 1; let _ = self.parse_assign(); }
                self.eat_punct(";");
            }
        }
        self.eat_punct("}");
        Ok(Stmt::Class(name, extends, methods))
    }

    fn parse_switch(&mut self) -> Result<Stmt, String> {
        self.i += 1; // skip 'switch'
        self.expect_punct("(")?;
        let disc = self.parse_expr()?;
        self.expect_punct(")")?;
        self.expect_punct("{")?;
        let mut cases: Vec<(Option<Expr>, Vec<Stmt>)> = Vec::new();
        while !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) {
            let test = if self.is_kw("case") {
                self.i += 1;
                let e = self.parse_expr()?;
                self.expect_punct(":")?;
                Some(e)
            } else if self.is_kw("default") {
                self.i += 1;
                self.eat_punct(":");
                None
            } else {
                break;
            };
            let mut body = Vec::new();
            while !self.is_kw("case") && !self.is_kw("default") && !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) {
                body.push(self.parse_stmt()?);
            }
            cases.push((test, body));
        }
        self.eat_punct("}");
        Ok(Stmt::Switch(disc, cases))
    }

    fn parse_for(&mut self) -> Result<Stmt, String> {
        self.i += 1; self.expect_punct("(")?;
        let decl = if let Tok::Keyword(k) = self.peek() { if k == "var" || k == "let" || k == "const" { true } else { false } } else { false };
        let save = self.i;
        if decl { self.i += 1; }
        // for (let name of/in expr) — var simple
        if let Tok::Ident(name) = self.peek().clone() {
            let after = (self.i + 1).min(self.toks.len() - 1);
            if matches!(&self.toks[after].0, Tok::Keyword(k) if k == "in" || k == "of") {
                self.i += 1;
                let is_of = matches!(self.peek(), Tok::Keyword(k) if k == "of");
                self.i += 1;
                let obj = self.parse_expr()?; self.expect_punct(")")?;
                let body = Box::new(self.parse_stmt()?);
                return Ok(Stmt::ForIn(name, obj, body, is_of));
            }
        }
        // for (let {pat}/[pat] of/in expr) — destructuration
        if self.is_punct("{") || self.is_punct("[") {
            self.skip_pattern();
            // éventuel = default pour le motif entier
            if self.is_punct("=") { self.i += 1; let _ = self.parse_assign()?; }
            if matches!(self.peek(), Tok::Keyword(k) if k == "of" || k == "in") {
                let is_of = matches!(self.peek(), Tok::Keyword(k) if k == "of");
                self.i += 1;
                let obj = self.parse_expr()?; self.expect_punct(")")?;
                let body = Box::new(self.parse_stmt()?);
                return Ok(Stmt::ForIn("__pat__".into(), obj, body, is_of));
            }
        }
        self.i = save;
        let init = if self.is_punct(";") { self.i += 1; None } else {
            let s = if let Tok::Keyword(k) = self.peek().clone() { if k == "var" || k == "let" || k == "const" { self.i += 1; Stmt::Var(k != "var", self.parse_var_decls()?) } else { Stmt::Expr(self.parse_expr()?) } } else { Stmt::Expr(self.parse_expr()?) };
            self.expect_punct(";")?; Some(Box::new(s))
        };
        let test = if self.is_punct(";") { None } else { Some(self.parse_expr()?) };
        self.expect_punct(";")?;
        let update = if self.is_punct(")") { None } else { Some(self.parse_expr()?) };
        self.expect_punct(")")?;
        Ok(Stmt::For(init, test, update, Box::new(self.parse_stmt()?)))
    }

    fn parse_var_decls(&mut self) -> Result<Vec<(String, Option<Expr>)>, String> {
        let mut out = Vec::new();
        loop {
            // destructuration ignoree (on saute jusqu'a = ou , ou ;)
            if self.is_punct("{") || self.is_punct("[") { let mut d = 0; loop { if self.is_punct("{") || self.is_punct("[") { d += 1; } if self.is_punct("}") || self.is_punct("]") { d -= 1; } self.i += 1; if d == 0 || matches!(self.peek(), Tok::Eof) { break; } } if self.eat_punct("=") { let _ = self.parse_assign()?; } if !self.eat_punct(",") { break; } continue; }
            let name = self.ident()?;
            let init = if self.eat_punct("=") { Some(self.parse_assign()?) } else { None };
            out.push((name, init));
            if !self.eat_punct(",") { break; }
        }
        Ok(out)
    }

    fn ident(&mut self) -> Result<String, String> { match self.next() { Tok::Ident(s) => Ok(s), Tok::Keyword(k) => Ok(k), _ => Err("attendu identifiant".into()) } }

    fn parse_func_rest(&mut self) -> Result<FuncDef, String> {
        self.expect_punct("(")?; let (params, rest) = self.parse_params()?; self.expect_punct(")")?;
        self.expect_punct("{")?; let mut body = Vec::new(); while !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) { body.push(self.parse_stmt()?); } self.expect_punct("}")?;
        Ok(FuncDef { params, rest, body, arrow: false, expr_body: None })
    }

    /// Saute un pattern de destructuration `{...}` ou `[...]` en gerant
    /// l'imbrication. S'arrete apres avoir consomme le `}`/`]` correspondant.
    fn skip_pattern(&mut self) {
        if !(self.is_punct("{") || self.is_punct("[")) { return; }
        let mut d = 0;
        loop {
            if self.is_punct("{") || self.is_punct("[") { d += 1; }
            else if self.is_punct("}") || self.is_punct("]") { d -= 1; }
            self.i += 1;
            if d == 0 || matches!(self.peek(), Tok::Eof) { break; }
        }
    }

    fn parse_params(&mut self) -> Result<(Vec<String>, Option<String>), String> {
        let mut params = Vec::new(); let mut rest = None;
        while !self.is_punct(")") {
            if self.eat_punct("...") {
                // rest peut etre un identifiant ou un pattern de destructuration
                if self.is_punct("{") || self.is_punct("[") { self.skip_pattern(); } else { rest = Some(self.ident()?); }
                break;
            }
            // parametre destructure : `{a, b}` ou `[a, b]` (eventuellement avec defaut)
            if self.is_punct("{") || self.is_punct("[") {
                self.skip_pattern();
                if self.eat_punct("=") { let _ = self.parse_assign()?; }
                if !self.eat_punct(",") { break; }
                continue;
            }
            let name = self.ident()?;
            if self.eat_punct("=") { let _ = self.parse_assign()?; }
            params.push(name);
            if !self.eat_punct(",") { break; }
        }
        Ok((params, rest))
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_assign()?;
        if self.is_punct(",") { let mut seq = vec![e]; while self.eat_punct(",") { seq.push(self.parse_assign()?); } e = Expr::Seq(seq); }
        Ok(e)
    }

    fn parse_assign(&mut self) -> Result<Expr, String> {
        if let Some(a) = self.try_arrow()? { return Ok(a); }
        let left = self.parse_cond()?;
        let ops = ["=","+=","-=","*=","/=","%=","**=","<<=",">>=","&=","|=","^=","&&=","||=","??="];
        if let Tok::Punct(p) = self.peek().clone() { if ops.contains(&p.as_str()) { self.i += 1; let right = self.parse_assign()?; return Ok(Expr::Assign(p, Box::new(left), Box::new(right))); } }
        Ok(left)
    }

    fn try_arrow(&mut self) -> Result<Option<Expr>, String> {
        let save = self.i;
        if self.is_punct("(") {
            let mut d = 0; let mut j = self.i;
            while j < self.toks.len() { match &self.toks[j].0 { Tok::Punct(p) if p == "(" => d += 1, Tok::Punct(p) if p == ")" => { d -= 1; if d == 0 { break; } }, Tok::Eof => break, _ => {} } j += 1; }
            if j + 1 < self.toks.len() && matches!(&self.toks[j + 1].0, Tok::Punct(p) if p == "=>") {
                self.i += 1; let (params, rest) = self.parse_params()?; self.expect_punct(")")?; self.expect_punct("=>")?;
                return Ok(Some(self.arrow_body(params, rest)?));
            }
        } else if let Tok::Ident(name) = self.peek().clone() {
            if matches!(&self.toks[(self.i + 1).min(self.toks.len() - 1)].0, Tok::Punct(p) if p == "=>") { self.i += 2; return Ok(Some(self.arrow_body(vec![name], None)?)); }
        }
        self.i = save; Ok(None)
    }

    fn arrow_body(&mut self, params: Vec<String>, rest: Option<String>) -> Result<Expr, String> {
        if self.is_punct("{") { self.i += 1; let mut body = Vec::new(); while !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) { body.push(self.parse_stmt()?); } self.expect_punct("}")?; Ok(Expr::Func(Rc::new(FuncDef { params, rest, body, arrow: true, expr_body: None }))) }
        else { let e = self.parse_assign()?; Ok(Expr::Func(Rc::new(FuncDef { params, rest, body: Vec::new(), arrow: true, expr_body: Some(Box::new(e)) }))) }
    }

    fn parse_cond(&mut self) -> Result<Expr, String> {
        let c = self.parse_binary(0)?;
        if self.eat_punct("?") { let t = self.parse_assign()?; self.expect_punct(":")?; let e = self.parse_assign()?; return Ok(Expr::Cond(Box::new(c), Box::new(t), Box::new(e))); }
        Ok(c)
    }

    fn bin_prec(op: &str) -> Option<(u8, bool)> {
        Some(match op {
            "??" => (1, true), "||" => (2, true), "&&" => (3, true),
            "|" => (4, false), "^" => (5, false), "&" => (6, false),
            "==" | "!=" | "===" | "!==" => (7, false),
            "<" | ">" | "<=" | ">=" | "instanceof" | "in" => (8, false),
            "<<" | ">>" | ">>>" => (9, false), "+" | "-" => (10, false),
            "*" | "/" | "%" => (11, false), "**" => (12, false), _ => return None,
        })
    }

    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() { Tok::Punct(p) => p.clone(), Tok::Keyword(k) if k == "instanceof" || k == "in" => k.clone(), _ => break };
            let (prec, is_logic) = match Self::bin_prec(&op) { Some(x) => x, None => break };
            if prec < min_prec { break; }
            self.i += 1;
            let right = self.parse_binary(if op == "**" { prec } else { prec + 1 })?;
            left = if is_logic { Expr::Logic(op, Box::new(left), Box::new(right)) } else { Expr::Bin(op, Box::new(left), Box::new(right)) };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            Tok::Punct(p) if p == "!" || p == "-" || p == "+" || p == "~" => { self.i += 1; Ok(Expr::Unary(p, Box::new(self.parse_unary()?))) }
            Tok::Punct(p) if p == "++" || p == "--" => { self.i += 1; Ok(Expr::Update(p, true, Box::new(self.parse_unary()?))) }
            Tok::Keyword(k) if k == "typeof" || k == "void" || k == "delete" => { self.i += 1; Ok(Expr::Unary(k, Box::new(self.parse_unary()?))) }
            // await : en contexte synchrone on évalue simplement la valeur immédiatement
            Tok::Keyword(k) if k == "await" => { self.i += 1; self.parse_unary() }
            // yield : on évalue l'expression yieldée mais on ne suspend pas
            Tok::Keyword(k) if k == "yield" => {
                self.i += 1;
                if !self.nl_before() && !self.is_punct(";") && !self.is_punct("}") && !matches!(self.peek(), Tok::Eof) {
                    let _ = self.parse_assign()?;
                }
                Ok(Expr::Undef)
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_call_member()?;
        if !self.nl_before() { if let Tok::Punct(p) = self.peek().clone() { if p == "++" || p == "--" { self.i += 1; e = Expr::Update(p, false, Box::new(e)); } } }
        Ok(e)
    }

    fn parse_call_member(&mut self) -> Result<Expr, String> {
        let mut e = if self.is_kw("new") {
            self.i += 1; let callee = self.parse_member_only()?; let args = if self.is_punct("(") { self.parse_args()? } else { Vec::new() }; Expr::New(Box::new(callee), args)
        } else { self.parse_primary()? };
        loop {
            if self.eat_punct(".") { let name = self.ident()?; e = Expr::Member(Box::new(e), name, false); }
            else if self.is_punct("?.") { self.i += 1; if self.is_punct("(") { let args = self.parse_args()?; e = Expr::Call(Box::new(e), args, true); } else { let name = self.ident()?; e = Expr::Member(Box::new(e), name, true); } }
            else if self.eat_punct("[") { let idx = self.parse_expr()?; self.expect_punct("]")?; e = Expr::Index(Box::new(e), Box::new(idx)); }
            else if self.is_punct("(") { let args = self.parse_args()?; e = Expr::Call(Box::new(e), args, false); }
            else { break; }
        }
        Ok(e)
    }

    fn parse_member_only(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_primary()?;
        loop {
            if self.eat_punct(".") { let name = self.ident()?; e = Expr::Member(Box::new(e), name, false); }
            else if self.eat_punct("[") { let idx = self.parse_expr()?; self.expect_punct("]")?; e = Expr::Index(Box::new(e), Box::new(idx)); }
            else { break; }
        }
        Ok(e)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, String> {
        self.expect_punct("(")?; let mut args = Vec::new();
        while !self.is_punct(")") {
            if self.eat_punct("...") { args.push(Expr::Spread(Box::new(self.parse_assign()?))); } else { args.push(self.parse_assign()?); }
            if !self.eat_punct(",") { break; }
        }
        self.expect_punct(")")?; Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Tpl(parts) => { let mut out = Vec::new(); for p in parts { match p { TplPart::Str(s) => out.push(Expr::Str(s)), TplPart::Expr(toks) => { let mut t: Vec<(Tok, bool)> = toks.into_iter().map(|x| (x, false)).collect(); t.push((Tok::Eof, false)); out.push(Parser::new(t).parse_expr()?); } } } Ok(Expr::Tpl(out)) }
            Tok::Regex => Ok(Expr::Object(Vec::new())),
            Tok::Ident(s) => Ok(Expr::Ident(s)),
            Tok::Keyword(k) => match k.as_str() {
                "true" => Ok(Expr::Bool(true)), "false" => Ok(Expr::Bool(false)), "null" => Ok(Expr::Null),
                "undefined" => Ok(Expr::Undef), "this" => Ok(Expr::This),
                "function" => {
                    // optionnellement : `function* name` (generateur) — on ignore le `*`
                    if self.is_punct("*") { self.i += 1; }
                    let _ = if let Tok::Ident(_) | Tok::Keyword(_) = self.peek() { Some(self.next()) } else { None };
                    let def = self.parse_func_rest()?;
                    Ok(Expr::Func(Rc::new(def)))
                }
                "async" => {
                    // async function expression ou async arrow
                    if self.is_kw("function") {
                        self.i += 1;
                        if self.is_punct("*") { self.i += 1; }
                        let _ = if let Tok::Ident(_) = self.peek() { Some(self.next()) } else { None };
                        let def = self.parse_func_rest()?;
                        Ok(Expr::Func(Rc::new(def)))
                    } else {
                        // async arrow : async (params) => body  ou  async param => body
                        // On réutilise try_arrow qui regarde en avant depuis la position courante.
                        // Si ce n'est pas une arrow, traiter async comme un identifiant.
                        if let Some(arrow) = self.try_arrow()? { Ok(arrow) }
                        else { Ok(Expr::Ident("async".into())) }
                    }
                }
                _ => Ok(Expr::Ident(k)),
            },
            Tok::Punct(p) => match p.as_str() {
                "(" => { let e = self.parse_expr()?; self.expect_punct(")")?; Ok(e) }
                "[" => { let mut items = Vec::new(); while !self.is_punct("]") { if self.eat_punct("...") { items.push(Expr::Spread(Box::new(self.parse_assign()?))); } else if self.is_punct(",") { items.push(Expr::Undef); } else { items.push(self.parse_assign()?); } if !self.eat_punct(",") { break; } } self.expect_punct("]")?; Ok(Expr::Array(items)) }
                "{" => self.parse_object(),
                // recovery: skip token and return undefined
                _ => { Ok(Expr::Undef) }
            },
            Tok::Eof => Err("fin inattendue".into()),
        }
    }

    fn parse_object(&mut self) -> Result<Expr, String> {
        let mut props = Vec::new();
        while !self.is_punct("}") {
            if self.eat_punct("...") { let _ = self.parse_assign()?; if !self.eat_punct(",") { break; } continue; }
            // get/set/async/static/from peuvent être clés d'objet (mots-clés contextuels)
            // Ex: { get foo() {}, set bar(v) {}, async baz() {} }
            if let Tok::Keyword(k) = self.peek().clone() {
                if matches!(k.as_str(), "get"|"set"|"async"|"static"|"from") {
                    let next_is_key = self.toks.get(self.i + 1).map(|t| {
                        matches!(&t.0, Tok::Ident(_) | Tok::Str(_) | Tok::Num(_) | Tok::Keyword(_))
                    }).unwrap_or(false);
                    if next_is_key { self.i += 1; }
                }
            }
            let key = match self.next() {
                Tok::Ident(s) => s, Tok::Keyword(k) => k, Tok::Str(s) => s, Tok::Num(n) => num_to_str(n),
                Tok::Punct(p) if p == "[" => { let _ = self.parse_assign()?; self.expect_punct("]")?; self.expect_punct(":")?; let _ = self.parse_assign()?; if !self.eat_punct(",") { break; } continue; }
                _ => return Err("cle d'objet invalide".into()),
            };
            if self.is_punct("(") { let def = self.parse_func_rest()?; props.push((key, Expr::Func(Rc::new(def)))); }
            else if self.eat_punct(":") { props.push((key, self.parse_assign()?)); }
            // {key = default} : shorthand avec valeur par défaut (pattern destructuring)
            else if self.eat_punct("=") { let _ = self.parse_assign()?; props.push((key.clone(), Expr::Ident(key))); }
            else { props.push((key.clone(), Expr::Ident(key))); }
            if !self.eat_punct(",") { break; }
        }
        self.expect_punct("}")?; Ok(Expr::Object(props))
    }
}

// ============================================================================
// Environnement
// ============================================================================

pub type Env = Rc<RefCell<Scope>>;
pub struct Scope { vars: BTreeMap<String, Value>, parent: Option<Env> }
fn new_scope(parent: Option<Env>) -> Env { Rc::new(RefCell::new(Scope { vars: BTreeMap::new(), parent })) }
fn scope_get(env: &Env, name: &str) -> Option<Value> { let s = env.borrow(); if let Some(v) = s.vars.get(name) { return Some(v.clone()); } if let Some(p) = &s.parent { return scope_get(p, name); } None }
fn scope_set(env: &Env, name: &str, val: Value) -> bool { let mut s = env.borrow_mut(); if s.vars.contains_key(name) { s.vars.insert(name.to_string(), val); return true; } if let Some(p) = s.parent.clone() { drop(s); return scope_set(&p, name, val); } false }
fn scope_declare(env: &Env, name: &str, val: Value) { env.borrow_mut().vars.insert(name.to_string(), val); }
// Cree une portee fille de `outer` en recopiant la valeur courante des `names`
// depuis `from` (pour la liaison `let` par iteration de boucle).
fn copy_scope(outer: &Env, from: &Env, names: &[String]) -> Env {
    let e = new_scope(Some(outer.clone()));
    for n in names { scope_declare(&e, n, scope_get(from, n).unwrap_or(Value::Undefined)); }
    e
}

// ============================================================================
// Interpreteur
// ============================================================================

enum Flow { Normal(Value), Return(Value), Break, Continue, Throw(Value) }

pub struct Interp {
    global: Env,
    steps: u64,
    max_steps: u64,
    pub out: Vec<String>,                 // console.*
    pub writes: String,                   // document.write (script courant)
    pub dom: DomModel,                    // DOM de la page (lecture + mutations)
    wasm: Vec<crate::wasm::Instance>,     // instances WebAssembly vivantes (API JS)
    listeners: Vec<(i64, String, Value)>, // (noeud, type, callback) ; noeud -1 = window/document
    microtasks: Vec<(Value, Vec<Value>)>, // Promise.then / queueMicrotask
    macrotasks: Vec<(Value, Vec<Value>)>, // setTimeout / setInterval (un tour)
    timer_seq: f64,                       // identifiants de timers
    pub base_url: String,                 // URL de base (resolution des <script src>)
}

impl Interp {
    pub fn new() -> Interp {
        let global = new_scope(None);
        let mut it = Interp {
            global: global.clone(), steps: 0, max_steps: 60_000_000,
            out: Vec::new(), writes: String::new(), dom: DomModel::empty(),
            wasm: Vec::new(), listeners: Vec::new(),
            microtasks: Vec::new(), macrotasks: Vec::new(), timer_seq: 1.0,
            base_url: String::new(),
        };
        install(&mut it);
        it
    }

    // Draine les files de taches : microtaches (Promise) en priorite, puis une
    // macrotache (setTimeout) a la fois, jusqu'a epuisement (borne anti-boucle).
    pub fn pump(&mut self) {
        let mut budget = 1_000_000u32;
        loop {
            let next = if !self.microtasks.is_empty() {
                Some(self.microtasks.remove(0))
            } else if !self.macrotasks.is_empty() {
                Some(self.macrotasks.remove(0))
            } else {
                None
            };
            match next {
                Some((cb, args)) => { if budget == 0 { break; } budget -= 1; let _ = self.call(cb, Value::Undefined, &args); }
                None => break,
            }
        }
    }

    // Declenche tous les ecouteurs enregistres pour (node, type), avec un objet
    // `event` minimal. node = -1 cible window/document (load, DOMContentLoaded).
    pub fn fire_event(&mut self, node: i64, ty: &str) {
        let cbs: Vec<Value> = self.listeners.iter().filter(|(n, t, _)| *n == node && t == ty).map(|(_, _, c)| c.clone()).collect();
        if cbs.is_empty() { return; }
        let ev = new_obj(Obj::plain());
        set(&ev, "type", str_val(ty.to_string()));
        if node >= 0 { set(&ev, "target", node_handle(node as usize)); }
        set(&ev, "preventDefault", native_val(|_it, _t, _a| Ok(Value::Undefined)));
        set(&ev, "stopPropagation", native_val(|_it, _t, _a| Ok(Value::Undefined)));
        for cb in cbs {
            let target = if node >= 0 { node_handle(node as usize) } else { Value::Undefined };
            let _ = self.call(cb, target, &[ev.clone()]);
        }
        self.pump();
    }

    fn tick(&mut self) -> Result<(), Value> { self.steps += 1; if self.steps > self.max_steps { return Err(str_val("RangeError: trop d'operations")); } Ok(()) }

    pub fn run(&mut self, src: &str) -> Result<Value, String> {
        let toks = Lexer::new(src.as_bytes()).lex_all()?;
        let prog = Parser::new(toks).parse_program()?;
        let genv = self.global.clone();
        self.hoist(&prog, &genv);
        let mut last = Value::Undefined;
        for st in &prog {
            match self.exec(st, &genv) {
                Flow::Normal(v) => last = v,
                Flow::Throw(v) => {
                    let msg = if let Value::Obj(o) = &v {
                        o.borrow().props.get("message").map(|m| self.to_string(m))
                            .or_else(|| o.borrow().props.get("msg").map(|m| self.to_string(m)))
                            .unwrap_or_else(|| self.to_string(&v))
                    } else { self.to_string(&v) };
                    return Err(format!("Uncaught {}", msg));
                }
                Flow::Return(_) => break,
                _ => {}
            }
        }
        Ok(last)
    }

    fn hoist(&mut self, stmts: &[Stmt], env: &Env) {
        for s in stmts {
            if let Stmt::Func(name, def) = s { let f = new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::User { def: def.clone(), env: env.clone() }), class: "Function" }); scope_declare(env, name, f); }
            if let Stmt::Class(name, ..) = s { scope_declare(env, name, Value::Undefined); }
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt], env: &Env) -> Flow {
        self.hoist(stmts, env);
        for s in stmts { match self.exec(s, env) { Flow::Normal(_) => {}, other => return other } }
        Flow::Normal(Value::Undefined)
    }

    fn exec(&mut self, s: &Stmt, env: &Env) -> Flow {
        if let Err(e) = self.tick() { return Flow::Throw(e); }
        match s {
            Stmt::Empty | Stmt::Func(..) => Flow::Normal(Value::Undefined),
            Stmt::Expr(e) => match self.eval(e, env) { Ok(v) => Flow::Normal(v), Err(t) => Flow::Throw(t) },
            Stmt::Var(_, decls) => { for (name, init) in decls { let v = match init { Some(e) => match self.eval(e, env) { Ok(v) => v, Err(t) => return Flow::Throw(t) }, None => Value::Undefined }; scope_declare(env, name, v); } Flow::Normal(Value::Undefined) }
            Stmt::Block(b) => { let inner = new_scope(Some(env.clone())); self.exec_block(b, &inner) }
            Stmt::Return(e) => { let v = match e { Some(e) => match self.eval(e, env) { Ok(v) => v, Err(t) => return Flow::Throw(t) }, None => Value::Undefined }; Flow::Return(v) }
            Stmt::If(c, t, e) => match self.eval(c, env) { Ok(v) => if truthy(&v) { self.exec(t, env) } else if let Some(e) = e { self.exec(e, env) } else { Flow::Normal(Value::Undefined) }, Err(x) => Flow::Throw(x) },
            Stmt::While(c, body) => { loop { if let Err(e) = self.tick() { return Flow::Throw(e); } match self.eval(c, env) { Ok(v) => if !truthy(&v) { break; }, Err(t) => return Flow::Throw(t) } match self.exec(body, env) { Flow::Break => break, Flow::Continue => {}, Flow::Normal(_) => {}, other => return other } } Flow::Normal(Value::Undefined) }
            Stmt::DoWhile(body, c) => { loop { if let Err(e) = self.tick() { return Flow::Throw(e); } match self.exec(body, env) { Flow::Break => break, Flow::Continue => {}, Flow::Normal(_) => {}, other => return other } match self.eval(c, env) { Ok(v) => if !truthy(&v) { break; }, Err(t) => return Flow::Throw(t) } } Flow::Normal(Value::Undefined) }
            Stmt::For(init, test, update, body) => {
                let inner = new_scope(Some(env.clone()));
                // `let`/`const` dans l'init : liaison fraiche par iteration (les
                // closures creees dans le corps capturent la valeur de leur tour).
                let mut block_names: Vec<String> = Vec::new();
                if let Some(i) = init {
                    if let Stmt::Var(true, decls) = &**i { for (n, _) in decls { block_names.push(n.clone()); } }
                    match self.exec(i, &inner) { Flow::Normal(_) => {}, other => return other }
                }
                let per_iter = !block_names.is_empty();
                let mut cur = if per_iter { copy_scope(env, &inner, &block_names) } else { inner };
                loop {
                    if let Err(e) = self.tick() { return Flow::Throw(e); }
                    if let Some(t) = test { match self.eval(t, &cur) { Ok(v) => if !truthy(&v) { break; }, Err(x) => return Flow::Throw(x) } }
                    match self.exec(body, &cur) { Flow::Break => break, Flow::Continue => {}, Flow::Normal(_) => {}, other => return other }
                    if per_iter { cur = copy_scope(env, &cur, &block_names); }
                    if let Some(u) = update { if let Err(t) = self.eval(u, &cur) { return Flow::Throw(t); } }
                }
                Flow::Normal(Value::Undefined)
            }
            Stmt::ForIn(var, obj, body, is_of) => {
                let o = match self.eval(obj, env) { Ok(v) => v, Err(t) => return Flow::Throw(t) };
                let items: Vec<Value> = if *is_of { self.iterable(&o) } else { self.keys_of(&o) };
                for it in items { if let Err(e) = self.tick() { return Flow::Throw(e); } let inner = new_scope(Some(env.clone())); scope_declare(&inner, var, it); match self.exec(body, &inner) { Flow::Break => break, Flow::Continue => {}, Flow::Normal(_) => {}, other => return other } }
                Flow::Normal(Value::Undefined)
            }
            Stmt::Break => Flow::Break,
            Stmt::Continue => Flow::Continue,
            Stmt::Throw(e) => match self.eval(e, env) { Ok(v) => Flow::Throw(v), Err(t) => Flow::Throw(t) },
            Stmt::Try(tb, catch, fin) => {
                let inner = new_scope(Some(env.clone()));
                let r = self.exec_block(tb, &inner);
                let r = if let Flow::Throw(ex) = r { if let Some((p, cb)) = catch { let c = new_scope(Some(env.clone())); scope_declare(&c, p, ex); self.exec_block(cb, &c) } else { Flow::Throw(ex) } } else { r };
                if let Some(fb) = fin { let f = new_scope(Some(env.clone())); match self.exec_block(fb, &f) { Flow::Normal(_) => r, other => other } } else { r }
            }
            Stmt::Class(name, extends, methods) => {
                let proto = new_obj(Obj::plain());
                // Optionally copy parent prototype methods (extends)
                if let Some(ext_expr) = extends {
                    if let Ok(parent) = self.eval(ext_expr, env) {
                        if let Ok(parent_proto) = self.get_prop(&parent, "prototype") {
                            if let Value::Obj(ref pp) = parent_proto {
                                let keys: Vec<String> = pp.borrow().props.keys().cloned().collect();
                                let vals: Vec<Value> = pp.borrow().props.values().cloned().collect();
                                for (k, v) in keys.into_iter().zip(vals.into_iter()) {
                                    set(&proto, &k, v);
                                }
                            }
                        }
                    }
                }
                let ctor_def = methods.iter().find(|(_, n, _)| n == "constructor")
                    .map(|(_, _, d)| d.clone())
                    .unwrap_or_else(|| Rc::new(FuncDef { params: Vec::new(), rest: None, body: Vec::new(), arrow: false, expr_body: None }));
                for (is_static, mname, def) in methods {
                    if mname == "constructor" { continue; }
                    let mfunc = new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::User { def: def.clone(), env: env.clone() }), class: "Function" });
                    if !is_static { set(&proto, mname, mfunc); }
                }
                let ctor = new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::User { def: ctor_def, env: env.clone() }), class: "Function" });
                set(&ctor, "prototype", proto.clone());
                set(&proto, "constructor", ctor.clone());
                scope_declare(env, name, ctor);
                Flow::Normal(Value::Undefined)
            }
            Stmt::Switch(disc, cases) => {
                let dv = match self.eval(disc, env) { Ok(v) => v, Err(t) => return Flow::Throw(t) };
                let inner = new_scope(Some(env.clone()));
                let mut found = false;
                'sw: for (test, body) in cases {
                    if !found {
                        if let Some(t) = test {
                            match self.eval(t, &inner) {
                                Ok(tv) => { if strict_eq(&dv, &tv) { found = true; } }
                                Err(e) => return Flow::Throw(e),
                            }
                        } else { found = true; } // default
                    }
                    if found {
                        for s in body {
                            match self.exec(s, &inner) {
                                Flow::Break => break 'sw,
                                Flow::Normal(_) | Flow::Continue => {},
                                other => return other,
                            }
                        }
                    }
                }
                Flow::Normal(Value::Undefined) // switch absorbs Break
            }
        }
    }

    fn keys_of(&self, v: &Value) -> Vec<Value> { match v { Value::Obj(o) => { let b = o.borrow(); if let Some(a) = &b.arr { (0..a.len()).map(|i| str_val(i.to_string())).collect() } else { b.props.keys().map(|k| str_val(k.clone())).collect() } } _ => Vec::new() } }
    fn iterable(&self, v: &Value) -> Vec<Value> { match v { Value::Obj(o) => { let b = o.borrow(); if let Some(a) = &b.arr { a.clone() } else { Vec::new() } } Value::Str(s) => s.chars().map(|c| str_val(c.to_string())).collect(), _ => Vec::new() } }

    fn eval(&mut self, e: &Expr, env: &Env) -> Result<Value, Value> {
        self.tick()?;
        match e {
            Expr::Num(n) => Ok(Value::Num(*n)),
            Expr::Str(s) => Ok(str_val(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Null => Ok(Value::Null),
            Expr::Undef => Ok(Value::Undefined),
            Expr::This => Ok(scope_get(env, "this").unwrap_or(Value::Undefined)),
            Expr::Tpl(parts) => { let mut s = String::new(); for p in parts { let v = self.eval(p, env)?; s.push_str(&self.to_string(&v)); } Ok(str_val(s)) }
            Expr::Ident(name) => scope_get(env, name).ok_or_else(|| str_val(format!("ReferenceError: {} is not defined", name))),
            Expr::Array(items) => { let mut out = Vec::new(); for it in items { if let Expr::Spread(inner) = it { let v = self.eval(inner, env)?; out.extend(self.iterable(&v)); } else { out.push(self.eval(it, env)?); } } Ok(array_val(out)) }
            Expr::Object(props) => { let mut o = Obj::plain(); for (k, ve) in props { let v = self.eval(ve, env)?; o.props.insert(k.clone(), v); } Ok(new_obj(o)) }
            Expr::Func(def) => Ok(new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::User { def: def.clone(), env: env.clone() }), class: "Function" })),
            Expr::Spread(e) => self.eval(e, env),
            Expr::Unary(op, x) => self.eval_unary(op, x, env),
            Expr::Update(op, prefix, t) => self.eval_update(op, *prefix, t, env),
            Expr::Bin(op, a, b) => { let l = self.eval(a, env)?; let r = self.eval(b, env)?; self.binop(op, l, r) }
            Expr::Logic(op, a, b) => { let l = self.eval(a, env)?; match op.as_str() { "&&" => if truthy(&l) { self.eval(b, env) } else { Ok(l) }, "||" => if truthy(&l) { Ok(l) } else { self.eval(b, env) }, "??" => if matches!(l, Value::Undefined | Value::Null) { self.eval(b, env) } else { Ok(l) }, _ => Ok(Value::Undefined) } }
            Expr::Cond(c, t, e) => { let cv = self.eval(c, env)?; if truthy(&cv) { self.eval(t, env) } else { self.eval(e, env) } }
            Expr::Assign(op, t, v) => self.eval_assign(op, t, v, env),
            Expr::Seq(list) => { let mut v = Value::Undefined; for e in list { v = self.eval(e, env)?; } Ok(v) }
            Expr::Member(obj, name, opt) => { let o = self.eval(obj, env)?; if *opt && matches!(o, Value::Undefined | Value::Null) { return Ok(Value::Undefined); } self.get_prop(&o, name) }
            Expr::Index(obj, idx) => { let o = self.eval(obj, env)?; let i = self.eval(idx, env)?; let key = self.to_string(&i); self.get_prop(&o, &key) }
            Expr::Call(callee, args, opt) => self.eval_call(callee, args, *opt, env),
            Expr::New(callee, args) => self.eval_new(callee, args, env),
        }
    }

    fn eval_args(&mut self, args: &[Expr], env: &Env) -> Result<Vec<Value>, Value> {
        let mut out = Vec::new();
        for a in args { if let Expr::Spread(inner) = a { let v = self.eval(inner, env)?; out.extend(self.iterable(&v)); } else { out.push(self.eval(a, env)?); } }
        Ok(out)
    }

    fn eval_unary(&mut self, op: &str, x: &Expr, env: &Env) -> Result<Value, Value> {
        if op == "typeof" { let v = self.eval(x, env).unwrap_or(Value::Undefined); return Ok(str_val(type_of(&v))); }
        let v = self.eval(x, env)?;
        Ok(match op { "!" => Value::Bool(!truthy(&v)), "-" => Value::Num(-self.to_num(&v)), "+" => Value::Num(self.to_num(&v)), "~" => Value::Num(!to_i32(self.to_num(&v)) as f64), "void" => Value::Undefined, "delete" => Value::Bool(true), _ => Value::Undefined })
    }

    fn eval_update(&mut self, op: &str, prefix: bool, target: &Expr, env: &Env) -> Result<Value, Value> {
        let cur = self.eval(target, env)?; let old = self.to_num(&cur);
        let new = if op == "++" { old + 1.0 } else { old - 1.0 };
        self.assign_to(target, Value::Num(new), env)?;
        Ok(Value::Num(if prefix { new } else { old }))
    }

    fn eval_assign(&mut self, op: &str, target: &Expr, val: &Expr, env: &Env) -> Result<Value, Value> {
        if op == "=" { let v = self.eval(val, env)?; self.assign_to(target, v.clone(), env)?; return Ok(v); }
        let cur = self.eval(target, env)?; let rhs = self.eval(val, env)?; let base = &op[..op.len() - 1];
        let nv = match base { "&&" => if truthy(&cur) { rhs } else { return Ok(cur) }, "||" => if truthy(&cur) { return Ok(cur); } else { rhs }, "??" => if matches!(cur, Value::Undefined | Value::Null) { rhs } else { return Ok(cur) }, _ => self.binop(base, cur, rhs)? };
        self.assign_to(target, nv.clone(), env)?; Ok(nv)
    }

    fn assign_to(&mut self, target: &Expr, v: Value, env: &Env) -> Result<(), Value> {
        match target {
            Expr::Ident(name) => { if !scope_set(env, name, v.clone()) { scope_declare(&self.global, name, v); } Ok(()) }
            Expr::Member(obj, name, _) => { let o = self.eval(obj, env)?; self.set_prop(&o, name, v); Ok(()) }
            Expr::Index(obj, idx) => { let o = self.eval(obj, env)?; let i = self.eval(idx, env)?; let key = self.to_string(&i); self.set_prop(&o, &key, v); Ok(()) }
            _ => Err(str_val("cible d'affectation invalide")),
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Expr], opt: bool, env: &Env) -> Result<Value, Value> {
        let (func, this) = match callee {
            Expr::Member(obj, name, mopt) => { let o = self.eval(obj, env)?; if (*mopt || opt) && matches!(o, Value::Undefined | Value::Null) { return Ok(Value::Undefined); } let f = self.get_prop(&o, name)?; (f, o) }
            Expr::Index(obj, idx) => { let o = self.eval(obj, env)?; let i = self.eval(idx, env)?; let key = self.to_string(&i); let f = self.get_prop(&o, &key)?; (f, o) }
            _ => (self.eval(callee, env)?, Value::Undefined),
        };
        if opt && matches!(func, Value::Undefined | Value::Null) { return Ok(Value::Undefined); }
        let argv = self.eval_args(args, env)?;
        let r = self.call(func, this, &argv);
        // Enrichit le diagnostic "not a function" avec le nom de l'appelé.
        if let Err(e) = &r {
            if matches!(e, Value::Str(s) if s.as_str() == "TypeError: not a function") {
                let nm = match callee {
                    Expr::Member(_, name, _) => Some(name.clone()),
                    Expr::Ident(name) => Some(name.clone()),
                    _ => None,
                };
                if let Some(nm) = nm {
                    return Err(str_val(format!("TypeError: {} is not a function", nm)));
                }
            }
        }
        r
    }

    pub fn call(&mut self, func: Value, this: Value, args: &[Value]) -> Result<Value, Value> {
        // Fonction liee par Function.prototype.bind : redirige vers la cible
        // avec le `this` et les arguments pre-lies.
        if let Value::Obj(o) = &func {
            let bound = {
                let b = o.borrow();
                b.props.get("__bound_target__").cloned().map(|t| (
                    t,
                    b.props.get("__bound_this__").cloned().unwrap_or(Value::Undefined),
                    b.props.get("__bound_args__").and_then(|v| if let Value::Obj(a) = v { a.borrow().arr.clone() } else { None }).unwrap_or_default(),
                ))
            };
            if let Some((target, bthis, bargs)) = bound {
                let mut all = bargs;
                all.extend_from_slice(args);
                return self.call(target, bthis, &all);
            }
        }
        // Fonction exportee par un module WebAssembly : route vers le runtime wasm.
        if let Value::Obj(o) = &func {
            let bound = { let b = o.borrow(); match (b.props.get("__wasm_inst__"), b.props.get("__wasm_fn__")) {
                (Some(i), Some(Value::Str(n))) => Some((to_num_simple(i) as usize, (**n).clone())), _ => None } };
            if let Some((inst, name)) = bound {
                let argv: Vec<f64> = args.iter().map(|v| self.to_num(v)).collect();
                return match self.wasm.get_mut(inst).map(|w| w.call(&name, &argv)) {
                    Some(Ok(Some(v))) => Ok(Value::Num(v)),
                    Some(Ok(None)) => Ok(Value::Undefined),
                    Some(Err(e)) => Err(str_val(e)),
                    None => Err(str_val("WebAssembly: instance invalide")),
                };
            }
            // resolve/reject d'une promesse (lies a leur promesse via __settle__).
            let settle = { let b = o.borrow(); b.props.get("__settle__").cloned().map(|p| (p, b.props.get("__settle_kind__").map(to_num_simple).unwrap_or(1.0))) };
            if let Some((p, kind)) = settle {
                let val = args.get(0).cloned().unwrap_or(Value::Undefined);
                promise_settle(self, &p, kind, val);
                return Ok(Value::Undefined);
            }
        }
        let (def, fenv, native) = match &func {
            Value::Obj(o) => { let b = o.borrow(); match &b.call { Some(Callable::User { def, env }) => (Some(def.clone()), Some(env.clone()), None), Some(Callable::Native(f)) => (None, None, Some(*f)), None => return Err(str_val("TypeError: not a function")) } }
            _ => return Err(str_val("TypeError: not a function")),
        };
        if let Some(f) = native { return f(self, this, args); }
        let def = def.unwrap(); let fenv = fenv.unwrap();
        let scope = new_scope(Some(fenv));
        if !def.arrow { scope_declare(&scope, "this", this); }
        for (i, p) in def.params.iter().enumerate() { scope_declare(&scope, p, args.get(i).cloned().unwrap_or(Value::Undefined)); }
        if let Some(rest) = &def.rest { let r: Vec<Value> = if args.len() > def.params.len() { args[def.params.len()..].to_vec() } else { Vec::new() }; scope_declare(&scope, rest, array_val(r)); }
        if !def.arrow { scope_declare(&scope, "arguments", array_val(args.to_vec())); }
        if let Some(body) = &def.expr_body { return self.eval(body, &scope); }
        match self.exec_block(&def.body, &scope) { Flow::Return(v) => Ok(v), Flow::Throw(t) => Err(t), _ => Ok(Value::Undefined) }
    }

    fn eval_new(&mut self, callee: &Expr, args: &[Expr], env: &Env) -> Result<Value, Value> {
        let func = self.eval(callee, env)?;
        let argv = self.eval_args(args, env)?;
        // Copy prototype methods onto the fresh instance.
        let proto = if let Value::Obj(ref o) = func { o.borrow().props.get("prototype").cloned() } else { None };
        let this = new_obj(Obj::plain());
        if let Some(Value::Obj(ref pp)) = proto {
            let keys: Vec<String> = pp.borrow().props.keys().cloned().collect();
            let vals: Vec<Value> = pp.borrow().props.values().cloned().collect();
            for (k, v) in keys.into_iter().zip(vals.into_iter()) {
                if k != "constructor" { set(&this, &k, v); }
            }
        }
        let r = self.call(func, this.clone(), &argv)?;
        Ok(if matches!(r, Value::Obj(_)) { r } else { this })
    }

    pub fn get_prop(&mut self, o: &Value, name: &str) -> Result<Value, Value> {
        match o {
            Value::Str(s) => Ok(string_prop(s, name)),
            Value::Num(_) => Ok(number_prop(name)),
            Value::Obj(obj) => {
                let (class, has_arr) = { let b = obj.borrow(); (b.class, b.arr.is_some()) };
                if class == "Element" || class == "Document" { if let Some(v) = dom_get(self, obj, name) { return Ok(v); } }
                if class == "Style" {
                    if name == "cssText" { return Ok(str_val(serialize_style(obj))); }
                    let b = obj.borrow();
                    let key = css_prop_name(name);
                    return Ok(b.props.get(&key).or_else(|| b.props.get(name)).cloned().unwrap_or(Value::Undefined));
                }
                if has_arr {
                    let b = obj.borrow();
                    let arr = b.arr.as_ref().unwrap();
                    if name == "length" { return Ok(Value::Num(arr.len() as f64)); }
                    if let Ok(i) = name.parse::<usize>() { return Ok(arr.get(i).cloned().unwrap_or(Value::Undefined)); }
                    if let Some(v) = b.props.get(name) { return Ok(v.clone()); }
                    drop(b);
                    return Ok(array_prop(name));
                }
                // Function.prototype : call / apply / bind / name / length.
                // Les proprietes propres (fn.foo = ...) restent prioritaires.
                let (is_fn, own, nparams) = {
                    let b = obj.borrow();
                    let np = match &b.call {
                        Some(Callable::User { def, .. }) => def.params.len(),
                        _ => 0,
                    };
                    (b.call.is_some(), b.props.get(name).cloned(), np)
                };
                if let Some(v) = own { return Ok(v); }
                if is_fn {
                    match name {
                        "call" => return Ok(native_val(fn_call)),
                        "apply" => return Ok(native_val(fn_apply)),
                        "bind" => return Ok(native_val(fn_bind)),
                        "length" => return Ok(Value::Num(nparams as f64)),
                        "name" => return Ok(str_val("")),
                        "toString" => return Ok(native_val(|_it, _t, _a| Ok(str_val("function () { [native code] }")))),
                        "prototype" => { let p = new_obj(Obj::plain()); set(o, "prototype", p.clone()); return Ok(p); }
                        _ => {}
                    }
                }
                let b = obj.borrow();
                if let Some(v) = b.props.get(name) { return Ok(v.clone()); }
                drop(b);
                Ok(object_prop(name))
            }
            Value::Null => Err(str_val(format!("TypeError: Cannot read properties of null (reading '{}')", name))),
            Value::Undefined => Err(str_val(format!("TypeError: Cannot read properties of undefined (reading '{}')", name))),
            _ => Ok(Value::Undefined),
        }
    }

    pub fn set_prop(&mut self, o: &Value, name: &str, v: Value) {
        if let Value::Obj(obj) = o {
            let class = { obj.borrow().class };
            if class == "Element" { if dom_set(self, obj, name, &v) { return; } }
            if class == "Style" && !name.starts_with("__") {
                let owner = { obj.borrow().props.get("__owner__").map(|x| to_num_simple(x) as i64).unwrap_or(-1) };
                if name == "cssText" {
                    let css = self.to_string(&v);
                    if owner >= 0 { self.dom.set_attr(owner as usize, "style", css); }
                    obj.borrow_mut().props.insert("cssText".to_string(), v);
                    return;
                }
                obj.borrow_mut().props.insert(css_prop_name(name), v);
                if owner >= 0 { let css = serialize_style(obj); self.dom.set_attr(owner as usize, "style", css); }
                return;
            }
            let mut b = obj.borrow_mut();
            if let Some(arr) = &mut b.arr { if name == "length" { let nl = to_num_simple(&v) as usize; arr.resize(nl.min(1_000_000), Value::Undefined); return; } if let Ok(i) = name.parse::<usize>() { if i < 1_000_000 { if i >= arr.len() { arr.resize(i + 1, Value::Undefined); } arr[i] = v; } return; } }
            b.props.insert(name.to_string(), v);
        }
    }

    fn binop(&mut self, op: &str, l: Value, r: Value) -> Result<Value, Value> {
        Ok(match op {
            "+" => { let ls_str = matches!(l, Value::Str(_)); let rs_str = matches!(r, Value::Str(_)); let lo = matches!(l, Value::Obj(_)); let ro = matches!(r, Value::Obj(_)); if ls_str || rs_str || lo || ro { str_val(format!("{}{}", self.to_string(&l), self.to_string(&r))) } else { Value::Num(self.to_num(&l) + self.to_num(&r)) } }
            "-" => Value::Num(self.to_num(&l) - self.to_num(&r)),
            "*" => Value::Num(self.to_num(&l) * self.to_num(&r)),
            "/" => Value::Num(self.to_num(&l) / self.to_num(&r)),
            "%" => Value::Num(self.to_num(&l) % self.to_num(&r)),
            "**" => Value::Num(powf(self.to_num(&l), self.to_num(&r))),
            "==" => Value::Bool(self.loose_eq(&l, &r)),
            "!=" => Value::Bool(!self.loose_eq(&l, &r)),
            "===" => Value::Bool(strict_eq(&l, &r)),
            "!==" => Value::Bool(!strict_eq(&l, &r)),
            "<" | ">" | "<=" | ">=" => self.compare(op, &l, &r),
            "&" => Value::Num((to_i32(self.to_num(&l)) & to_i32(self.to_num(&r))) as f64),
            "|" => Value::Num((to_i32(self.to_num(&l)) | to_i32(self.to_num(&r))) as f64),
            "^" => Value::Num((to_i32(self.to_num(&l)) ^ to_i32(self.to_num(&r))) as f64),
            "<<" => Value::Num((to_i32(self.to_num(&l)) << (to_i32(self.to_num(&r)) & 31)) as f64),
            ">>" => Value::Num((to_i32(self.to_num(&l)) >> (to_i32(self.to_num(&r)) & 31)) as f64),
            ">>>" => Value::Num(((to_i32(self.to_num(&l)) as u32) >> (to_i32(self.to_num(&r)) & 31)) as f64),
            "instanceof" => Value::Bool(false),
            "in" => { let key = self.to_string(&l); Value::Bool(matches!(&r, Value::Obj(o) if o.borrow().props.contains_key(&key))) }
            _ => Value::Undefined,
        })
    }

    fn compare(&mut self, op: &str, l: &Value, r: &Value) -> Value {
        if let (Value::Str(a), Value::Str(b)) = (l, r) { let c = a.as_str().cmp(b.as_str()); return Value::Bool(match op { "<" => c.is_lt(), ">" => c.is_gt(), "<=" => c.is_le(), _ => c.is_ge() }); }
        let a = self.to_num(l); let b = self.to_num(r); if a.is_nan() || b.is_nan() { return Value::Bool(false); }
        Value::Bool(match op { "<" => a < b, ">" => a > b, "<=" => a <= b, _ => a >= b })
    }

    fn loose_eq(&mut self, l: &Value, r: &Value) -> bool {
        match (l, r) {
            (Value::Null, Value::Undefined) | (Value::Undefined, Value::Null) => true,
            _ => { if matches!(l, Value::Null | Value::Undefined) || matches!(r, Value::Null | Value::Undefined) { return strict_eq(l, r); } if let (Value::Obj(a), Value::Obj(b)) = (l, r) { return Rc::ptr_eq(a, b); } if let (Value::Str(a), Value::Str(b)) = (l, r) { return a == b; } let ln = self.to_num(l); let rn = self.to_num(r); ln == rn && !ln.is_nan() }
        }
    }

    pub fn to_num(&self, v: &Value) -> f64 { to_num_simple(v) }

    pub fn to_string(&self, v: &Value) -> String {
        match v {
            Value::Undefined => "undefined".into(), Value::Null => "null".into(),
            Value::Bool(b) => if *b { "true".into() } else { "false".into() },
            Value::Num(n) => num_to_str(*n), Value::Str(s) => (**s).clone(),
            Value::Obj(o) => { let b = o.borrow(); if let Some(a) = &b.arr { a.iter().map(|x| if matches!(x, Value::Null | Value::Undefined) { String::new() } else { self.to_string(x) }).collect::<Vec<_>>().join(",") } else if b.call.is_some() { "function".into() } else { "[object Object]".into() } }
        }
    }

    fn inspect(&self, v: &Value, depth: u32) -> String {
        match v {
            Value::Str(s) => if depth == 0 { (**s).clone() } else { format!("'{}'", s) },
            Value::Obj(o) => {
                let b = o.borrow();
                if let Some(a) = &b.arr { if a.is_empty() { "[]".into() } else { let inner: Vec<String> = a.iter().map(|x| self.inspect(x, depth + 1)).collect(); format!("[ {} ]", inner.join(", ")) } }
                else if b.call.is_some() { "[Function]".into() }
                else { if b.props.is_empty() { "{}".into() } else { let inner: Vec<String> = b.props.iter().map(|(k, x)| format!("{}: {}", k, self.inspect(x, depth + 1))).collect(); format!("{{ {} }}", inner.join(", ")) } }
            }
            _ => self.to_string(v),
        }
    }
}

// ============================================================================
// Helpers de valeur
// ============================================================================

pub fn truthy(v: &Value) -> bool { match v { Value::Undefined | Value::Null => false, Value::Bool(b) => *b, Value::Num(n) => *n != 0.0 && !n.is_nan(), Value::Str(s) => !s.is_empty(), Value::Obj(_) => true } }
fn type_of(v: &Value) -> &'static str { match v { Value::Undefined => "undefined", Value::Null => "object", Value::Bool(_) => "boolean", Value::Num(_) => "number", Value::Str(_) => "string", Value::Obj(o) => if o.borrow().call.is_some() { "function" } else { "object" } } }
fn strict_eq(l: &Value, r: &Value) -> bool { match (l, r) { (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true, (Value::Bool(a), Value::Bool(b)) => a == b, (Value::Num(a), Value::Num(b)) => a == b, (Value::Str(a), Value::Str(b)) => a == b, (Value::Obj(a), Value::Obj(b)) => Rc::ptr_eq(a, b), _ => false } }
fn to_i32(n: f64) -> i32 { if n.is_nan() || n.is_infinite() { return 0; } (trunc_(n) as i64 as u32) as i32 }
fn to_num_simple(v: &Value) -> f64 { match v { Value::Num(n) => *n, Value::Bool(b) => if *b { 1.0 } else { 0.0 }, Value::Null => 0.0, Value::Undefined => f64::NAN, Value::Str(s) => { let t = s.trim(); if t.is_empty() { 0.0 } else { t.parse::<f64>().unwrap_or(f64::NAN) } } Value::Obj(o) => { let b = o.borrow(); if let Some(a) = &b.arr { if a.is_empty() { 0.0 } else if a.len() == 1 { to_num_simple(&a[0]) } else { f64::NAN } } else { f64::NAN } } } }

pub fn num_to_str(n: f64) -> String { if n.is_nan() { return "NaN".into(); } if n.is_infinite() { return if n > 0.0 { "Infinity".into() } else { "-Infinity".into() }; } if n == 0.0 { return "0".into(); } format!("{}", n) }

fn powf(a: f64, b: f64) -> f64 {
    if b == 0.0 { return 1.0; } if a == 0.0 { return 0.0; }
    if fract_(b) == 0.0 && b.abs() < 1024.0 { let mut r = 1.0f64; let mut e = b.abs() as i64; let mut base = a; while e > 0 { if e & 1 == 1 { r *= base; } base *= base; e >>= 1; } return if b < 0.0 { 1.0 / r } else { r }; }
    if a < 0.0 { return f64::NAN; }
    exp_(b * ln_(a))
}
fn ln_(mut x: f64) -> f64 { if x <= 0.0 { return f64::NAN; } let mut k = 0i32; while x > 1.5 { x /= core::f64::consts::E; k += 1; } while x < 0.5 { x *= core::f64::consts::E; k -= 1; } let t = (x - 1.0) / (x + 1.0); let t2 = t * t; let mut term = t; let mut sum = 0.0; let mut n = 1.0; for _ in 0..30 { sum += term / n; term *= t2; n += 2.0; } 2.0 * sum + k as f64 }
fn exp_(x: f64) -> f64 { let mut term = 1.0; let mut sum = 1.0; for i in 1..40 { term *= x / i as f64; sum += term; } sum }
fn sqrt_(x: f64) -> f64 { if x < 0.0 { return f64::NAN; } if x == 0.0 { return 0.0; } let mut g = x; for _ in 0..40 { g = 0.5 * (g + x / g); } g }
// no_std : pas de f64::floor/ceil/trunc/fract. Implementations maison.
fn trunc_(x: f64) -> f64 { if x.is_nan() || x.is_infinite() { return x; } if x.abs() >= 9.007199254740992e15 { return x; } x as i64 as f64 }
fn floor_(x: f64) -> f64 { let t = trunc_(x); if x < 0.0 && t != x { t - 1.0 } else { t } }
fn ceil_(x: f64) -> f64 { let t = trunc_(x); if x > 0.0 && t != x { t + 1.0 } else { t } }
fn fract_(x: f64) -> f64 { x - trunc_(x) }

// ============================================================================
// Built-ins
// ============================================================================

fn set(obj: &Value, k: &str, v: Value) { if let Value::Obj(o) = obj { o.borrow_mut().props.insert(k.to_string(), v); } }

fn install(it: &mut Interp) {
    let g = it.global.clone();
    // console
    let console = new_obj(Obj::plain());
    for m in ["log", "info", "warn", "error", "debug"] { set(&console, m, native_val(|it, _t, a| { let s = a.iter().map(|v| it.inspect(v, 0)).collect::<Vec<_>>().join(" "); it.out.push(s); Ok(Value::Undefined) })); }
    scope_declare(&g, "console", console);

    // Math
    let math = new_obj(Obj::plain());
    set(&math, "PI", Value::Num(core::f64::consts::PI));
    set(&math, "E", Value::Num(core::f64::consts::E));
    set(&math, "abs", native_val(|it, _t, a| Ok(Value::Num(it.to_num(a.get(0).unwrap_or(&Value::Undefined)).abs()))));
    set(&math, "floor", native_val(|it, _t, a| Ok(Value::Num(floor_(it.to_num(a.get(0).unwrap_or(&Value::Undefined)))))));
    set(&math, "ceil", native_val(|it, _t, a| Ok(Value::Num(ceil_(it.to_num(a.get(0).unwrap_or(&Value::Undefined)))))));
    set(&math, "round", native_val(|it, _t, a| { let x = it.to_num(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Num(floor_(x + 0.5))) }));
    set(&math, "trunc", native_val(|it, _t, a| Ok(Value::Num(trunc_(it.to_num(a.get(0).unwrap_or(&Value::Undefined)))))));
    set(&math, "sqrt", native_val(|it, _t, a| Ok(Value::Num(sqrt_(it.to_num(a.get(0).unwrap_or(&Value::Undefined)))))));
    set(&math, "pow", native_val(|it, _t, a| Ok(Value::Num(powf(it.to_num(a.get(0).unwrap_or(&Value::Undefined)), it.to_num(a.get(1).unwrap_or(&Value::Undefined)))))));
    set(&math, "sign", native_val(|it, _t, a| { let x = it.to_num(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Num(if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { x })) }));
    set(&math, "min", native_val(|it, _t, a| { let mut m = f64::INFINITY; for v in a { let x = it.to_num(v); if x.is_nan() { return Ok(Value::Num(f64::NAN)); } if x < m { m = x; } } Ok(Value::Num(m)) }));
    set(&math, "max", native_val(|it, _t, a| { let mut m = f64::NEG_INFINITY; for v in a { let x = it.to_num(v); if x.is_nan() { return Ok(Value::Num(f64::NAN)); } if x > m { m = x; } } Ok(Value::Num(m)) }));
    set(&math, "log", native_val(|it, _t, a| Ok(Value::Num(ln_(it.to_num(a.get(0).unwrap_or(&Value::Undefined)))))));
    set(&math, "exp", native_val(|it, _t, a| Ok(Value::Num(exp_(it.to_num(a.get(0).unwrap_or(&Value::Undefined)))))));
    set(&math, "random", native_val(|_it, _t, _a| Ok(Value::Num(prng()))));
    // On conserve une référence (même Rc) pour la poser aussi sur `window`,
    // afin que la détection Closure `d.Math == Math` réussisse (cf. window).
    let math_ref = math.clone();
    scope_declare(&g, "Math", math);

    // JSON
    let json = new_obj(Obj::plain());
    set(&json, "stringify", native_val(|it, _t, a| { let s = json_stringify(it, a.get(0).unwrap_or(&Value::Undefined)); Ok(match s { Some(s) => str_val(s), None => Value::Undefined }) }));
    set(&json, "parse", native_val(|_it, _t, a| { let s = match a.get(0) { Some(Value::Str(s)) => (**s).clone(), Some(_) => return Err(str_val("JSON.parse: not a string")), None => return Err(str_val("JSON.parse: undefined")) }; json_parse(&s).ok_or_else(|| str_val("SyntaxError: JSON")) }));
    scope_declare(&g, "JSON", json);

    // Object
    let object = native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or_else(|| new_obj(Obj::plain()))));
    set(&object, "keys", native_val(|_it, _t, a| Ok(match a.get(0) { Some(Value::Obj(o)) => { let b = o.borrow(); if let Some(arr) = &b.arr { array_val((0..arr.len()).map(|i| str_val(i.to_string())).collect()) } else { array_val(b.props.keys().map(|k| str_val(k.clone())).collect()) } } _ => array_val(Vec::new()) })));
    set(&object, "values", native_val(|_it, _t, a| Ok(match a.get(0) { Some(Value::Obj(o)) => { let b = o.borrow(); if let Some(arr) = &b.arr { array_val(arr.clone()) } else { array_val(b.props.values().cloned().collect()) } } _ => array_val(Vec::new()) })));
    set(&object, "entries", native_val(|_it, _t, a| Ok(match a.get(0) { Some(Value::Obj(o)) => { let b = o.borrow(); array_val(b.props.iter().map(|(k, v)| array_val(vec![str_val(k.clone()), v.clone()])).collect()) } _ => array_val(Vec::new()) })));
    set(&object, "assign", native_val(|_it, _t, a| { if let Some(Value::Obj(target)) = a.get(0) { for src in &a[1..] { if let Value::Obj(s) = src { let kv: Vec<(String, Value)> = s.borrow().props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(); for (k, v) in kv { target.borrow_mut().props.insert(k, v); } } } } Ok(a.get(0).cloned().unwrap_or(Value::Undefined)) }));
    set(&object, "freeze", native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or(Value::Undefined))));
    set(&object, "seal", native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or(Value::Undefined))));
    // Object.create(proto, [descriptor]) : crée un objet vide (proto ignoré)
    set(&object, "create", native_val(|_it, _t, _a| Ok(new_obj(Obj::plain()))));
    // Object.defineProperty / defineProperties : stub passthrough
    set(&object, "defineProperty", native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or(Value::Undefined))));
    set(&object, "defineProperties", native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or(Value::Undefined))));
    // Object.getOwnPropertyNames / Descriptor / getPrototypeOf stubs
    set(&object, "getOwnPropertyNames", native_val(|_it, _t, a| Ok(match a.get(0) { Some(Value::Obj(o)) => array_val(o.borrow().props.keys().map(|k| str_val(k.clone())).collect()), _ => array_val(Vec::new()) })));
    set(&object, "getOwnPropertyDescriptor", native_val(|_it, _t, a| {
        if let (Some(Value::Obj(o)), Some(Value::Str(k))) = (a.get(0), a.get(1)) {
            if let Some(v) = o.borrow().props.get(k.as_str()).cloned() {
                let d = new_obj(Obj::plain());
                set(&d, "value", v);
                set(&d, "writable", Value::Bool(true));
                set(&d, "enumerable", Value::Bool(true));
                set(&d, "configurable", Value::Bool(true));
                return Ok(d);
            }
        }
        Ok(Value::Undefined)
    }));
    set(&object, "getOwnPropertyDescriptors", native_val(|_it, _t, a| {
        let o2 = new_obj(Obj::plain());
        if let Some(Value::Obj(o)) = a.get(0) {
            let kv: Vec<(String, Value)> = o.borrow().props.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            for (k, v) in kv {
                let d = new_obj(Obj::plain());
                set(&d, "value", v);
                set(&d, "writable", Value::Bool(true));
                set(&d, "enumerable", Value::Bool(true));
                set(&d, "configurable", Value::Bool(true));
                set(&o2, &k, d);
            }
        }
        Ok(o2)
    }));
    set(&object, "getPrototypeOf", native_val(|_it, _t, _a| Ok(Value::Null)));
    set(&object, "setPrototypeOf", native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or(Value::Undefined))));
    set(&object, "is", native_val(|_it, _t, a| {
        let x = a.get(0).unwrap_or(&Value::Undefined);
        let y = a.get(1).unwrap_or(&Value::Undefined);
        Ok(Value::Bool(strict_eq(x, y)))
    }));
    set(&object, "hasOwn", native_val(|_it, _t, a| Ok(Value::Bool(
        if let (Some(Value::Obj(o)), Some(k)) = (a.get(0), a.get(1)) {
            let ks = if let Value::Str(s) = k { (**s).clone() } else { return Ok(Value::Bool(false)); };
            o.borrow().props.contains_key(ks.as_str())
        } else { false }
    ))));
    set(&object, "fromEntries", native_val(|it, _t, a| {
        let o = new_obj(Obj::plain());
        for entry in it.iterable(a.get(0).unwrap_or(&Value::Undefined)) {
            if let Value::Obj(pair) = &entry {
                let b = pair.borrow();
                if let Some(arr) = &b.arr {
                    let k = arr.get(0).map(|v| it.to_string(v)).unwrap_or_default();
                    let v = arr.get(1).cloned().unwrap_or(Value::Undefined);
                    set(&o, &k, v);
                }
            }
        }
        Ok(o)
    }));
    scope_declare(&g, "Object", object);

    // Array + Array.isArray, Array.from
    let array = native_val(|_it, _t, a| { if a.len() == 1 { if let Value::Num(n) = a[0] { return Ok(array_val(vec![Value::Undefined; (n as usize).min(100000)])); } } Ok(array_val(a.to_vec())) });
    set(&array, "isArray", native_val(|_it, _t, a| Ok(Value::Bool(matches!(a.get(0), Some(Value::Obj(o)) if o.borrow().arr.is_some())))));
    set(&array, "from", native_val(|it, _t, a| {
        let v = a.get(0).cloned().unwrap_or(Value::Undefined);
        // Source : iterable (tableau/chaine) ou array-like ({ length: n }).
        let mut items = it.iterable(&v);
        if items.is_empty() {
            if let Value::Obj(o) = &v {
                let len = { let b = o.borrow(); b.arr.is_none().then(|| b.props.get("length").map(|l| it.to_num(l))).flatten() };
                if let Some(len) = len {
                    let n = (len.max(0.0) as usize).min(1_000_000);
                    items = (0..n).map(|i| it.get_prop(&v, &i.to_string()).unwrap_or(Value::Undefined)).collect();
                }
            }
        }
        if let Some(f) = a.get(1) {
            if matches!(f, Value::Obj(o) if o.borrow().call.is_some()) {
                let mut out = Vec::with_capacity(items.len());
                for (i, x) in items.into_iter().enumerate() { out.push(it.call(f.clone(), Value::Undefined, &[x, Value::Num(i as f64)])?); }
                return Ok(array_val(out));
            }
        }
        Ok(array_val(items))
    }));
    set(&array, "of", native_val(|_it, _t, a| Ok(array_val(a.to_vec()))));
    scope_declare(&g, "Array", array);

    // fonctions globales
    scope_declare(&g, "parseInt", native_val(|it, _t, a| { let s = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); let radix = a.get(1).map(|v| it.to_num(v) as u32).filter(|r| *r >= 2 && *r <= 36).unwrap_or(10); Ok(Value::Num(parse_int(&s, radix))) }));
    scope_declare(&g, "parseFloat", native_val(|it, _t, a| { let s = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Num(parse_float(&s))) }));
    scope_declare(&g, "isNaN", native_val(|it, _t, a| Ok(Value::Bool(it.to_num(a.get(0).unwrap_or(&Value::Undefined)).is_nan()))));
    scope_declare(&g, "isFinite", native_val(|it, _t, a| { let n = it.to_num(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Bool(n.is_finite())) }));
    let string_ctor = native_val(|it, _t, a| Ok(str_val(match a.get(0) { Some(v) => it.to_string(v), None => String::new() })));
    set(&string_ctor, "fromCharCode", native_val(|it, _t, a| { let mut s = String::new(); for v in a { if let Some(c) = core::char::from_u32(it.to_num(v) as u32) { s.push(c); } } Ok(str_val(s)) }));
    scope_declare(&g, "String", string_ctor);
    let number_ctor = native_val(|it, _t, a| Ok(Value::Num(match a.get(0) { Some(v) => it.to_num(v), None => 0.0 })));
    set(&number_ctor, "isInteger", native_val(|it, _t, a| { let n = it.to_num(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Bool(n.is_finite() && fract_(n) == 0.0)) }));
    set(&number_ctor, "isNaN", native_val(|_it, _t, a| Ok(Value::Bool(matches!(a.get(0), Some(Value::Num(n)) if n.is_nan())))));
    set(&number_ctor, "parseFloat", native_val(|it, _t, a| Ok(Value::Num(parse_float(&it.to_string(a.get(0).unwrap_or(&Value::Undefined)))))));
    set(&number_ctor, "MAX_SAFE_INTEGER", Value::Num(9007199254740991.0));
    set(&number_ctor, "MIN_SAFE_INTEGER", Value::Num(-9007199254740991.0));
    scope_declare(&g, "Number", number_ctor);
    scope_declare(&g, "Boolean", native_val(|_it, _t, a| Ok(Value::Bool(a.get(0).map(truthy).unwrap_or(false)))));
    scope_declare(&g, "NaN", Value::Num(f64::NAN));
    scope_declare(&g, "Infinity", Value::Num(f64::INFINITY));
    scope_declare(&g, "undefined", Value::Undefined);
    // stubs sans effet
    scope_declare(&g, "setTimeout", native_val(|it, _t, a| { if let Some(f) = a.get(0) { let f = f.clone(); let _ = it.call(f, Value::Undefined, &[]); } Ok(Value::Num(0.0)) }));
    scope_declare(&g, "alert", native_val(|it, _t, a| { let s = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); it.out.push(format!("[alert] {}", s)); Ok(Value::Undefined) }));

    // document
    let doc = new_obj(Obj { props: OrderedMap::new(), arr: None, call: None, class: "Document" });
    set(&doc, "write", native_val(|it, _t, a| { for v in a { let s = it.to_string(v); it.writes.push_str(&s); } Ok(Value::Undefined) }));
    set(&doc, "writeln", native_val(|it, _t, a| { for v in a { let s = it.to_string(v); it.writes.push_str(&s); } it.writes.push('\n'); Ok(Value::Undefined) }));
    set(&doc, "getElementById", native_val(|it, _t, a| { let id = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(it.dom.find_by_id(&id).map(node_handle).unwrap_or(Value::Null)) }));
    set(&doc, "querySelector", native_val(|it, _t, a| { let sel = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(it.dom.query(&sel, 0, true).first().map(|&n| node_handle(n)).unwrap_or(Value::Null)) }));
    set(&doc, "querySelectorAll", native_val(|it, _t, a| { let sel = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(array_val(it.dom.query(&sel, 0, false).into_iter().map(node_handle).collect())) }));
    set(&doc, "getElementsByTagName", native_val(|it, _t, a| { let t = it.to_string(a.get(0).unwrap_or(&Value::Undefined)).to_lowercase(); Ok(array_val(it.dom.query(&t, 0, false).into_iter().map(node_handle).collect())) }));
    set(&doc, "getElementsByClassName", native_val(|it, _t, a| { let c = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(array_val(it.dom.query(&format!(".{}", c), 0, false).into_iter().map(node_handle).collect())) }));
    set(&doc, "createElement", native_val(|it, _t, a| { let tag = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(detached_element(&tag)) }));
    set(&doc, "createElementNS", native_val(|it, _t, a| { let tag = it.to_string(a.get(1).unwrap_or(&Value::Undefined)); Ok(detached_element(&tag)) }));
    set(&doc, "createTextNode", native_val(|it, _t, a| { let txt = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); let el = detached_element("#text"); set(&el, "__html__", str_val(escape_html(&txt))); Ok(el) }));
    set(&doc, "createComment", native_val(|_it, _t, _a| Ok(detached_element("#comment"))));
    set(&doc, "createDocumentFragment", native_val(|_it, _t, _a| Ok(detached_element("#fragment"))));
    set(&doc, "createRange", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "setStart", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "setEnd",   native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "getBoundingClientRect", native_val(|_i, _t, _a| {
            let r = new_obj(Obj::plain());
            set(&r, "top", Value::Num(0.0)); set(&r, "left", Value::Num(0.0));
            set(&r, "width", Value::Num(0.0)); set(&r, "height", Value::Num(0.0));
            Ok(r)
        }));
        Ok(o)
    }));
    // document.body / document.head stubs
    set(&doc, "body", detached_element("body"));
    set(&doc, "head", detached_element("head"));
    set(&doc, "documentElement", detached_element("html"));
    set(&doc, "title", str_val(""));
    set(&doc, "location", { let loc = new_obj(Obj::plain()); set(&loc, "href", str_val("")); set(&loc, "pathname", str_val("/")); loc });
    set(&doc, "dispatchEvent", native_val(|_it, _t, _a| Ok(Value::Bool(true))));
    set(&doc, "hasFocus", native_val(|_it, _t, _a| Ok(Value::Bool(false))));
    // document.fonts (FontFaceSet) : load() renvoie une promesse resolue.
    set(&doc, "fonts", {
        let fonts = new_obj(Obj::plain());
        set(&fonts, "load", native_val(|_it, _t, _a| Ok(make_resolved_thenable(array_val(Vec::new())))));
        set(&fonts, "ready", make_resolved_thenable(Value::Undefined));
        set(&fonts, "add", native_val(|_it, _t, _a| Ok(Value::Undefined)));
        set(&fonts, "check", native_val(|_it, _t, _a| Ok(Value::Bool(true))));
        set(&fonts, "addEventListener", native_val(|_it, _t, _a| Ok(Value::Undefined)));
        set(&fonts, "status", str_val("loaded"));
        fonts
    });
    // addEventListener reel : enregistre l'ecouteur (cible document = noeud -1).
    set(&doc, "addEventListener", native_val(|it, _t, a| { let ty = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); if let Some(cb) = a.get(1) { it.listeners.push((-1, ty, cb.clone())); } Ok(Value::Undefined) }));
    set(&doc, "removeEventListener", native_val(|it, _t, a| { let ty = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); it.listeners.retain(|(n, t, _)| !(*n == -1 && t == &ty)); Ok(Value::Undefined) }));
    set(&doc, "createEvent", native_val(|_it, _t, _a| Ok(new_obj(Obj::plain()))));
    set(&doc, "readyState", str_val("complete"));
    set(&doc, "cookie", str_val(""));
    let g2 = it.global.clone();
    scope_declare(&g2, "document", doc);

    // window = objet global minimal (alias des globales courantes)
    let window = new_obj(Obj::plain());
    set(&window, "addEventListener", native_val(|it, _t, a| { let ty = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); if let Some(cb) = a.get(1) { it.listeners.push((-1, ty, cb.clone())); } Ok(Value::Undefined) }));
    set(&window, "removeEventListener", native_val(|it, _t, a| { let ty = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); it.listeners.retain(|(n, t, _)| !(*n == -1 && t == &ty)); Ok(Value::Undefined) }));
    set(&window, "setTimeout", native_val(native_set_timeout));
    set(&window, "setInterval", native_val(native_set_timeout));
    set(&window, "clearTimeout", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&window, "clearInterval", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&window, "requestAnimationFrame", native_val(native_set_timeout));
    set(&window, "queueMicrotask", native_val(native_queue_microtask));
    set(&window, "location", { let loc = new_obj(Obj::plain()); set(&loc, "href", str_val("")); set(&loc, "protocol", str_val("https:")); set(&loc, "host", str_val("")); set(&loc, "pathname", str_val("/")); set(&loc, "reload", native_val(|_it, _t, _a| Ok(Value::Undefined))); loc });
    set(&window, "devicePixelRatio", Value::Num(1.0));
    set(&window, "innerWidth", Value::Num(1024.0));
    set(&window, "innerHeight", Value::Num(768.0));
    set(&window, "getComputedStyle", native_val(|_it, _t, _a| { let o = new_obj(Obj::plain()); set(&o, "getPropertyValue", native_val(|_it, _t, _a| Ok(str_val("")))); Ok(o) }));
    set(&window, "matchMedia", native_val(|_it, _t, _a| { let o = new_obj(Obj::plain()); set(&o, "matches", Value::Bool(false)); set(&o, "media", str_val("")); set(&o, "addListener", native_val(|_i, _t, _a| Ok(Value::Undefined))); set(&o, "removeListener", native_val(|_i, _t, _a| Ok(Value::Undefined))); set(&o, "addEventListener", native_val(|_i, _t, _a| Ok(Value::Undefined))); Ok(o) }));
    // `window.Math` doit être le MÊME objet (Rc) que le Math global, sinon la
    // détection Closure-compiler `function(a){...; if(d&&d.Math==Math)return d; throw Error("b")}`
    // appelée en `x(this)` échoue et lève « Uncaught b ».
    set(&window, "Math", math_ref);
    // window référence le document et se référence elle-même (window.window,
    // window.self, window.globalThis, window.top, window.parent, window.frames).
    set(&window, "document", { let d = scope_get(&g2, "document").unwrap_or(Value::Undefined); d });
    set(&window, "window", window.clone());
    set(&window, "self", window.clone());
    set(&window, "globalThis", window.clone());
    set(&window, "top", window.clone());
    set(&window, "parent", window.clone());
    set(&window, "frames", window.clone());
    // Alias du global (self/globalThis/top/parent/frames pointent sur window).
    let win_alias = window.clone();
    scope_declare(&g2, "window", window.clone());
    scope_declare(&g2, "self", win_alias.clone());
    scope_declare(&g2, "globalThis", win_alias.clone());
    scope_declare(&g2, "frames", win_alias.clone());
    scope_declare(&g2, "top", win_alias.clone());
    scope_declare(&g2, "parent", win_alias.clone());
    // `this` au niveau racine d'un script = objet global (window). Permet à
    // `this.gbar_ = {...}` puis la relecture `this.gbar_` de fonctionner, et à
    // `x(this)` de retrouver l'objet global (qui possède `.Math == Math`).
    scope_declare(&g2, "this", win_alias);
    // Image() : constructeur d'image hors-DOM (preload). Stub avec src/onload.
    scope_declare(&g2, "Image", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "src", str_val("")); set(&o, "width", Value::Num(0.0)); set(&o, "height", Value::Num(0.0));
        set(&o, "onload", Value::Null); set(&o, "onerror", Value::Null);
        set(&o, "addEventListener", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        Ok(o)
    }));
    // screen : metriques d'ecran (beaucoup de scripts les lisent au demarrage).
    let screen = new_obj(Obj::plain());
    set(&screen, "width", Value::Num(1280.0)); set(&screen, "height", Value::Num(720.0));
    set(&screen, "availWidth", Value::Num(1280.0)); set(&screen, "availHeight", Value::Num(720.0));
    set(&screen, "colorDepth", Value::Num(24.0)); set(&screen, "pixelDepth", Value::Num(24.0));
    scope_declare(&g2, "screen", screen);
    scope_declare(&g2, "getComputedStyle", native_val(|_it, _t, _a| { let o = new_obj(Obj::plain()); set(&o, "getPropertyValue", native_val(|_it, _t, _a| Ok(str_val("")))); Ok(o) }));
    scope_declare(&g2, "matchMedia", native_val(|_it, _t, _a| { let o = new_obj(Obj::plain()); set(&o, "matches", Value::Bool(false)); set(&o, "addListener", native_val(|_i, _t, _a| Ok(Value::Undefined))); set(&o, "addEventListener", native_val(|_i, _t, _a| Ok(Value::Undefined))); Ok(o) }));
    // navigator minimal
    let nav = new_obj(Obj::plain());
    set(&nav, "userAgent", str_val("BouchaudOS"));
    set(&nav, "language", str_val("fr-FR"));
    set(&nav, "platform", str_val("BouchaudOS"));
    scope_declare(&g2, "navigator", nav);

    // timers + microtaches globaux (remplacent les stubs synchrones)
    scope_declare(&g2, "setTimeout", native_val(native_set_timeout));
    scope_declare(&g2, "setInterval", native_val(native_set_timeout));
    scope_declare(&g2, "clearTimeout", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    scope_declare(&g2, "clearInterval", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    scope_declare(&g2, "requestAnimationFrame", native_val(native_set_timeout));
    scope_declare(&g2, "queueMicrotask", native_val(native_queue_microtask));
    scope_declare(&g2, "encodeURIComponent", native_val(|it, _t, a| Ok(str_val(uri_encode(&it.to_string(a.get(0).unwrap_or(&Value::Undefined)), false)))));
    scope_declare(&g2, "encodeURI", native_val(|it, _t, a| Ok(str_val(uri_encode(&it.to_string(a.get(0).unwrap_or(&Value::Undefined)), true)))));
    scope_declare(&g2, "decodeURIComponent", native_val(|it, _t, a| Ok(str_val(uri_decode(&it.to_string(a.get(0).unwrap_or(&Value::Undefined)))))));
    scope_declare(&g2, "decodeURI", native_val(|it, _t, a| Ok(str_val(uri_decode(&it.to_string(a.get(0).unwrap_or(&Value::Undefined)))))));
    scope_declare(&g2, "structuredClone", native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or(Value::Undefined))));
    // google : namespace global utilise par tous les scripts Google (Search, Maps, Analytics…).
    // Les scripts Google font `window.google = window.google || {}` puis y attachent des sous-
    // namespaces (google.search, google.maps, etc.). On précrée un objet vide avec les
    // sous-objets courants pour que `google.xxx.yyy` ne plante pas même si le script principal
    // (>200KB) est ignoré.
    {
        let g = new_obj(Obj::plain());
        let mk = |name: &str| -> Value {
            let o = new_obj(Obj::plain());
            let _ = name; // évite unused warning
            set(&o, "log", native_val(|_i, _t, _a| Ok(Value::Undefined)));
            o
        };
        for sub in &["search","maps","loader","accounts","ima","ads","adsense","tagmanager","analytics"] {
            set(&g, sub, mk(sub));
        }
        // google.log(), google.tick(), google.ml() — appelés fréquemment
        set(&g, "log", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&g, "tick", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&g, "ml", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&g, "timers", new_obj(Obj::plain()));
        // google.erd : namespace de remontée d'erreurs (error reporting daemon).
        // Les bundles lisent `google.erd.jsr`, `.deb`, `.bv`… au démarrage.
        {
            let erd = new_obj(Obj::plain());
            set(&erd, "jsr", native_val(|_i, _t, _a| Ok(Value::Undefined)));
            set(&erd, "deb", str_val(""));
            set(&erd, "bv", Value::Num(0.0));
            set(&g, "erd", erd);
        }
        set(&g, "kEI", str_val(""));
        set(&g, "kCSI", new_obj(Obj::plain()));
        scope_declare(&g2, "google", g.clone());
        // win_alias est déjà consommé (moved), on réutilise g2 qui est la portée globale
        // La clé window y est accessible via scope_get, pas besoin de set dessus directement.
        let _ = g;
    }
    // Dispatcher interne pour les ecouteurs "click" (voir dom_add_event_listener).
    scope_declare(&g2, "__ael", native_val(|it, _t, a| { let n = it.to_num(a.get(0).unwrap_or(&Value::Undefined)) as i64; it.fire_event(n, "click"); Ok(Value::Undefined) }));

    install_promise(&g2);
    install_date(&g2);
    install_map_set(&g2);

    // WebAssembly : API web standard, branchee sur le runtime wasmi (crate::wasm).
    let webassembly = new_obj(Obj::plain());
    set(&webassembly, "validate", native_val(|_it, _t, a| { let bytes = js_to_bytes(a.get(0)); Ok(Value::Bool(crate::wasm::validate(&bytes))) }));
    set(&webassembly, "instantiate", native_val(|it, _t, a| {
        let bytes = js_to_bytes(a.get(0));
        let instance = wasm_instance_obj(it, &bytes)?;
        let result = new_obj(Obj::plain());
        set(&result, "instance", instance);
        set(&result, "module", new_obj(Obj::plain()));
        Ok(make_resolved_thenable(result))
    }));
    set(&webassembly, "compile", native_val(|_it, _t, a| {
        let bytes = js_to_bytes(a.get(0));
        let m = new_obj(Obj::plain());
        set(&m, "__wasm_bytes__", array_val(bytes.iter().map(|b| Value::Num(*b as f64)).collect()));
        Ok(make_resolved_thenable(m))
    }));
    set(&webassembly, "Module", native_val(|_it, _t, a| { let bytes = js_to_bytes(a.get(0)); let m = new_obj(Obj::plain()); set(&m, "__wasm_bytes__", array_val(bytes.iter().map(|b| Value::Num(*b as f64)).collect())); Ok(m) }));
    set(&webassembly, "Instance", native_val(|it, _t, a| { let bytes = js_to_bytes(a.get(0)); wasm_instance_obj(it, &bytes) }));
    scope_declare(&g2, "WebAssembly", webassembly);

    // Globals fréquemment utilisés par les scripts modernes ----------------
    // `_` : raccourci Google (window._ = window._ || {}). Objet vide pour que
    // `_._DumpException = ...` et `_s._DumpException = _._DumpException` marchent.
    scope_declare(&g2, "_", new_obj(Obj::plain()));
    // `_s` / `_qs` : namespaces sœurs de `_` chez Google (idem).
    scope_declare(&g2, "_s", new_obj(Obj::plain()));
    scope_declare(&g2, "_qs", new_obj(Obj::plain()));
    // performance.now() : temps monotone (stub retourne 0)
    let perf = new_obj(Obj::plain());
    set(&perf, "now", native_val(|_it, _t, _a| Ok(Value::Num(0.0))));
    set(&perf, "mark", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&perf, "measure", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&perf, "getEntriesByName", native_val(|_it, _t, _a| Ok(array_val(Vec::new()))));
    set(&perf, "getEntriesByType", native_val(|_it, _t, _a| Ok(array_val(Vec::new()))));
    set(&perf, "clearMarks", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&perf, "clearMeasures", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    scope_declare(&g2, "performance", perf);
    // fetch : renvoie une Promise résolue avec une Response vide
    scope_declare(&g2, "fetch", native_val(|_it, _t, _a| Ok(make_resolved_thenable(new_obj(Obj::plain())))));
    // XMLHttpRequest stub
    scope_declare(&g2, "XMLHttpRequest", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "open",  native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "send",  native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "setRequestHeader", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "abort", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "addEventListener",    native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "removeEventListener", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "readyState", Value::Num(0.0));
        set(&o, "status", Value::Num(0.0));
        set(&o, "responseText", str_val(""));
        set(&o, "responseURL", str_val(""));
        Ok(o)
    }));
    // URL / URLSearchParams stubs
    scope_declare(&g2, "URL", native_val(|it, _t, a| {
        let o = new_obj(Obj::plain());
        let href = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        set(&o, "href", str_val(href.clone()));
        set(&o, "origin", str_val(""));
        set(&o, "pathname", str_val(""));
        set(&o, "search", str_val(""));
        set(&o, "searchParams", { let sp = new_obj(Obj::plain()); set(&sp, "get", native_val(|_i, _t, _a| Ok(Value::Null))); set(&sp, "set", native_val(|_i, _t, _a| Ok(Value::Undefined))); sp });
        set(&o, "toString", native_val(|_i, _t, _a| Ok(str_val(""))));
        Ok(o)
    }));
    scope_declare(&g2, "URLSearchParams", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "get",    native_val(|_i, _t, _a| Ok(Value::Null)));
        set(&o, "set",    native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "append", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "has",    native_val(|_i, _t, _a| Ok(Value::Bool(false))));
        set(&o, "toString", native_val(|_i, _t, _a| Ok(str_val(""))));
        Ok(o)
    }));
    // Event / CustomEvent stubs
    scope_declare(&g2, "Event", native_val(|it, _t, a| {
        let o = new_obj(Obj::plain());
        set(&o, "type", str_val(it.to_string(a.get(0).unwrap_or(&Value::Undefined))));
        set(&o, "bubbles", Value::Bool(false));
        set(&o, "cancelable", Value::Bool(false));
        set(&o, "preventDefault", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "stopPropagation", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        Ok(o)
    }));
    scope_declare(&g2, "CustomEvent", native_val(|it, _t, a| {
        let o = new_obj(Obj::plain());
        set(&o, "type", str_val(it.to_string(a.get(0).unwrap_or(&Value::Undefined))));
        set(&o, "detail", a.get(1).and_then(|v| if let Value::Obj(o) = v { o.borrow().props.get("detail").cloned() } else { None }).unwrap_or(Value::Undefined));
        set(&o, "preventDefault", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "stopPropagation", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        Ok(o)
    }));
    // MutationObserver stub
    scope_declare(&g2, "MutationObserver", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "observe",    native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "disconnect", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        Ok(o)
    }));
    // IntersectionObserver / ResizeObserver stubs
    scope_declare(&g2, "IntersectionObserver", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "observe",    native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "unobserve",  native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "disconnect", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        Ok(o)
    }));
    scope_declare(&g2, "ResizeObserver", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "observe",    native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "unobserve",  native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "disconnect", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        Ok(o)
    }));
    // requestIdleCallback / cancelIdleCallback
    scope_declare(&g2, "requestIdleCallback", native_val(native_set_timeout));
    scope_declare(&g2, "cancelIdleCallback", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    // setImmediate / clearImmediate (Node.js compat utilisé par certains bundles)
    scope_declare(&g2, "setImmediate", native_val(native_set_timeout));
    scope_declare(&g2, "clearImmediate", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    // Symbol stub (retourne une string unique)
    scope_declare(&g2, "Symbol", native_val(|it, _t, a| {
        let desc = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        Ok(str_val(format!("Symbol({})", desc)))
    }));
    // Proxy stub : retourne la cible inchangée
    scope_declare(&g2, "Proxy", native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or(Value::Undefined))));
    // Reflect stub
    let reflect = new_obj(Obj::plain());
    set(&reflect, "apply",  native_val(|it, _t, a| { if let Some(f) = a.get(0) { let this = a.get(1).cloned().unwrap_or(Value::Undefined); let args = if let Some(Value::Obj(o)) = a.get(2) { o.borrow().arr.clone().unwrap_or_default() } else { Vec::new() }; it.call(f.clone(), this, &args) } else { Ok(Value::Undefined) } }));
    set(&reflect, "has",    native_val(|_it, _t, a| { Ok(Value::Bool(if let Some(Value::Obj(o)) = a.get(0) { if let Some(Value::Str(k)) = a.get(1) { o.borrow().props.contains_key(k.as_str()) } else { false } } else { false })) }));
    set(&reflect, "ownKeys", native_val(|_it, _t, a| { Ok(if let Some(Value::Obj(o)) = a.get(0) { array_val(o.borrow().props.keys().map(|k| str_val(k.clone())).collect()) } else { array_val(Vec::new()) }) }));
    scope_declare(&g2, "Reflect", reflect);
    // Error constructors
    for ctor in &["Error", "TypeError", "RangeError", "ReferenceError", "SyntaxError", "URIError"] {
        scope_declare(&g2, ctor, native_val(|it, _t, a| {
            let o = new_obj(Obj::plain());
            set(&o, "message", str_val(it.to_string(a.get(0).unwrap_or(&Value::Undefined))));
            set(&o, "stack", str_val(""));
            Ok(o)
        }));
    }
    // ArrayBuffer / Uint8Array / Int32Array stubs
    scope_declare(&g2, "ArrayBuffer", native_val(|it, _t, a| {
        let n = (it.to_num(a.get(0).unwrap_or(&Value::Undefined)) as usize).min(64 * 1024 * 1024);
        Ok(array_val(vec![Value::Num(0.0); n]))
    }));
    for tname in &["Uint8Array","Uint8ClampedArray","Int8Array","Uint16Array","Int16Array","Uint32Array","Int32Array","Float32Array","Float64Array","BigUint64Array","BigInt64Array"] {
        scope_declare(&g2, tname, native_val(|it, _t, a| {
            let n = (it.to_num(a.get(0).unwrap_or(&Value::Undefined)) as usize).min(16 * 1024 * 1024);
            Ok(array_val(vec![Value::Num(0.0); n]))
        }));
    }
    // DataView stub
    scope_declare(&g2, "DataView", native_val(|_it, _t, a| {
        let o = new_obj(Obj::plain());
        set(&o, "buffer", a.get(0).cloned().unwrap_or(Value::Undefined));
        set(&o, "getUint8",  native_val(|_i, _t, _a| Ok(Value::Num(0.0))));
        set(&o, "getUint32", native_val(|_i, _t, _a| Ok(Value::Num(0.0))));
        set(&o, "setUint8",  native_val(|_i, _t, _a| Ok(Value::Undefined)));
        set(&o, "setUint32", native_val(|_i, _t, _a| Ok(Value::Undefined)));
        Ok(o)
    }));
    // TextEncoder / TextDecoder stubs
    scope_declare(&g2, "TextEncoder", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "encode", native_val(|it, _t, a| {
            let s = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
            Ok(array_val(s.bytes().map(|b| Value::Num(b as f64)).collect()))
        }));
        Ok(o)
    }));
    scope_declare(&g2, "TextDecoder", native_val(|_it, _t, _a| {
        let o = new_obj(Obj::plain());
        set(&o, "decode", native_val(|_it, _t, a| {
            if let Some(Value::Obj(arr)) = a.get(0) {
                let bytes: Vec<u8> = arr.borrow().arr.as_ref().map(|v| v.iter().map(|x| if let Value::Num(n) = x { *n as u8 } else { 0 }).collect()).unwrap_or_default();
                Ok(str_val(String::from_utf8_lossy(&bytes).into_owned()))
            } else { Ok(str_val("")) }
        }));
        Ok(o)
    }));
    // crypto.getRandomValues stub
    let crypto = new_obj(Obj::plain());
    set(&crypto, "getRandomValues", native_val(|_it, _t, a| Ok(a.get(0).cloned().unwrap_or(Value::Undefined))));
    set(&crypto, "randomUUID", native_val(|_it, _t, _a| Ok(str_val("00000000-0000-0000-0000-000000000000"))));
    scope_declare(&g2, "crypto", crypto);
    // atob / btoa stubs
    scope_declare(&g2, "atob", native_val(|_it, _t, _a| Ok(str_val(""))));
    scope_declare(&g2, "btoa", native_val(|_it, _t, _a| Ok(str_val(""))));
    // localStorage / sessionStorage stubs
    let storage = new_obj(Obj::plain());
    set(&storage, "getItem",    native_val(|_it, _t, _a| Ok(Value::Null)));
    set(&storage, "setItem",    native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&storage, "removeItem", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&storage, "clear",      native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&storage, "length", Value::Num(0.0));
    scope_declare(&g2, "localStorage", storage.clone());
    scope_declare(&g2, "sessionStorage", storage);
    // history stub
    let history = new_obj(Obj::plain());
    set(&history, "pushState",    native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&history, "replaceState", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&history, "back",         native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&history, "forward",      native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&history, "length", Value::Num(1.0));
    scope_declare(&g2, "history", history);
}

// ============================================================================
// Timers, microtaches, URI, Promise, Date, WebAssembly (helpers)
// ============================================================================

// Function.prototype.call(thisArg, ...args). `this` est la fonction cible.
fn fn_call(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let this_arg = a.get(0).cloned().unwrap_or(Value::Undefined);
    let rest = if a.len() > 1 { &a[1..] } else { &[][..] };
    it.call(this, this_arg, rest)
}

// Function.prototype.apply(thisArg, argsArray).
fn fn_apply(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let this_arg = a.get(0).cloned().unwrap_or(Value::Undefined);
    let args: Vec<Value> = match a.get(1) {
        Some(Value::Obj(o)) => o.borrow().arr.clone().unwrap_or_default(),
        _ => Vec::new(),
    };
    it.call(this, this_arg, &args)
}

// Function.prototype.bind(thisArg, ...boundArgs) -> fonction liee. On stocke la
// cible/this/args ; `Interp::call` redirige les objets ainsi marques.
fn fn_bind(_it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let bound = new_obj(Obj::plain());
    // Doit etre appelable (typeof === "function") : trampoline neutre.
    if let Value::Obj(o) = &bound {
        o.borrow_mut().call = Some(Callable::Native(|_it, _t, _a| Ok(Value::Undefined)));
    }
    set(&bound, "__bound_target__", this);
    set(&bound, "__bound_this__", a.get(0).cloned().unwrap_or(Value::Undefined));
    let bargs: Vec<Value> = if a.len() > 1 { a[1..].to_vec() } else { Vec::new() };
    set(&bound, "__bound_args__", array_val(bargs));
    Ok(bound)
}

fn native_set_timeout(it: &mut Interp, _t: Value, a: &[Value]) -> Result<Value, Value> {
    if let Some(cb) = a.get(0) {
        if matches!(cb, Value::Obj(o) if o.borrow().call.is_some()) {
            let extra: Vec<Value> = if a.len() > 2 { a[2..].to_vec() } else { Vec::new() };
            it.macrotasks.push((cb.clone(), extra));
        }
    }
    let id = it.timer_seq; it.timer_seq += 1.0; Ok(Value::Num(id))
}

fn native_queue_microtask(it: &mut Interp, _t: Value, a: &[Value]) -> Result<Value, Value> {
    if let Some(cb) = a.get(0) { it.microtasks.push((cb.clone(), Vec::new())); }
    Ok(Value::Undefined)
}

fn hex_up(n: u8) -> char { core::char::from_digit((n & 15) as u32, 16).unwrap_or('0').to_ascii_uppercase() }
fn uri_encode(s: &str, full: bool) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')')
            || (full && matches!(b, b';' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',' | b'#'));
        if keep { o.push(b as char); } else { o.push('%'); o.push(hex_up(b >> 4)); o.push(hex_up(b)); }
    }
    o
}
fn uri_decode(s: &str) -> String {
    let b = s.as_bytes(); let mut out: Vec<u8> = Vec::new(); let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() { out.push((hexv(b[i + 1]) * 16 + hexv(b[i + 2])) as u8); i += 3; }
        else { out.push(b[i]); i += 1; }
    }
    String::from_utf8(out).unwrap_or_default()
}

// --- Promise ---------------------------------------------------------------
// Etat : __pstate__ (0 pending / 1 fulfilled / 2 rejected), __pvalue__,
// __pcbs__ (file de reactions {f, r, next}). Les reactions sont rejouees comme
// microtaches. Limitation : un handler renvoyant une promesse ne chaine pas.

fn is_promise(v: &Value) -> bool { matches!(v, Value::Obj(o) if o.borrow().props.contains_key("__ispromise__")) }

fn new_promise() -> Value {
    let p = new_obj(Obj::plain());
    set(&p, "__ispromise__", Value::Bool(true));
    set(&p, "__pstate__", Value::Num(0.0));
    set(&p, "__pvalue__", Value::Undefined);
    set(&p, "__pcbs__", array_val(Vec::new()));
    set(&p, "then", native_val(promise_then));
    set(&p, "catch", native_val(promise_catch));
    set(&p, "finally", native_val(promise_finally));
    p
}

fn promise_settle(it: &mut Interp, p: &Value, kind: f64, value: Value) {
    if let Value::Obj(o) = p {
        {
            let mut b = o.borrow_mut();
            let st = b.props.get("__pstate__").map(to_num_simple).unwrap_or(0.0);
            if st != 0.0 { return; }
            b.props.insert("__pstate__".into(), Value::Num(kind));
            b.props.insert("__pvalue__".into(), value.clone());
        }
        let cbs = { o.borrow().props.get("__pcbs__").cloned() };
        if let Some(Value::Obj(arr)) = cbs {
            let reactions: Vec<Value> = arr.borrow().arr.clone().unwrap_or_default();
            for r in reactions { it.microtasks.push((native_val(promise_react), vec![r, value.clone(), Value::Num(kind)])); }
            if let Some(slot) = &mut arr.borrow_mut().arr { slot.clear(); }
        }
    }
}

fn promise_react(it: &mut Interp, _t: Value, a: &[Value]) -> Result<Value, Value> {
    let reaction = a.get(0).cloned().unwrap_or(Value::Undefined);
    let value = a.get(1).cloned().unwrap_or(Value::Undefined);
    let kind = a.get(2).map(to_num_simple).unwrap_or(1.0);
    let (handler, next) = if let Value::Obj(o) = &reaction {
        let b = o.borrow();
        let h = if kind == 1.0 { b.props.get("f").cloned() } else { b.props.get("r").cloned() };
        (h, b.props.get("next").cloned())
    } else { (None, None) };
    let next = next.unwrap_or(Value::Undefined);
    match handler {
        Some(h) if matches!(&h, Value::Obj(o) if o.borrow().call.is_some()) => match it.call(h, Value::Undefined, &[value]) {
            Ok(res) => promise_settle(it, &next, 1.0, res),
            Err(e) => promise_settle(it, &next, 2.0, e),
        },
        _ => promise_settle(it, &next, kind, value), // pas de handler : propage
    }
    Ok(Value::Undefined)
}

fn promise_then(it: &mut Interp, t: Value, a: &[Value]) -> Result<Value, Value> {
    let onf = a.get(0).cloned().unwrap_or(Value::Undefined);
    let onr = a.get(1).cloned().unwrap_or(Value::Undefined);
    let next = new_promise();
    let reaction = new_obj(Obj::plain());
    set(&reaction, "f", onf); set(&reaction, "r", onr); set(&reaction, "next", next.clone());
    if let Value::Obj(o) = &t {
        let (st, val) = { let b = o.borrow(); (b.props.get("__pstate__").map(to_num_simple).unwrap_or(0.0), b.props.get("__pvalue__").cloned().unwrap_or(Value::Undefined)) };
        if st == 0.0 {
            if let Some(Value::Obj(arr)) = o.borrow().props.get("__pcbs__").cloned() { if let Some(slot) = &mut arr.borrow_mut().arr { slot.push(reaction); } }
        } else {
            it.microtasks.push((native_val(promise_react), vec![reaction, val, Value::Num(st)]));
        }
    }
    Ok(next)
}

fn promise_catch(it: &mut Interp, t: Value, a: &[Value]) -> Result<Value, Value> {
    promise_then(it, t, &[Value::Undefined, a.get(0).cloned().unwrap_or(Value::Undefined)])
}

fn promise_finally(it: &mut Interp, t: Value, a: &[Value]) -> Result<Value, Value> {
    if let Some(cb) = a.get(0) { it.microtasks.push((cb.clone(), Vec::new())); }
    Ok(t)
}

fn promise_settle_stub(_it: &mut Interp, _t: Value, _a: &[Value]) -> Result<Value, Value> { Ok(Value::Undefined) }

fn native_promise_ctor(it: &mut Interp, _this: Value, a: &[Value]) -> Result<Value, Value> {
    let p = new_promise();
    let resolve = new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::Native(promise_settle_stub)), class: "Function" });
    set(&resolve, "__settle__", p.clone()); set(&resolve, "__settle_kind__", Value::Num(1.0));
    let reject = new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::Native(promise_settle_stub)), class: "Function" });
    set(&reject, "__settle__", p.clone()); set(&reject, "__settle_kind__", Value::Num(2.0));
    if let Some(ex) = a.get(0) {
        if let Err(e) = it.call(ex.clone(), Value::Undefined, &[resolve, reject]) { promise_settle(it, &p, 2.0, e); }
    }
    Ok(p)
}

fn promise_all(it: &mut Interp, _t: Value, a: &[Value]) -> Result<Value, Value> {
    it.pump(); // tente de regler les promesses en attente
    let items = it.iterable(a.get(0).unwrap_or(&Value::Undefined));
    let mut out = Vec::new();
    for v in items {
        if is_promise(&v) { if let Value::Obj(o) = &v { out.push(o.borrow().props.get("__pvalue__").cloned().unwrap_or(Value::Undefined)); } }
        else { out.push(v); }
    }
    let p = new_promise(); promise_settle(it, &p, 1.0, array_val(out)); Ok(p)
}

fn install_promise(g: &Env) {
    let promise = new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::Native(native_promise_ctor)), class: "Function" });
    set(&promise, "resolve", native_val(|it, _t, a| { let p = new_promise(); promise_settle(it, &p, 1.0, a.get(0).cloned().unwrap_or(Value::Undefined)); Ok(p) }));
    set(&promise, "reject", native_val(|it, _t, a| { let p = new_promise(); promise_settle(it, &p, 2.0, a.get(0).cloned().unwrap_or(Value::Undefined)); Ok(p) }));
    set(&promise, "all", native_val(promise_all));
    set(&promise, "race", native_val(promise_all));
    set(&promise, "allSettled", native_val(promise_all));
    scope_declare(g, "Promise", promise);
}

// --- Date ------------------------------------------------------------------
fn date_now_ms() -> f64 { (crate::kernel::timer::seconds() as f64) * 1000.0 }
fn two(n: u32) -> String { if n < 10 { format!("0{}", n) } else { format!("{}", n) } }
fn make_date_obj() -> Value {
    let dt = crate::arch::x86_64::rtc::now();
    let o = new_obj(Obj::plain());
    set(&o, "__y__", Value::Num(dt.year as f64));
    set(&o, "__mo__", Value::Num((dt.month.saturating_sub(1)) as f64));
    set(&o, "__d__", Value::Num(dt.day as f64));
    set(&o, "__h__", Value::Num(dt.hour as f64));
    set(&o, "__mi__", Value::Num(dt.minute as f64));
    set(&o, "__s__", Value::Num(dt.second as f64));
    let iso = format!("{}-{}-{}T{}:{}:{}Z", dt.year, two(dt.month as u32), two(dt.day as u32), two(dt.hour as u32), two(dt.minute as u32), two(dt.second as u32));
    set(&o, "__iso__", str_val(iso));
    set(&o, "getFullYear", native_val(|it, t, _a| it.get_prop(&t, "__y__")));
    set(&o, "getMonth", native_val(|it, t, _a| it.get_prop(&t, "__mo__")));
    set(&o, "getDate", native_val(|it, t, _a| it.get_prop(&t, "__d__")));
    set(&o, "getDay", native_val(|_it, _t, _a| Ok(Value::Num(0.0))));
    set(&o, "getHours", native_val(|it, t, _a| it.get_prop(&t, "__h__")));
    set(&o, "getMinutes", native_val(|it, t, _a| it.get_prop(&t, "__mi__")));
    set(&o, "getSeconds", native_val(|it, t, _a| it.get_prop(&t, "__s__")));
    set(&o, "getMilliseconds", native_val(|_it, _t, _a| Ok(Value::Num(0.0))));
    set(&o, "getTime", native_val(|_it, _t, _a| Ok(Value::Num(date_now_ms()))));
    set(&o, "valueOf", native_val(|_it, _t, _a| Ok(Value::Num(date_now_ms()))));
    set(&o, "toISOString", native_val(|it, t, _a| it.get_prop(&t, "__iso__")));
    set(&o, "toJSON", native_val(|it, t, _a| it.get_prop(&t, "__iso__")));
    set(&o, "toString", native_val(|it, t, _a| it.get_prop(&t, "__iso__")));
    o
}
fn install_date(g: &Env) {
    let date = native_val(|_it, _t, _a| Ok(make_date_obj()));
    set(&date, "now", native_val(|_it, _t, _a| Ok(Value::Num(date_now_ms()))));
    set(&date, "parse", native_val(|_it, _t, _a| Ok(Value::Num(date_now_ms()))));
    set(&date, "UTC", native_val(|_it, _t, _a| Ok(Value::Num(date_now_ms()))));
    scope_declare(g, "Date", date);
}

fn make_map_obj(items: Vec<(Value, Value)>) -> Value {
    let o = new_obj(Obj { props: OrderedMap::new(), arr: Some(Vec::new()), call: None, class: "Map" });
    // store as parallel arrays in __keys__ / __vals__ props
    let keys: Vec<Value> = items.iter().map(|(k, _)| k.clone()).collect();
    let vals: Vec<Value> = items.iter().map(|(_, v)| v.clone()).collect();
    set(&o, "__keys__", array_val(keys));
    set(&o, "__vals__", array_val(vals));
    set(&o, "size", Value::Num(items.len() as f64));
    o
}

fn map_get_kv(o: &Value) -> (Vec<Value>, Vec<Value>) {
    let keys = if let Value::Obj(ob) = o { ob.borrow().props.get("__keys__").and_then(|v| if let Value::Obj(a) = v { a.borrow().arr.clone() } else { None }).unwrap_or_default() } else { Vec::new() };
    let vals = if let Value::Obj(ob) = o { ob.borrow().props.get("__vals__").and_then(|v| if let Value::Obj(a) = v { a.borrow().arr.clone() } else { None }).unwrap_or_default() } else { Vec::new() };
    (keys, vals)
}

fn install_map_set(g: &Env) {
    // Map
    let map_ctor = native_val(|it, _t, a| {
        let mut items: Vec<(Value, Value)> = Vec::new();
        if let Some(init) = a.get(0) {
            for pair in it.iterable(init) {
                let kv = it.iterable(&pair);
                let k = kv.get(0).cloned().unwrap_or(Value::Undefined);
                let v = kv.get(1).cloned().unwrap_or(Value::Undefined);
                items.push((k, v));
            }
        }
        let m = make_map_obj(items);
        set(&m, "get", native_val(|_it, t, a| {
            let (keys, vals) = map_get_kv(&t);
            let key = a.get(0).cloned().unwrap_or(Value::Undefined);
            for (i, k) in keys.iter().enumerate() { if strict_eq(k, &key) { return Ok(vals.get(i).cloned().unwrap_or(Value::Undefined)); } }
            Ok(Value::Undefined)
        }));
        set(&m, "set", native_val(|_it, t, a| {
            let key = a.get(0).cloned().unwrap_or(Value::Undefined);
            let val = a.get(1).cloned().unwrap_or(Value::Undefined);
            if let Value::Obj(ob) = &t {
                let keys_v = ob.borrow().props.get("__keys__").cloned();
                let vals_v = ob.borrow().props.get("__vals__").cloned();
                if let (Some(Value::Obj(kobj)), Some(Value::Obj(vobj))) = (keys_v, vals_v) {
                    let pos = kobj.borrow().arr.as_ref().map(|a| a.iter().position(|k| strict_eq(k, &key))).flatten();
                    if let Some(i) = pos {
                        if let Some(arr) = &mut vobj.borrow_mut().arr { arr[i] = val; }
                    } else {
                        let len = kobj.borrow().arr.as_ref().map(|a| a.len()).unwrap_or(0);
                        if let Some(arr) = &mut kobj.borrow_mut().arr { arr.push(key); }
                        if let Some(arr) = &mut vobj.borrow_mut().arr { arr.push(val); }
                        ob.borrow_mut().props.insert("size".to_string(), Value::Num((len + 1) as f64));
                    }
                }
            }
            Ok(t)
        }));
        set(&m, "has", native_val(|_it, t, a| {
            let (keys, _) = map_get_kv(&t);
            let key = a.get(0).cloned().unwrap_or(Value::Undefined);
            Ok(Value::Bool(keys.iter().any(|k| strict_eq(k, &key))))
        }));
        set(&m, "delete", native_val(|_it, t, a| {
            let key = a.get(0).cloned().unwrap_or(Value::Undefined);
            if let Value::Obj(ob) = &t {
                let keys_v = ob.borrow().props.get("__keys__").cloned();
                let vals_v = ob.borrow().props.get("__vals__").cloned();
                if let (Some(Value::Obj(kobj)), Some(Value::Obj(vobj))) = (keys_v, vals_v) {
                    let pos = kobj.borrow().arr.as_ref().map(|a| a.iter().position(|k| strict_eq(k, &key))).flatten();
                    if let Some(i) = pos {
                        if let Some(arr) = &mut kobj.borrow_mut().arr { arr.remove(i); }
                        if let Some(arr) = &mut vobj.borrow_mut().arr { arr.remove(i); }
                        let len = kobj.borrow().arr.as_ref().map(|a| a.len()).unwrap_or(0);
                        ob.borrow_mut().props.insert("size".to_string(), Value::Num(len as f64));
                        return Ok(Value::Bool(true));
                    }
                }
            }
            Ok(Value::Bool(false))
        }));
        set(&m, "clear", native_val(|_it, t, _a| {
            if let Value::Obj(ob) = &t {
                let keys_v = ob.borrow().props.get("__keys__").cloned();
                let vals_v = ob.borrow().props.get("__vals__").cloned();
                if let Some(Value::Obj(k)) = keys_v { k.borrow_mut().arr = Some(Vec::new()); }
                if let Some(Value::Obj(v)) = vals_v { v.borrow_mut().arr = Some(Vec::new()); }
                ob.borrow_mut().props.insert("size".to_string(), Value::Num(0.0));
            }
            Ok(Value::Undefined)
        }));
        set(&m, "forEach", native_val(|it, t, a| {
            let (keys, vals) = map_get_kv(&t);
            let f = a.get(0).cloned().unwrap_or(Value::Undefined);
            for (k, v) in keys.iter().zip(vals.iter()) { it.call(f.clone(), Value::Undefined, &[v.clone(), k.clone(), t.clone()])?; }
            Ok(Value::Undefined)
        }));
        set(&m, "keys", native_val(|_it, t, _a| { let (keys, _) = map_get_kv(&t); Ok(array_val(keys)) }));
        set(&m, "values", native_val(|_it, t, _a| { let (_, vals) = map_get_kv(&t); Ok(array_val(vals)) }));
        set(&m, "entries", native_val(|_it, t, _a| { let (keys, vals) = map_get_kv(&t); Ok(array_val(keys.into_iter().zip(vals.into_iter()).map(|(k, v)| array_val(vec![k, v])).collect())) }));
        Ok(m)
    });
    scope_declare(g, "Map", map_ctor);

    // Set
    let set_ctor = native_val(|it, _t, a| {
        let mut items: Vec<Value> = Vec::new();
        if let Some(init) = a.get(0) {
            for v in it.iterable(init) {
                if !items.iter().any(|x| strict_eq(x, &v)) { items.push(v); }
            }
        }
        let s = new_obj(Obj { props: OrderedMap::new(), arr: Some(items.clone()), call: None, class: "Set" });
        set(&s, "size", Value::Num(items.len() as f64));
        set(&s, "has", native_val(|_it, t, a| {
            let key = a.get(0).cloned().unwrap_or(Value::Undefined);
            let items = if let Value::Obj(o) = &t { o.borrow().arr.clone().unwrap_or_default() } else { Vec::new() };
            Ok(Value::Bool(items.iter().any(|v| strict_eq(v, &key))))
        }));
        set(&s, "add", native_val(|_it, t, a| {
            let key = a.get(0).cloned().unwrap_or(Value::Undefined);
            if let Value::Obj(o) = &t {
                let has = o.borrow().arr.as_ref().map(|a| a.iter().any(|v| strict_eq(v, &key))).unwrap_or(false);
                if !has {
                    let len = o.borrow().arr.as_ref().map(|a| a.len()).unwrap_or(0);
                    if let Some(arr) = &mut o.borrow_mut().arr { arr.push(key); }
                    o.borrow_mut().props.insert("size".to_string(), Value::Num((len + 1) as f64));
                }
            }
            Ok(t)
        }));
        set(&s, "delete", native_val(|_it, t, a| {
            let key = a.get(0).cloned().unwrap_or(Value::Undefined);
            if let Value::Obj(o) = &t {
                let pos = o.borrow().arr.as_ref().map(|a| a.iter().position(|v| strict_eq(v, &key))).flatten();
                if let Some(i) = pos {
                    if let Some(arr) = &mut o.borrow_mut().arr { arr.remove(i); }
                    let len = o.borrow().arr.as_ref().map(|a| a.len()).unwrap_or(0);
                    o.borrow_mut().props.insert("size".to_string(), Value::Num(len as f64));
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }));
        set(&s, "clear", native_val(|_it, t, _a| {
            if let Value::Obj(o) = &t { o.borrow_mut().arr = Some(Vec::new()); o.borrow_mut().props.insert("size".to_string(), Value::Num(0.0)); }
            Ok(Value::Undefined)
        }));
        set(&s, "forEach", native_val(|it, t, a| {
            let items = if let Value::Obj(o) = &t { o.borrow().arr.clone().unwrap_or_default() } else { Vec::new() };
            let f = a.get(0).cloned().unwrap_or(Value::Undefined);
            for v in &items { it.call(f.clone(), Value::Undefined, &[v.clone(), v.clone(), t.clone()])?; }
            Ok(Value::Undefined)
        }));
        set(&s, "values", native_val(|_it, t, _a| {
            let items = if let Value::Obj(o) = &t { o.borrow().arr.clone().unwrap_or_default() } else { Vec::new() };
            Ok(array_val(items))
        }));
        set(&s, "keys", native_val(|_it, t, _a| {
            let items = if let Value::Obj(o) = &t { o.borrow().arr.clone().unwrap_or_default() } else { Vec::new() };
            Ok(array_val(items))
        }));
        set(&s, "entries", native_val(|_it, t, _a| {
            let items = if let Value::Obj(o) = &t { o.borrow().arr.clone().unwrap_or_default() } else { Vec::new() };
            Ok(array_val(items.iter().map(|v| array_val(vec![v.clone(), v.clone()])).collect()))
        }));
        Ok(s)
    });
    scope_declare(g, "Set", set_ctor);
}

// --- WebAssembly (helpers) -------------------------------------------------
fn js_to_bytes(v: Option<&Value>) -> Vec<u8> {
    match v {
        Some(Value::Obj(o)) => {
            let b = o.borrow();
            if let Some(arr) = &b.arr {
                arr.iter().map(|x| to_num_simple(x) as i64 as u8).collect()
            } else if let Some(Value::Obj(inner)) = b.props.get("__wasm_bytes__") {
                inner.borrow().arr.as_ref().map(|a| a.iter().map(|x| to_num_simple(x) as i64 as u8).collect()).unwrap_or_default()
            } else { Vec::new() }
        }
        _ => Vec::new(),
    }
}

fn wasm_export_stub(_it: &mut Interp, _t: Value, _a: &[Value]) -> Result<Value, Value> { Ok(Value::Undefined) }

fn wasm_instance_obj(it: &mut Interp, bytes: &[u8]) -> Result<Value, Value> {
    match crate::wasm::instantiate(bytes) {
        Ok(inst) => {
            let idx = it.wasm.len();
            it.wasm.push(inst);
            let names: Vec<String> = it.wasm[idx].export_funcs().to_vec();
            let exports = new_obj(Obj::plain());
            for name in names {
                let f = new_obj(Obj { props: OrderedMap::new(), arr: None, call: Some(Callable::Native(wasm_export_stub)), class: "Function" });
                set(&f, "__wasm_inst__", Value::Num(idx as f64));
                set(&f, "__wasm_fn__", str_val(name.clone()));
                set(&exports, &name, f);
            }
            let instance = new_obj(Obj::plain());
            set(&instance, "exports", exports);
            Ok(instance)
        }
        Err(e) => Err(str_val(format!("WebAssembly: {}", e))),
    }
}

// Objet "thenable" resolu immediatement : permet `WebAssembly.instantiate(b).then(cb)`
// sans machinerie Promise complete.
fn make_resolved_thenable(v: Value) -> Value {
    if matches!(&v, Value::Obj(_)) {
        set(&v, "then", native_val(|it, t, a| { if let Some(cb) = a.get(0) { let r = it.call(cb.clone(), Value::Undefined, &[t.clone()])?; return Ok(make_resolved_thenable(r)); } Ok(t) }));
        set(&v, "catch", native_val(|_it, t, _a| Ok(t)));
    }
    v
}

// PRNG simple (LCG) pour Math.random (deterministe ; suffisant pour le rendu).
fn prng() -> f64 { use core::sync::atomic::{AtomicU64, Ordering}; static S: AtomicU64 = AtomicU64::new(0x2545F4914F6CDD1D); let mut x = S.load(Ordering::Relaxed); x ^= x << 13; x ^= x >> 7; x ^= x << 17; S.store(x, Ordering::Relaxed); ((x >> 11) as f64) / ((1u64 << 53) as f64) }

fn parse_int(s: &str, radix: u32) -> f64 {
    let s = s.trim(); let bytes = s.as_bytes(); let mut i = 0; let mut sign = 1.0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') { if bytes[i] == b'-' { sign = -1.0; } i += 1; }
    if radix == 16 && s[i..].starts_with("0x") { i += 2; }
    let mut val = 0.0f64; let mut any = false;
    while i < bytes.len() { let d = (bytes[i] as char).to_digit(36); match d { Some(d) if d < radix => { val = val * radix as f64 + d as f64; any = true; i += 1; } _ => break } }
    if any { sign * val } else { f64::NAN }
}
fn parse_float(s: &str) -> f64 {
    let s = s.trim(); let b = s.as_bytes(); let mut seen_dot = false; let mut seen_e = false; let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') { i += 1; }
    while i < b.len() { let c = b[i]; if c.is_ascii_digit() { } else if c == b'.' && !seen_dot && !seen_e { seen_dot = true; } else if (c | 32) == b'e' && !seen_e { seen_e = true; if i + 1 < b.len() && (b[i + 1] == b'+' || b[i + 1] == b'-') { i += 1; } } else { break; } i += 1; }
    s[..i].parse::<f64>().unwrap_or(f64::NAN)
}

// --- methodes String (this = chaine) ---
fn string_prop(s: &str, name: &str) -> Value {
    match name {
        "length" => Value::Num(s.chars().count() as f64),
        "toUpperCase" => native_val(|it, t, _a| Ok(str_val(it.to_string(&t).to_uppercase()))),
        "toLowerCase" => native_val(|it, t, _a| Ok(str_val(it.to_string(&t).to_lowercase()))),
        "trim" => native_val(|it, t, _a| Ok(str_val(it.to_string(&t).trim().to_string()))),
        "charAt" => native_val(|it, t, a| { let s = it.to_string(&t); let i = it.to_num(a.get(0).unwrap_or(&Value::Num(0.0))) as usize; Ok(str_val(s.chars().nth(i).map(|c| c.to_string()).unwrap_or_default())) }),
        "charCodeAt" => native_val(|it, t, a| { let s = it.to_string(&t); let i = it.to_num(a.get(0).unwrap_or(&Value::Num(0.0))) as usize; Ok(s.chars().nth(i).map(|c| Value::Num(c as u32 as f64)).unwrap_or(Value::Num(f64::NAN))) }),
        "indexOf" => native_val(|it, t, a| { let s = it.to_string(&t); let n = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Num(byte_to_char_index(&s, s.find(&n)))) }),
        "lastIndexOf" => native_val(|it, t, a| { let s = it.to_string(&t); let n = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Num(byte_to_char_index(&s, s.rfind(&n)))) }),
        "includes" => native_val(|it, t, a| { let s = it.to_string(&t); let n = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Bool(s.contains(&n))) }),
        "startsWith" => native_val(|it, t, a| { let s = it.to_string(&t); let n = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Bool(s.starts_with(&n))) }),
        "endsWith" => native_val(|it, t, a| { let s = it.to_string(&t); let n = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Bool(s.ends_with(&n))) }),
        "slice" => native_val(|it, t, a| { let s: Vec<char> = it.to_string(&t).chars().collect(); let (st, en) = slice_bounds(s.len(), a, it); Ok(str_val(s[st..en].iter().collect::<String>())) }),
        "substring" => native_val(|it, t, a| { let s: Vec<char> = it.to_string(&t).chars().collect(); let mut st = it.to_num(a.get(0).unwrap_or(&Value::Num(0.0))).max(0.0) as usize; let mut en = a.get(1).map(|v| it.to_num(v) as usize).unwrap_or(s.len()).min(s.len()); st = st.min(s.len()); en = en.min(s.len()); if st > en { core::mem::swap(&mut st, &mut en); } Ok(str_val(s[st..en].iter().collect::<String>())) }),
        "substr" => native_val(|it, t, a| { let s: Vec<char> = it.to_string(&t).chars().collect(); let st = (it.to_num(a.get(0).unwrap_or(&Value::Num(0.0))).max(0.0) as usize).min(s.len()); let len = a.get(1).map(|v| it.to_num(v) as usize).unwrap_or(s.len() - st).min(s.len() - st); Ok(str_val(s[st..st + len].iter().collect::<String>())) }),
        "split" => native_val(|it, t, a| { let s = it.to_string(&t); match a.get(0) { None | Some(Value::Undefined) => Ok(array_val(vec![str_val(s)])), Some(sep) => { let sep = it.to_string(sep); if sep.is_empty() { Ok(array_val(s.chars().map(|c| str_val(c.to_string())).collect())) } else { Ok(array_val(s.split(&sep as &str).map(|p| str_val(p.to_string())).collect())) } } } }),
        "replace" => native_val(|it, t, a| { let s = it.to_string(&t); let from = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); let to = it.to_string(a.get(1).unwrap_or(&Value::Undefined)); Ok(str_val(s.replacen(&from as &str, &to, 1))) }),
        "replaceAll" => native_val(|it, t, a| { let s = it.to_string(&t); let from = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); let to = it.to_string(a.get(1).unwrap_or(&Value::Undefined)); Ok(str_val(if from.is_empty() { s } else { s.replace(&from as &str, &to) })) }),
        "repeat" => native_val(|it, t, a| { let s = it.to_string(&t); let n = it.to_num(a.get(0).unwrap_or(&Value::Num(0.0))) as usize; Ok(str_val(s.repeat(n.min(100000)))) }),
        "padStart" => native_val(|it, t, a| { let s = it.to_string(&t); let len = it.to_num(a.get(0).unwrap_or(&Value::Num(0.0))) as usize; let pad = a.get(1).map(|v| it.to_string(v)).unwrap_or_else(|| " ".into()); Ok(str_val(pad_str(&s, len, &pad, true))) }),
        "padEnd" => native_val(|it, t, a| { let s = it.to_string(&t); let len = it.to_num(a.get(0).unwrap_or(&Value::Num(0.0))) as usize; let pad = a.get(1).map(|v| it.to_string(v)).unwrap_or_else(|| " ".into()); Ok(str_val(pad_str(&s, len, &pad, false))) }),
        "concat" => native_val(|it, t, a| { let mut s = it.to_string(&t); for v in a { s.push_str(&it.to_string(v)); } Ok(str_val(s)) }),
        "toString" => native_val(|it, t, _a| Ok(str_val(it.to_string(&t)))),
        "trimStart" => native_val(|it, t, _a| Ok(str_val(it.to_string(&t).trim_start().to_string()))),
        "trimEnd" => native_val(|it, t, _a| Ok(str_val(it.to_string(&t).trim_end().to_string()))),
        _ => { if let Ok(i) = name.parse::<usize>() { return s.chars().nth(i).map(|c| str_val(c.to_string())).unwrap_or(Value::Undefined); } Value::Undefined }
    }
}
fn byte_to_char_index(s: &str, b: Option<usize>) -> f64 { match b { Some(bi) => s[..bi].chars().count() as f64, None => -1.0 } }
fn pad_str(s: &str, len: usize, pad: &str, start: bool) -> String { let cur = s.chars().count(); if cur >= len || pad.is_empty() { return s.to_string(); } let mut p = String::new(); while cur + p.chars().count() < len { p.push_str(pad); } let p: String = p.chars().take(len - cur).collect(); if start { format!("{}{}", p, s) } else { format!("{}{}", s, p) } }
fn slice_bounds(len: usize, a: &[Value], it: &Interp) -> (usize, usize) {
    let norm = |v: f64, len: usize| -> usize { if v < 0.0 { ((len as f64 + v).max(0.0)) as usize } else { (v as usize).min(len) } };
    let st = a.get(0).map(|v| norm(it.to_num(v), len)).unwrap_or(0);
    let en = a.get(1).map(|v| if matches!(v, Value::Undefined) { len } else { norm(it.to_num(v), len) }).unwrap_or(len);
    (st.min(en), en.max(st))
}

fn number_prop(name: &str) -> Value {
    match name {
        "toFixed" => native_val(|it, t, a| { let n = it.to_num(&t); let d = it.to_num(a.get(0).unwrap_or(&Value::Num(0.0))) as usize; Ok(str_val(fixed(n, d))) }),
        "toString" => native_val(|it, t, a| { let n = it.to_num(&t); let radix = a.get(0).map(|v| it.to_num(v) as u32).unwrap_or(10); if radix == 10 { Ok(str_val(num_to_str(n))) } else { Ok(str_val(to_radix(n, radix))) } }),
        _ => Value::Undefined,
    }
}
fn fixed(n: f64, d: usize) -> String { if n.is_nan() { return "NaN".into(); } let m = powf(10.0, d as f64); let r = trunc_(n * m + if n >= 0.0 { 0.5 } else { -0.5 }) / m; if d == 0 { return num_to_str(trunc_(r)); } let s = format!("{}", r); if let Some(dot) = s.find('.') { let have = s.len() - dot - 1; if have >= d { s[..dot + 1 + d].to_string() } else { let mut s = s; for _ in 0..(d - have) { s.push('0'); } s } } else { let mut s = s; s.push('.'); for _ in 0..d { s.push('0'); } s } }
fn to_radix(mut n: f64, radix: u32) -> String { if n == 0.0 { return "0".into(); } let neg = n < 0.0; n = trunc_(n.abs()); let mut out = Vec::new(); let mut x = n as u64; if x == 0 { return "0".into(); } while x > 0 { let d = (x % radix as u64) as u32; out.push(core::char::from_digit(d, radix).unwrap_or('0')); x /= radix as u64; } if neg { out.push('-'); } out.iter().rev().collect() }

// --- methodes Object.prototype (objet simple) ---
fn object_prop(name: &str) -> Value {
    match name {
        "toString" => native_val(|_it, _t, _a| Ok(str_val("[object Object]"))),
        "valueOf" => native_val(|_it, t, _a| Ok(t)),
        "hasOwnProperty" => native_val(|it, t, a| { let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); Ok(Value::Bool(matches!(&t, Value::Obj(o) if o.borrow().props.contains_key(&k)))) }),
        _ => Value::Undefined,
    }
}

// --- methodes Array (this = tableau) ---
fn array_prop(name: &str) -> Value {
    match name {
        "push" => native_val(|_it, t, a| { if let Value::Obj(o) = &t { if let Some(arr) = &mut o.borrow_mut().arr { for v in a { arr.push(v.clone()); } return Ok(Value::Num(arr.len() as f64)); } } Ok(Value::Num(0.0)) }),
        "pop" => native_val(|_it, t, _a| { if let Value::Obj(o) = &t { if let Some(arr) = &mut o.borrow_mut().arr { return Ok(arr.pop().unwrap_or(Value::Undefined)); } } Ok(Value::Undefined) }),
        "shift" => native_val(|_it, t, _a| { if let Value::Obj(o) = &t { if let Some(arr) = &mut o.borrow_mut().arr { if arr.is_empty() { return Ok(Value::Undefined); } return Ok(arr.remove(0)); } } Ok(Value::Undefined) }),
        "unshift" => native_val(|_it, t, a| { if let Value::Obj(o) = &t { if let Some(arr) = &mut o.borrow_mut().arr { for (i, v) in a.iter().enumerate() { arr.insert(i, v.clone()); } return Ok(Value::Num(arr.len() as f64)); } } Ok(Value::Num(0.0)) }),
        "join" => native_val(|it, t, a| { let sep = a.get(0).map(|v| if matches!(v, Value::Undefined) { ",".into() } else { it.to_string(v) }).unwrap_or_else(|| ",".into()); if let Value::Obj(o) = &t { let arr = o.borrow().arr.clone().unwrap_or_default(); let parts: Vec<String> = arr.iter().map(|v| if matches!(v, Value::Null | Value::Undefined) { String::new() } else { it.to_string(v) }).collect(); return Ok(str_val(parts.join(&sep))); } Ok(str_val(String::new())) }),
        "indexOf" => native_val(|_it, t, a| { if let Value::Obj(o) = &t { let arr = o.borrow().arr.clone().unwrap_or_default(); let target = a.get(0).cloned().unwrap_or(Value::Undefined); for (i, v) in arr.iter().enumerate() { if strict_eq(v, &target) { return Ok(Value::Num(i as f64)); } } } Ok(Value::Num(-1.0)) }),
        "lastIndexOf" => native_val(|_it, t, a| { if let Value::Obj(o) = &t { let arr = o.borrow().arr.clone().unwrap_or_default(); let target = a.get(0).cloned().unwrap_or(Value::Undefined); for (i, v) in arr.iter().enumerate().rev() { if strict_eq(v, &target) { return Ok(Value::Num(i as f64)); } } } Ok(Value::Num(-1.0)) }),
        "includes" => native_val(|_it, t, a| { if let Value::Obj(o) = &t { let arr = o.borrow().arr.clone().unwrap_or_default(); let target = a.get(0).cloned().unwrap_or(Value::Undefined); for v in &arr { if strict_eq(v, &target) { return Ok(Value::Bool(true)); } } } Ok(Value::Bool(false)) }),
        "slice" => native_val(|it, t, a| { if let Value::Obj(o) = &t { let arr = o.borrow().arr.clone().unwrap_or_default(); let (st, en) = slice_bounds(arr.len(), a, it); return Ok(array_val(arr[st..en].to_vec())); } Ok(array_val(Vec::new())) }),
        "concat" => native_val(|it, t, a| { let mut out = Vec::new(); if let Value::Obj(o) = &t { out.extend(o.borrow().arr.clone().unwrap_or_default()); } for v in a { match v { Value::Obj(o) if o.borrow().arr.is_some() => out.extend(o.borrow().arr.clone().unwrap()), _ => out.push(v.clone()) } } let _ = it; Ok(array_val(out)) }),
        "reverse" => native_val(|_it, t, _a| { if let Value::Obj(o) = &t { if let Some(arr) = &mut o.borrow_mut().arr { arr.reverse(); } } Ok(t) }),
        "map" => native_val(|it, t, a| { let arr = arr_of(&t); let f = a.get(0).cloned().unwrap_or(Value::Undefined); let mut out = Vec::new(); for (i, v) in arr.iter().enumerate() { out.push(it.call(f.clone(), Value::Undefined, &[v.clone(), Value::Num(i as f64), t.clone()])?); } Ok(array_val(out)) }),
        "filter" => native_val(|it, t, a| { let arr = arr_of(&t); let f = a.get(0).cloned().unwrap_or(Value::Undefined); let mut out = Vec::new(); for (i, v) in arr.iter().enumerate() { if truthy(&it.call(f.clone(), Value::Undefined, &[v.clone(), Value::Num(i as f64), t.clone()])?) { out.push(v.clone()); } } Ok(array_val(out)) }),
        "forEach" => native_val(|it, t, a| { let arr = arr_of(&t); let f = a.get(0).cloned().unwrap_or(Value::Undefined); for (i, v) in arr.iter().enumerate() { it.call(f.clone(), Value::Undefined, &[v.clone(), Value::Num(i as f64), t.clone()])?; } Ok(Value::Undefined) }),
        "reduce" => native_val(|it, t, a| { let arr = arr_of(&t); let f = a.get(0).cloned().unwrap_or(Value::Undefined); let mut acc; let mut start = 0; if a.len() >= 2 { acc = a[1].clone(); } else { if arr.is_empty() { return Err(str_val("Reduce of empty array with no initial value")); } acc = arr[0].clone(); start = 1; } for i in start..arr.len() { acc = it.call(f.clone(), Value::Undefined, &[acc, arr[i].clone(), Value::Num(i as f64), t.clone()])?; } Ok(acc) }),
        "find" => native_val(|it, t, a| { let arr = arr_of(&t); let f = a.get(0).cloned().unwrap_or(Value::Undefined); for (i, v) in arr.iter().enumerate() { if truthy(&it.call(f.clone(), Value::Undefined, &[v.clone(), Value::Num(i as f64), t.clone()])?) { return Ok(v.clone()); } } Ok(Value::Undefined) }),
        "findIndex" => native_val(|it, t, a| { let arr = arr_of(&t); let f = a.get(0).cloned().unwrap_or(Value::Undefined); for (i, v) in arr.iter().enumerate() { if truthy(&it.call(f.clone(), Value::Undefined, &[v.clone(), Value::Num(i as f64), t.clone()])?) { return Ok(Value::Num(i as f64)); } } Ok(Value::Num(-1.0)) }),
        "some" => native_val(|it, t, a| { let arr = arr_of(&t); let f = a.get(0).cloned().unwrap_or(Value::Undefined); for (i, v) in arr.iter().enumerate() { if truthy(&it.call(f.clone(), Value::Undefined, &[v.clone(), Value::Num(i as f64), t.clone()])?) { return Ok(Value::Bool(true)); } } Ok(Value::Bool(false)) }),
        "every" => native_val(|it, t, a| { let arr = arr_of(&t); let f = a.get(0).cloned().unwrap_or(Value::Undefined); for (i, v) in arr.iter().enumerate() { if !truthy(&it.call(f.clone(), Value::Undefined, &[v.clone(), Value::Num(i as f64), t.clone()])?) { return Ok(Value::Bool(false)); } } Ok(Value::Bool(true)) }),
        "map_" => Value::Undefined,
        "sort" => native_val(|it, t, a| { if let Value::Obj(o) = &t { let mut arr = o.borrow().arr.clone().unwrap_or_default(); let cmp = a.get(0).cloned(); // tri a bulles (stable, petites listes)
            let n = arr.len(); for i in 0..n { for j in 0..n - 1 - i.min(n.saturating_sub(1)) { let swap = if let Some(f) = &cmp { let r = it.call(f.clone(), Value::Undefined, &[arr[j].clone(), arr[j + 1].clone()])?; it.to_num(&r) > 0.0 } else { it.to_string(&arr[j]) > it.to_string(&arr[j + 1]) }; if swap { arr.swap(j, j + 1); } } } if let Some(slot) = &mut o.borrow_mut().arr { *slot = arr; } } Ok(t) }),
        "fill" => native_val(|_it, t, a| { if let Value::Obj(o) = &t { let v = a.get(0).cloned().unwrap_or(Value::Undefined); if let Some(arr) = &mut o.borrow_mut().arr { for slot in arr.iter_mut() { *slot = v.clone(); } } } Ok(t) }),
        "flat" => native_val(|_it, t, _a| { let arr = arr_of(&t); let mut out = Vec::new(); for v in arr { match v { Value::Obj(o) if o.borrow().arr.is_some() => out.extend(o.borrow().arr.clone().unwrap()), _ => out.push(v) } } Ok(array_val(out)) }),
        "keys" => native_val(|_it, t, _a| { let arr = arr_of(&t); Ok(array_val((0..arr.len()).map(|i| Value::Num(i as f64)).collect())) }),
        "toString" => native_val(|it, t, _a| Ok(str_val(it.to_string(&t)))),
        _ => Value::Undefined,
    }
}
fn arr_of(t: &Value) -> Vec<Value> { if let Value::Obj(o) = t { o.borrow().arr.clone().unwrap_or_default() } else { Vec::new() } }

// --- JSON ---
fn json_stringify(it: &Interp, v: &Value) -> Option<String> {
    match v {
        Value::Undefined => None,
        Value::Null => Some("null".into()),
        Value::Bool(b) => Some(if *b { "true".into() } else { "false".into() }),
        Value::Num(n) => Some(if n.is_finite() { num_to_str(*n) } else { "null".into() }),
        Value::Str(s) => Some(json_quote(s)),
        Value::Obj(o) => { let b = o.borrow(); if let Some(a) = &b.arr { let parts: Vec<String> = a.iter().map(|x| json_stringify(it, x).unwrap_or_else(|| "null".into())).collect(); Some(format!("[{}]", parts.join(","))) } else if b.call.is_some() { None } else { let mut parts = Vec::new(); for (k, val) in b.props.iter() { if let Some(s) = json_stringify(it, val) { parts.push(format!("{}:{}", json_quote(k), s)); } } Some(format!("{{{}}}", parts.join(","))) } }
    }
}
fn json_quote(s: &str) -> String { let mut o = String::from("\""); for c in s.chars() { match c { '"' => o.push_str("\\\""), '\\' => o.push_str("\\\\"), '\n' => o.push_str("\\n"), '\r' => o.push_str("\\r"), '\t' => o.push_str("\\t"), c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)), c => o.push(c) } } o.push('"'); o }
fn json_parse(s: &str) -> Option<Value> { let b = s.as_bytes(); let mut i = 0; let v = jp_value(b, &mut i)?; jp_ws(b, &mut i); Some(v) }
fn jp_ws(b: &[u8], i: &mut usize) { while *i < b.len() && (b[*i] == b' ' || b[*i] == b'\t' || b[*i] == b'\n' || b[*i] == b'\r') { *i += 1; } }
fn jp_value(b: &[u8], i: &mut usize) -> Option<Value> {
    jp_ws(b, i); if *i >= b.len() { return None; }
    match b[*i] {
        b'{' => { *i += 1; let mut o = Obj::plain(); jp_ws(b, i); if *i < b.len() && b[*i] == b'}' { *i += 1; return Some(new_obj(o)); } loop { jp_ws(b, i); let k = if b.get(*i) == Some(&b'"') { jp_string(b, i)? } else { return None }; jp_ws(b, i); if b.get(*i) != Some(&b':') { return None; } *i += 1; let v = jp_value(b, i)?; o.props.insert(k, v); jp_ws(b, i); match b.get(*i) { Some(b',') => { *i += 1; } Some(b'}') => { *i += 1; break; } _ => return None } } Some(new_obj(o)) }
        b'[' => { *i += 1; let mut arr = Vec::new(); jp_ws(b, i); if *i < b.len() && b[*i] == b']' { *i += 1; return Some(array_val(arr)); } loop { let v = jp_value(b, i)?; arr.push(v); jp_ws(b, i); match b.get(*i) { Some(b',') => { *i += 1; } Some(b']') => { *i += 1; break; } _ => return None } } Some(array_val(arr)) }
        b'"' => Some(str_val(jp_string(b, i)?)),
        b't' => { if b[*i..].starts_with(b"true") { *i += 4; Some(Value::Bool(true)) } else { None } }
        b'f' => { if b[*i..].starts_with(b"false") { *i += 5; Some(Value::Bool(false)) } else { None } }
        b'n' => { if b[*i..].starts_with(b"null") { *i += 4; Some(Value::Null) } else { None } }
        _ => { let start = *i; while *i < b.len() && (b[*i].is_ascii_digit() || matches!(b[*i], b'-' | b'+' | b'.' | b'e' | b'E')) { *i += 1; } core::str::from_utf8(&b[start..*i]).ok()?.parse::<f64>().ok().map(Value::Num) }
    }
}
fn jp_string(b: &[u8], i: &mut usize) -> Option<String> {
    *i += 1; let mut out = String::new();
    while *i < b.len() && b[*i] != b'"' { if b[*i] == b'\\' { *i += 1; if *i >= b.len() { return None; } match b[*i] { b'"' => out.push('"'), b'\\' => out.push('\\'), b'/' => out.push('/'), b'n' => out.push('\n'), b't' => out.push('\t'), b'r' => out.push('\r'), b'b' => out.push('\u{8}'), b'f' => out.push('\u{c}'), b'u' => { let mut v = 0u32; for _ in 0..4 { *i += 1; if *i < b.len() { v = v * 16 + hexv(b[*i]); } } out.push(char::from_u32(v).unwrap_or('?')); } _ => out.push(b[*i] as char) } *i += 1; } else { out.push(b[*i] as char); *i += 1; } }
    *i += 1; Some(out)
}

// ============================================================================
// Binding DOM (modele HTML-string : voir execute_inline)
// ============================================================================

// --- Element JS <-> noeud DOM ---

fn node_handle(n: usize) -> Value {
    let el = new_obj(Obj { props: OrderedMap::new(), arr: None, call: None, class: "Element" });
    set(&el, "__node__", Value::Num(n as f64));
    el
}
fn detached_element(tag: &str) -> Value {
    let el = new_obj(Obj { props: OrderedMap::new(), arr: None, call: None, class: "Element" });
    set(&el, "__node__", Value::Num(-1.0));
    set(&el, "__tag__", str_val(tag.to_lowercase()));
    set(&el, "__html__", str_val(String::new()));
    set(&el, "__attrs__", new_obj(Obj::plain()));
    el
}
fn handle_node(it: &mut Interp, this: &Value) -> i64 {
    if let Value::Obj(o) = this { o.borrow().props.get("__node__").map(|v| it.to_num(v) as i64).unwrap_or(-1) } else { -1 }
}

// element.addEventListener(type, cb) : enregistre l'ecouteur sur le noeud. Pour
// "click", injecte un attribut onclick synthetique (`__ael(n)`) afin que le
// pipeline de liens `javascript:` existant rejoue l'ecouteur au clic.
fn dom_add_event_listener(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let node = handle_node(it, &this);
    let ty = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
    if node >= 0 {
        if let Some(cb) = a.get(1) { it.listeners.push((node, ty.clone(), cb.clone())); }
        if ty == "click" {
            let n = node as usize;
            let existing = it.dom.attr(n, "onclick").unwrap_or("").to_string();
            if !existing.contains("__ael(") {
                let call = format!("__ael({})", node);
                let val = if existing.is_empty() { call } else { format!("{};{}", existing, call) };
                it.dom.set_attr(n, "onclick", val);
            }
        }
    }
    Ok(Value::Undefined)
}

fn dom_remove_event_listener(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let node = handle_node(it, &this);
    let ty = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
    it.listeners.retain(|(n, t, _)| !(*n == node && t == &ty));
    Ok(Value::Undefined)
}

// HTML externe d'une valeur (element model-backed, element detache, ou texte).
fn element_outer(it: &mut Interp, v: &Value) -> String {
    if let Value::Obj(o) = v {
        let node = o.borrow().props.get("__node__").map(|x| it.to_num(x) as i64).unwrap_or(-2);
        if node >= 0 { return it.dom.outer_html(node as usize); }
        if node == -1 {
            let (tag, html) = { let b = o.borrow(); (b.props.get("__tag__").map(|x| it.to_string(x)).unwrap_or_default(), b.props.get("__html__").map(|x| it.to_string(x)).unwrap_or_default()) };
            if tag == "#text" || tag.is_empty() { return html; }
            let attrs = { let b = o.borrow(); b.props.get("__attrs__").cloned() };
            let mut av: Vec<(String, String)> = Vec::new();
            if let Some(Value::Obj(at)) = attrs { for (k, val) in at.borrow().props.iter() { av.push((k.clone(), it.to_string(val))); } }
            return format!("{}{}</{}>", serialize_open(&tag, &av), html, tag);
        }
    }
    escape_html(&it.to_string(v))
}

fn dom_get(it: &mut Interp, obj: &Rc<RefCell<Obj>>, name: &str) -> Option<Value> {
    let node = { let b = obj.borrow(); b.props.get("__node__").map(|v| it.to_num(v) as i64) };
    let this = Value::Obj(obj.clone());
    if let Some(n) = node {
        if n >= 0 {
            let n = n as usize;
            if n >= it.dom.nodes.len() { return Some(Value::Undefined); }
            return Some(match name {
                "innerHTML" => str_val(it.dom.inner_html(n)),
                "outerHTML" => str_val(it.dom.outer_html(n)),
                "textContent" | "innerText" => str_val(it.dom.text_content(n)),
                "tagName" | "nodeName" => str_val(it.dom.nodes[n].tag.to_uppercase()),
                "id" => str_val(it.dom.attr(n, "id").unwrap_or("").to_string()),
                "className" => str_val(it.dom.attr(n, "class").unwrap_or("").to_string()),
                "value" => str_val(it.dom.attr(n, "value").unwrap_or("").to_string()),
                "href" => str_val(it.dom.attr(n, "href").unwrap_or("").to_string()),
                "src" => str_val(it.dom.attr(n, "src").unwrap_or("").to_string()),
                "nodeType" => Value::Num(1.0),
                "children" | "childNodes" => array_val(it.dom.nodes[n].children.clone().into_iter().map(node_handle).collect()),
                "childElementCount" => Value::Num(it.dom.nodes[n].children.len() as f64),
                "firstChild" | "firstElementChild" => it.dom.nodes[n].children.first().map(|&c| node_handle(c)).unwrap_or(Value::Null),
                "lastChild" | "lastElementChild" => it.dom.nodes[n].children.last().map(|&c| node_handle(c)).unwrap_or(Value::Null),
                "parentNode" | "parentElement" => it.dom.parent_of(n).map(node_handle).unwrap_or(Value::Null),
                "ownerDocument" => scope_get(&it.global, "document").unwrap_or(Value::Null),
                "style" => style_object(&this),
                "dataset" => dataset_object(&this),
                "classList" => class_list(n as i64),
                "getAttribute" => native_val(dom_get_attr),
                "setAttribute" => native_val(dom_set_attr),
                "hasAttribute" => native_val(dom_has_attr),
                "removeAttribute" => native_val(|_it, _t, _a| Ok(Value::Undefined)),
                "appendChild" | "append" | "prepend" | "insertBefore" => native_val(dom_append_child),
                "querySelector" => native_val(dom_qs),
                "querySelectorAll" => native_val(dom_qsa),
                "getElementsByTagName" => native_val(dom_qsa),
                "getElementsByClassName" => native_val(dom_qsa),
                "addEventListener" => native_val(dom_add_event_listener),
                "removeEventListener" => native_val(dom_remove_event_listener),
                "dispatchEvent" => native_val(|it, t, _a| { let n = handle_node(it, &t); if n >= 0 { it.fire_event(n, "click"); } Ok(Value::Bool(true)) }),
                "click" => native_val(|it, t, _a| { let n = handle_node(it, &t); if n >= 0 { it.fire_event(n, "click"); } Ok(Value::Undefined) }),
                "removeChild" | "remove" | "replaceChild"
                | "focus" | "blur" | "scrollIntoView" | "setAttributeNS" => native_val(|_it, _t, _a| Ok(Value::Undefined)),
                "matches" | "contains" => native_val(|_it, _t, _a| Ok(Value::Bool(false))),
                _ => { let b = obj.borrow(); b.props.get(name).cloned().unwrap_or(Value::Undefined) }
            });
        }
        // element detache (createElement)
        return Some(match name {
            "innerHTML" | "textContent" | "innerText" => obj.borrow().props.get("__html__").cloned().unwrap_or_else(|| str_val(String::new())),
            "tagName" | "nodeName" => str_val(obj.borrow().props.get("__tag__").map(|v| it.to_string(v).to_uppercase()).unwrap_or_default()),
            "outerHTML" => str_val(element_outer(it, &this)),
            "nodeType" => Value::Num(1.0),
            "ownerDocument" => scope_get(&it.global, "document").unwrap_or(Value::Null),
            "getAttribute" => native_val(dom_get_attr),
            "setAttribute" => native_val(dom_set_attr),
            "hasAttribute" => native_val(dom_has_attr),
            "appendChild" | "append" | "prepend" => native_val(dom_append_child),
            "style" => style_object(&this),
            "classList" => class_list(-1),
            "addEventListener" | "removeEventListener" | "focus" | "click" => native_val(|_it, _t, _a| Ok(Value::Undefined)),
            _ => { let b = obj.borrow(); b.props.get(name).cloned().unwrap_or(Value::Undefined) }
        });
    }
    None
}

fn dom_set(it: &mut Interp, obj: &Rc<RefCell<Obj>>, name: &str, v: &Value) -> bool {
    let node = { let b = obj.borrow(); b.props.get("__node__").map(|x| it.to_num(x) as i64) };
    let node = match node { Some(n) => n, None => return false };
    if node >= 0 {
        let n = node as usize;
        if n >= it.dom.nodes.len() { return true; }
        match name {
            "innerHTML" => { let html = it.to_string(v); it.dom.set_inner(n, html); true }
            "textContent" | "innerText" => { let txt = escape_html(&it.to_string(v)); it.dom.set_inner(n, txt); true }
            "className" => { let val = it.to_string(v); it.dom.set_attr(n, "class", val); true }
            "id" => { let val = it.to_string(v); it.dom.set_attr(n, "id", val); true }
            "value" => { let val = it.to_string(v); it.dom.set_attr(n, "value", val); true }
            "href" | "src" | "title" | "alt" => { let val = it.to_string(v); it.dom.set_attr(n, name, val); true }
            // proprietes sans effet visuel : on absorbe (pas d'erreur).
            "onclick" | "onload" | "onchange" | "hidden" | "checked" | "disabled" | "scrollTop" | "scrollLeft" => true,
            _ => false,
        }
    } else {
        // element detache
        match name {
            "innerHTML" | "textContent" | "innerText" => { let html = if name == "innerHTML" { it.to_string(v) } else { escape_html(&it.to_string(v)) }; obj.borrow_mut().props.insert("__html__".into(), str_val(html)); true }
            "className" | "id" | "href" | "src" | "value" | "title" | "alt" => {
                let val = it.to_string(v);
                let key = if name == "className" { "class".to_string() } else { name.to_string() };
                let at = obj.borrow().props.get("__attrs__").cloned();
                if let Some(Value::Obj(a)) = at { a.borrow_mut().props.insert(key, str_val(val)); }
                true
            }
            _ => false,
        }
    }
}

// Objet `style` live : les ecritures (`el.style.color = ...`) sont re-serialisees
// vers l'attribut `style=""` du noeud, donc reprises par le moteur de layout.
fn style_object(this: &Value) -> Value {
    if let Value::Obj(o) = this {
        if let Some(s) = o.borrow().props.get("__style__").cloned() { return s; }
        let node = o.borrow().props.get("__node__").map(|v| to_num_simple(v) as i64).unwrap_or(-1);
        let s = new_obj(Obj { props: OrderedMap::new(), arr: None, call: None, class: "Style" });
        set(&s, "__owner__", Value::Num(node as f64));
        set(&s, "setProperty", native_val(|it, t, a| { let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); let v = a.get(1).cloned().unwrap_or(Value::Undefined); it.set_prop(&t, &k, v); Ok(Value::Undefined) }));
        set(&s, "removeProperty", native_val(|it, t, a| { let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); it.set_prop(&t, &k, str_val(String::new())); Ok(Value::Undefined) }));
        set(&s, "getPropertyValue", native_val(|it, t, a| { let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined)); it.get_prop(&t, &k) }));
        o.borrow_mut().props.insert("__style__".into(), s.clone());
        return s;
    }
    new_obj(Obj::plain())
}

// Objet `dataset` : simple memo (data-*), sans effet sur le layout.
fn dataset_object(this: &Value) -> Value {
    if let Value::Obj(o) = this {
        if let Some(s) = o.borrow().props.get("__dataset__").cloned() { return s; }
        let s = new_obj(Obj::plain());
        o.borrow_mut().props.insert("__dataset__".into(), s.clone());
        return s;
    }
    new_obj(Obj::plain())
}

// Convertit un nom de propriete style JS (camelCase) en propriete CSS (kebab).
fn css_prop_name(name: &str) -> String {
    let mut o = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() { o.push('-'); o.push(c.to_ascii_lowercase()); } else { o.push(c); }
    }
    o
}

// Serialise les proprietes d'un objet `style` en chaine CSS (`k:v;...`).
fn serialize_style(obj: &Rc<RefCell<Obj>>) -> String {
    let b = obj.borrow();
    let mut out = String::new();
    for (k, v) in b.props.iter() {
        if k.starts_with("__") || k == "setProperty" || k == "removeProperty" || k == "getPropertyValue" || k == "cssText" { continue; }
        if matches!(v, Value::Obj(o) if o.borrow().call.is_some()) { continue; }
        let val = match v { Value::Str(s) => (**s).clone(), Value::Num(n) => num_to_str(*n), Value::Bool(b) => if *b { "true".into() } else { "false".into() }, _ => continue };
        if val.is_empty() { continue; }
        out.push_str(k); out.push(':'); out.push_str(&val); out.push(';');
    }
    out
}

// classList lie a un noeud (n>=0) ; methodes add/remove/toggle/contains.
fn class_list(n: i64) -> Value {
    let cl = new_obj(Obj::plain());
    set(&cl, "__node__", Value::Num(n as f64));
    set(&cl, "add", native_val(|it, t, a| { cl_edit(it, &t, a, 0); Ok(Value::Undefined) }));
    set(&cl, "remove", native_val(|it, t, a| { cl_edit(it, &t, a, 1); Ok(Value::Undefined) }));
    set(&cl, "toggle", native_val(|it, t, a| { Ok(Value::Bool(cl_edit(it, &t, a, 2))) }));
    set(&cl, "contains", native_val(|it, t, a| {
        let n = handle_node(it, &t); if n < 0 { return Ok(Value::Bool(false)); }
        let c = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::Bool(it.dom.attr(n as usize, "class").map(|cl| cl.split(' ').any(|x| x == c)).unwrap_or(false)))
    }));
    cl
}
fn cl_edit(it: &mut Interp, this: &Value, a: &[Value], op: u8) -> bool {
    let n = handle_node(it, this); if n < 0 { return false; }
    let n = n as usize;
    let c = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
    let cur = it.dom.attr(n, "class").unwrap_or("").to_string();
    let mut parts: Vec<&str> = cur.split(' ').filter(|x| !x.is_empty()).collect();
    let present = parts.iter().any(|x| *x == c);
    let mut added = false;
    match op {
        0 => { if !present { parts.push(&c); added = true; } }
        1 => { parts.retain(|x| *x != c); }
        _ => { if present { parts.retain(|x| *x != c); } else { parts.push(&c); added = true; } }
    }
    it.dom.set_attr(n, "class", parts.join(" "));
    added
}

fn dom_get_attr(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let name = it.to_string(a.get(0).unwrap_or(&Value::Undefined)).to_lowercase();
    let n = handle_node(it, &this);
    if n >= 0 { return Ok(it.dom.attr(n as usize, &name).map(str_val).unwrap_or(Value::Null)); }
    if let Value::Obj(o) = &this { if let Some(Value::Obj(at)) = o.borrow().props.get("__attrs__").cloned() { return Ok(at.borrow().props.get(&name).cloned().unwrap_or(Value::Null)); } }
    Ok(Value::Null)
}
fn dom_set_attr(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let name = it.to_string(a.get(0).unwrap_or(&Value::Undefined)).to_lowercase();
    let val = it.to_string(a.get(1).unwrap_or(&Value::Undefined));
    let n = handle_node(it, &this);
    if n >= 0 { it.dom.set_attr(n as usize, &name, val); return Ok(Value::Undefined); }
    if let Value::Obj(o) = &this { if let Some(Value::Obj(at)) = o.borrow().props.get("__attrs__").cloned() { at.borrow_mut().props.insert(name, str_val(val)); } }
    Ok(Value::Undefined)
}
fn dom_has_attr(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let name = it.to_string(a.get(0).unwrap_or(&Value::Undefined)).to_lowercase();
    let n = handle_node(it, &this);
    if n >= 0 { return Ok(Value::Bool(it.dom.attr(n as usize, &name).is_some())); }
    Ok(Value::Bool(false))
}
fn dom_append_child(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let child = a.get(0).cloned().unwrap_or(Value::Undefined);
    let html = element_outer(it, &child);
    let n = handle_node(it, &this);
    if n >= 0 { it.dom.append_html(n as usize, &html); }
    else if let Value::Obj(o) = &this {
        let cur = o.borrow().props.get("__html__").map(|x| it.to_string(x)).unwrap_or_default();
        o.borrow_mut().props.insert("__html__".into(), str_val(format!("{}{}", cur, html)));
    }
    Ok(child)
}
fn dom_qs(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let sel = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
    let n = handle_node(it, &this);
    let from = if n >= 0 { n as usize } else { 0 };
    Ok(it.dom.query(&sel, from, true).first().map(|&x| node_handle(x)).unwrap_or(Value::Null))
}
fn dom_qsa(it: &mut Interp, this: Value, a: &[Value]) -> Result<Value, Value> {
    let sel = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
    let n = handle_node(it, &this);
    let from = if n >= 0 { n as usize } else { 0 };
    Ok(array_val(it.dom.query(&sel, from, false).into_iter().map(node_handle).collect()))
}

// ============================================================================
// Modele DOM (parse HTML -> arbre avec plages d'octets pour les mutations)
// ============================================================================

struct DomNode {
    tag: String,
    attrs: Vec<(String, String)>,
    open_start: usize,
    inner_start: usize,
    inner_end: usize,
    children: Vec<usize>,
    pending_inner: Option<String>,
    appends: String,
    dirty: bool,
}

pub struct DomModel { html: Vec<u8>, nodes: Vec<DomNode> }

#[derive(Clone)]
enum SelKind { Any, Id(String), Class(String), Tag(String) }

fn parse_sel(s: &str) -> SelKind {
    let s = s.trim();
    let last = s.split(|c: char| c == ' ' || c == '>' || c == '+' || c == '~').filter(|x| !x.is_empty()).last().unwrap_or(s);
    let base = last.split(|c: char| c == ':' || c == '[').next().unwrap_or(last);
    if base == "*" || base.is_empty() { return SelKind::Any; }
    if let Some(x) = base.strip_prefix('#') { return SelKind::Id(x.to_string()); }
    if let Some(x) = base.strip_prefix('.') { return SelKind::Class(x.split('.').next().unwrap_or(x).to_string()); }
    if let Some(p) = base.find(['.', '#']) {
        let rest = &base[p..];
        if let Some(c) = rest.strip_prefix('.') { return SelKind::Class(c.split('.').next().unwrap_or(c).to_string()); }
        if let Some(i) = rest.strip_prefix('#') { return SelKind::Id(i.to_string()); }
    }
    SelKind::Tag(base.to_ascii_lowercase())
}

impl DomModel {
    fn empty() -> DomModel { DomModel { html: Vec::new(), nodes: Vec::new() } }

    fn parse(html: &[u8]) -> DomModel {
        let mut nodes = alloc::vec![DomNode { tag: "#root".into(), attrs: Vec::new(), open_start: 0, inner_start: 0, inner_end: html.len(), children: Vec::new(), pending_inner: None, appends: String::new(), dirty: false }];
        let mut stack: Vec<usize> = alloc::vec![0];
        let mut i = 0usize;
        while i < html.len() {
            if html[i] != b'<' { i += 1; continue; }
            if html[i..].starts_with(b"<!--") { i = find_ci(html, b"-->", i).map(|p| p + 3).unwrap_or(html.len()); continue; }
            if i + 1 < html.len() && html[i + 1] == b'!' { i = find_ci(html, b">", i).map(|p| p + 1).unwrap_or(html.len()); continue; }
            let lt = i;
            let gt = match find_ci(html, b">", i) { Some(p) => p, None => break };
            let raw = &html[i + 1..gt];
            let closing = raw.first() == Some(&b'/');
            let mut p = if closing { 1 } else { 0 };
            let ns = p;
            while p < raw.len() && (raw[p] as char).is_ascii_alphanumeric() { p += 1; }
            let name: String = raw[ns..p].iter().map(|&c| (c as char).to_ascii_lowercase()).collect();
            if name.is_empty() { i = gt + 1; continue; }
            if name == "script" || name == "style" {
                if !closing {
                    let close: &[u8] = if name == "script" { b"</script" } else { b"</style" };
                    let ce = find_ci(html, close, gt + 1).unwrap_or(html.len());
                    i = find_ci(html, b">", ce).map(|q| q + 1).unwrap_or(html.len());
                } else { i = gt + 1; }
                continue;
            }
            if closing {
                if let Some(pos) = stack.iter().rposition(|&n| nodes[n].tag == name) {
                    let nidx = stack[pos];
                    nodes[nidx].inner_end = lt;
                    stack.truncate(pos.max(1));
                }
                i = gt + 1;
                continue;
            }
            let attrs = parse_attrs_dom(&raw[p..]);
            let self_closing = raw.last() == Some(&b'/');
            let idx = nodes.len();
            nodes.push(DomNode { tag: name.clone(), attrs, open_start: lt, inner_start: gt + 1, inner_end: gt + 1, children: Vec::new(), pending_inner: None, appends: String::new(), dirty: false });
            let parent = *stack.last().unwrap_or(&0);
            nodes[parent].children.push(idx);
            if !is_void_dom(&name) && !self_closing { stack.push(idx); }
            i = gt + 1;
            if nodes.len() > 40_000 { break; }
        }
        // ferme les noeuds restes ouverts
        for &n in stack.iter().skip(1) { if nodes[n].inner_end < nodes[n].inner_start { nodes[n].inner_end = html.len(); } }
        DomModel { html: html.to_vec(), nodes }
    }

    fn attr(&self, i: usize, name: &str) -> Option<&str> {
        self.nodes.get(i)?.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn parent_of(&self, n: usize) -> Option<usize> {
        (1..self.nodes.len()).find(|&p| self.nodes[p].children.contains(&n))
    }
    fn find_by_id(&self, id: &str) -> Option<usize> {
        (1..self.nodes.len()).find(|&n| self.attr(n, "id") == Some(id))
    }
    fn query(&self, sel: &str, from: usize, first: bool) -> Vec<usize> {
        let kind = parse_sel(sel);
        let mut out = Vec::new();
        let mut budget = 300_000usize;
        self.collect(from, &kind, first, &mut out, &mut budget);
        out
    }
    fn collect(&self, node: usize, kind: &SelKind, first: bool, out: &mut Vec<usize>, budget: &mut usize) {
        if node >= self.nodes.len() { return; }
        for ci in 0..self.nodes[node].children.len() {
            if *budget == 0 { return; }
            *budget -= 1;
            let c = self.nodes[node].children[ci];
            if self.matches(c, kind) { out.push(c); if first { return; } }
            self.collect(c, kind, first, out, budget);
            if first && !out.is_empty() { return; }
        }
    }
    fn matches(&self, n: usize, kind: &SelKind) -> bool {
        match kind {
            SelKind::Any => true,
            SelKind::Id(x) => self.attr(n, "id") == Some(x.as_str()),
            SelKind::Tag(t) => &self.nodes[n].tag == t,
            SelKind::Class(c) => self.attr(n, "class").map(|cl| cl.split(' ').any(|x| x == c)).unwrap_or(false),
        }
    }
    fn inner_html(&self, i: usize) -> String {
        let n = &self.nodes[i];
        let base = match &n.pending_inner {
            Some(s) => s.clone(),
            None => {
                let a = n.inner_start.min(self.html.len());
                let b = n.inner_end.min(self.html.len()).max(a);
                core::str::from_utf8(&self.html[a..b]).unwrap_or("").to_string()
            }
        };
        if n.appends.is_empty() { base } else { format!("{}{}", base, n.appends) }
    }
    fn text_content(&self, i: usize) -> String { strip_tags(&self.inner_html(i)) }
    fn outer_html(&self, i: usize) -> String {
        let n = &self.nodes[i];
        format!("{}{}</{}>", serialize_open(&n.tag, &n.attrs), self.inner_html(i), n.tag)
    }
    fn set_inner(&mut self, i: usize, html: String) { if i < self.nodes.len() { let n = &mut self.nodes[i]; n.pending_inner = Some(html); n.appends.clear(); } }
    fn append_html(&mut self, i: usize, html: &str) { if i < self.nodes.len() { self.nodes[i].appends.push_str(html); } }
    fn set_attr(&mut self, i: usize, name: &str, val: String) {
        if i >= self.nodes.len() { return; }
        let n = &mut self.nodes[i];
        if let Some(slot) = n.attrs.iter_mut().find(|(k, _)| k == name) { slot.1 = val; } else { n.attrs.push((name.to_string(), val)); }
        n.dirty = true;
    }
    // Reconstruit le HTML : retire/insere les scripts (document.write), applique
    // les mutations (innerHTML/append/attributs) par plages non chevauchantes.
    fn rebuild(&self, scripts: &[(usize, usize, String)]) -> Vec<u8> {
        let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        for &(s, e, ref w) in scripts { edits.push((s, e, w.clone().into_bytes())); }
        for (i, n) in self.nodes.iter().enumerate() {
            if i == 0 { continue; }
            if n.pending_inner.is_some() || !n.appends.is_empty() {
                edits.push((n.inner_start, n.inner_end, self.inner_html(i).into_bytes()));
            }
            if n.dirty {
                edits.push((n.open_start, n.inner_start, serialize_open(&n.tag, &n.attrs).into_bytes()));
            }
        }
        edits.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let mut out = Vec::with_capacity(self.html.len() + 256);
        let mut pos = 0usize;
        for (s, e, rep) in edits {
            if s < pos || e > self.html.len() || s > e { continue; }
            out.extend_from_slice(&self.html[pos..s]);
            out.extend_from_slice(&rep);
            pos = e;
        }
        out.extend_from_slice(&self.html[pos.min(self.html.len())..]);
        out
    }
}

fn is_void_dom(t: &str) -> bool { matches!(t, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr") }

fn parse_attrs_dom(raw: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        while i < raw.len() && matches!(raw[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') { i += 1; }
        let ks = i;
        while i < raw.len() && !matches!(raw[i], b'=' | b' ' | b'\t' | b'\n' | b'\r' | b'>') { i += 1; }
        if i == ks { break; }
        let key: String = raw[ks..i].iter().map(|&c| (c as char).to_ascii_lowercase()).collect();
        let mut val = String::new();
        while i < raw.len() && matches!(raw[i], b' ' | b'\t') { i += 1; }
        if i < raw.len() && raw[i] == b'=' {
            i += 1;
            while i < raw.len() && matches!(raw[i], b' ' | b'\t') { i += 1; }
            if i < raw.len() && (raw[i] == b'"' || raw[i] == b'\'') {
                let q = raw[i]; i += 1; let vs = i;
                while i < raw.len() && raw[i] != q { i += 1; }
                val = core::str::from_utf8(&raw[vs..i]).unwrap_or("").to_string(); i += 1;
            } else {
                let vs = i;
                while i < raw.len() && !matches!(raw[i], b' ' | b'>' | b'\t') { i += 1; }
                val = core::str::from_utf8(&raw[vs..i]).unwrap_or("").to_string();
            }
        }
        out.push((key, val));
    }
    out
}

fn serialize_open(tag: &str, attrs: &[(String, String)]) -> String {
    let mut s = format!("<{}", tag);
    for (k, v) in attrs { s.push(' '); s.push_str(k); s.push_str("=\""); s.push_str(&v.replace('"', "&quot;")); s.push('"'); }
    s.push('>');
    s
}

fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in html.chars() {
        match c { '<' => depth += 1, '>' => { if depth > 0 { depth -= 1; } } _ => if depth == 0 { out.push(c); } }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ============================================================================
// Integration page : execute_inline
// ============================================================================

const MAX_OUTPUT: usize = 16_000_000;
// Taille max d'un script execute. Le tas est desormais etendu sur la RAM
// physique mappee (plusieurs centaines de Mio, cf. `kernel::heap` /
// `kernel::memory`) et QEMU tourne avec `-m 2048`, ce qui permet de
// tokeniser/parser de vrais bundles modernes (frameworks, SPA) au lieu de les
// ignorer. On vise les gros `xjs`/`og` de Google (~1 Mo) et React/Bootstrap.
const MAX_SCRIPT: usize = 1_300_000;
// Budget cumule d'octets de script execute par page (borne memoire + vitesse).
const MAX_TOTAL_SCRIPT: usize = 6_000_000;

/// Execute les `<script>` inline sur un DOM partage et renvoie le HTML enrichi :
/// document.write insere a la position du script, mutations DOM (innerHTML,
/// textContent, appendChild...) appliquees par plages.
pub fn execute_inline(html: &[u8], base_url: &str) -> Vec<u8> {
    let (_ctx, out) = open_page(html, base_url);
    out
}

// Scheme/host d'une URL de base, pour resoudre les `<script src>` relatifs.
fn scheme_of(base: &str) -> &str {
    if base.starts_with("https://") { "https" } else if base.starts_with("http://") { "http" } else { "https" }
}
fn host_of(base: &str) -> &str {
    let rest = base.strip_prefix("https://").or_else(|| base.strip_prefix("http://")).unwrap_or(base);
    match rest.find('/') { Some(i) => &rest[..i], None => rest }
}

/// Evalue une expression JS isolee et renvoie son resultat formate (semantique
/// JS). Utilise par l'application Calculatrice : l'OS calcule via son propre
/// moteur de langage embarque.
pub fn eval_expr(src: &str) -> Result<String, String> {
    let mut interp = Interp::new();
    let v = interp.run(src)?;
    Ok(interp.to_string(&v))
}

/// Contexte JS persistant d'une page : conserve l'interpreteur (donc l'etat des
/// variables/fonctions et le DOM mute) entre l'ouverture et les evenements.
/// Permet des mini-applications interactives (boutons `onclick`).
pub struct PageCtx {
    pub interp: Interp,
    scripts: Vec<(usize, usize, String)>,
}

impl PageCtx {
    /// Rejoue un gestionnaire d'evenement (code JS) puis renvoie le HTML a jour.
    pub fn dispatch(&mut self, code: &str) -> Vec<u8> {
        self.interp.writes.clear();
        let _ = self.interp.run(code);
        self.interp.pump(); // draine timers + microtaches declenches par le handler
        let mut out = self.interp.dom.rebuild(&self.scripts);
        if out.len() > MAX_OUTPUT { out.truncate(MAX_OUTPUT); }
        out
    }
    /// Reconstruit le HTML courant sans rejouer de code (re-rendu).
    pub fn html(&self) -> Vec<u8> {
        let mut out = self.interp.dom.rebuild(&self.scripts);
        if out.len() > MAX_OUTPUT { out.truncate(MAX_OUTPUT); }
        out
    }
}

/// Ouvre une page : construit le DOM, execute les scripts (inline ET externes
/// via `<script src>`, telecharges depuis `base_url`), et renvoie le contexte
/// persistant + le HTML initial enrichi.
pub fn open_page(html: &[u8], base_url: &str) -> (PageCtx, Vec<u8>) { open_page_inner(html, base_url, true) }

/// Variante sans execution du JS de la page (rendu statique).
pub fn open_page_static(html: &[u8], base_url: &str) -> (PageCtx, Vec<u8>) { open_page_inner(html, base_url, false) }

// Extrait la valeur d'un attribut (`name="..."`/`name='...'`/`name=x`) d'un
// fragment d'en-tete de balise.
fn tag_attr<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    let lower = header.to_ascii_lowercase();
    let key = alloc::format!("{}=", name);
    let pos = lower.find(&key)? + key.len();
    let rest = &header[pos..];
    let rest = rest.trim_start();
    let (quote, body) = match rest.as_bytes().first() {
        Some(b'"') => ('"', &rest[1..]),
        Some(b'\'') => ('\'', &rest[1..]),
        // Valeur non quotee : se termine a l'espace ou au `>` (les URLs
        // contiennent des `/`, ne pas couper dessus). On retire un `/` final
        // eventuel issu d'une balise auto-fermante `<.../>`.
        _ => {
            let v = rest.split(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '>').next().unwrap_or("");
            let v = v.strip_suffix('/').unwrap_or(v);
            return if v.is_empty() { None } else { Some(v) };
        }
    };
    body.split(quote).next().filter(|s| !s.is_empty())
}

fn open_page_inner(html: &[u8], base_url: &str, run: bool) -> (PageCtx, Vec<u8>) {
    let mut interp = Interp::new();
    interp.base_url = base_url.to_string();
    interp.dom = DomModel::parse(html);
    let mut scripts: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0usize;
    let mut ran = 0u32;
    let mut script_bytes = 0usize; // budget cumule d'octets de script executes
    while i < html.len() {
        if starts_ci(&html[i..], b"<script") {
            let outer_start = i;
            let header_end = find_ci(html, b">", i).map(|p| p + 1).unwrap_or(html.len());
            let header = core::str::from_utf8(&html[i..header_end]).unwrap_or("");
            let is_external = header.contains("src=") || header.contains("src =");
            // Ne pas executer les scripts non-JS (JSON-LD, templates, modules data).
            let typ = tag_attr(header, "type").unwrap_or("").to_ascii_lowercase();
            let is_js = typ.is_empty() || typ.contains("javascript") || typ == "module" || typ == "text/babel";
            let content_start = header_end;
            let content_end = find_ci(html, b"</script", content_start).unwrap_or(html.len());
            let outer_end = find_ci(html, b">", content_end).map(|p| p + 1).unwrap_or(html.len());
            let mut wr = String::new();
            // Budget global atteint -> on n'execute plus de script (anti-OOM).
            let budget_ok = script_bytes < MAX_TOTAL_SCRIPT;
            if run && is_js && ran < 2000 && budget_ok {
                if is_external {
                    // <script src="..."> : telecharge (avec cache) et execute, si
                    // le bundle reste sous le plafond (sinon il ferait exploser le
                    // tas au parsing pour rien).
                    if let Some(src_url) = tag_attr(header, "src") {
                        let abs = crate::net::http::resolve_location(
                            scheme_of(base_url), host_of(base_url), src_url);
                        if let Some(bytes) = crate::net::fetch_cached(&abs) {
                            if bytes.len() <= MAX_SCRIPT && script_bytes + bytes.len() <= MAX_TOTAL_SCRIPT {
                                let code = alloc::string::String::from_utf8_lossy(&bytes);
                                interp.writes.clear();
                                match interp.run(&code) {
                                    Ok(_) => crate::dlog!(crate::diag::Cat::Js, "script ext {}o OK {}", bytes.len(), abs),
                                    Err(e) => crate::dlog!(crate::diag::Cat::Err, "script ext {} : {}", abs, e),
                                }
                                ran += 1;
                                script_bytes += bytes.len();
                                wr = core::mem::take(&mut interp.writes);
                            } else {
                                crate::dlog!(crate::diag::Cat::Warn, "script ext ignore ({}o, plafond {}o): {}", bytes.len(), MAX_SCRIPT, abs);
                            }
                        }
                    }
                } else if content_end > content_start {
                    let len = content_end - content_start;
                    if len <= MAX_SCRIPT && script_bytes + len <= MAX_TOTAL_SCRIPT {
                        if let Ok(src) = core::str::from_utf8(&html[content_start..content_end]) {
                            interp.writes.clear();
                            if let Err(e) = interp.run(src) {
                                crate::dlog!(crate::diag::Cat::Err, "script inline ({}o) : {}", src.len(), e);
                            }
                            ran += 1;
                            script_bytes += len;
                            wr = core::mem::take(&mut interp.writes);
                        }
                    } else {
                        crate::dlog!(crate::diag::Cat::Warn, "script inline ignore ({}o, plafond)", len);
                    }
                }
            }
            scripts.push((outer_start, outer_end, wr));
            i = outer_end;
            continue;
        }
        i += 1;
    }
    if run {
        // Boucle d'evenements initiale : draine microtaches/timers puis declenche
        // les evenements de chargement (init de beaucoup de pages).
        interp.pump();
        interp.fire_event(-1, "readystatechange");
        interp.fire_event(-1, "DOMContentLoaded");
        interp.fire_event(-1, "load");
        interp.pump();
        let budget = if interp.steps >= interp.max_steps { " (BUDGET ATTEINT)" } else { "" };
        crate::dlog!(crate::diag::Cat::Js, "{} scripts executes, {} steps{}", ran, interp.steps, budget);
    }
    let mut out = interp.dom.rebuild(&scripts);
    if out.len() > MAX_OUTPUT { out.truncate(MAX_OUTPUT); }
    (PageCtx { interp, scripts }, out)
}

// --- helpers HTML ---
fn escape_html(s: &str) -> String { let mut out = String::new(); for c in s.chars() { match c { '&' => out.push_str("&amp;"), '<' => out.push_str("&lt;"), '>' => out.push_str("&gt;"), '"' => out.push_str("&quot;"), _ => out.push(c) } } out }
fn starts_ci(hay: &[u8], needle: &[u8]) -> bool { hay.len() >= needle.len() && hay[..needle.len()].iter().zip(needle).all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase()) }
fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> { if needle.is_empty() || from >= hay.len() { return None; } let mut i = from; while i + needle.len() <= hay.len() { let mut k = 0; while k < needle.len() && hay[i + k].to_ascii_lowercase() == needle[k].to_ascii_lowercase() { k += 1; } if k == needle.len() { return Some(i); } i += 1; } None }

// ============================================================================
// Selftest
// ============================================================================

pub fn selftest() -> Result<(), &'static str> {
    let html = br#"<div id="app">old</div><script>
        var who = 'Bouchaud';
        document.getElementById('app').innerHTML = '<b>' + who + '</b>';
        document.write('<p>OK</p>');
    </script>"#;
    let out = execute_inline(html, "");
    let s = core::str::from_utf8(&out).map_err(|_| "utf8")?;
    if !s.contains("<div id=\"app\"><b>Bouchaud</b></div>") { return Err("innerHTML"); }
    if !s.contains("<p>OK</p>") || s.contains("<script>") { return Err("write"); }
    // langage : arithmetique, boucle, fonction
    let mut it = Interp::new();
    it.run("var s=0; for(var i=1;i<=10;i++){s+=i;} function f(x){return x*x;} console.log(s+','+f(5));").map_err(|_| "run")?;
    if it.out.last().map(|x| x.as_str()) != Some("55,25") { return Err("lang"); }
    // closures + liaison let par iteration
    let mut it = Interp::new();
    it.run("var f=[]; for(let i=0;i<3;i++){f.push(()=>i);} console.log(f[0]()+''+f[1]()+f[2]());").map_err(|_| "run2")?;
    if it.out.last().map(|x| x.as_str()) != Some("012") { return Err("closure"); }
    // tableaux d'ordre superieur + objets + JSON
    let mut it = Interp::new();
    it.run("var a=[1,2,3,4]; var o={n:a.filter(x=>x%2===0).reduce((s,x)=>s+x,0)}; console.log(JSON.stringify(o));").map_err(|_| "run3")?;
    if it.out.last().map(|x| x.as_str()) != Some("{\"n\":6}") { return Err("hof"); }
    // Promise + microtaches (drainees par pump)
    let mut it = Interp::new();
    it.run("var px=0; Promise.resolve(41).then(function(v){ px=v+1; });").map_err(|_| "run4")?;
    it.pump();
    it.run("console.log(px);").map_err(|_| "run5")?;
    if it.out.last().map(|x| x.as_str()) != Some("42") { return Err("promise"); }
    // setTimeout (macrotache, drainee par pump)
    let mut it = Interp::new();
    it.run("var ty=0; setTimeout(function(){ ty=7; }, 0);").map_err(|_| "run6")?;
    it.pump();
    it.run("console.log(ty);").map_err(|_| "run7")?;
    if it.out.last().map(|x| x.as_str()) != Some("7") { return Err("timer"); }
    // addEventListener('click') + dispatch via element.click()
    let (mut ctx, _o) = open_page(br#"<button id="b">x</button><script>
        var n=0;
        document.getElementById('b').addEventListener('click', function(){ n++; document.getElementById('b').textContent = 'clic'+n; });
    </script>"#, "");
    let html = ctx.dispatch("document.getElementById('b').click()");
    let s = core::str::from_utf8(&html).unwrap_or("");
    if !s.contains("clic1") { return Err("event"); }
    // style live -> attribut style (repris par le layout)
    let out = execute_inline(br#"<div id="d">hi</div><script>document.getElementById('d').style.color='red';</script>"#, "");
    let s = core::str::from_utf8(&out).unwrap_or("");
    if !s.contains("color:red") { return Err("style"); }
    Ok(())
}
