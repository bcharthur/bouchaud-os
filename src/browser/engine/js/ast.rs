//! AST du langage : expressions, statements, definitions de fonctions et
//! motifs de destructuration. Types purs, sans logique d'evaluation.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

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
    // Litteral d'expression rationnelle : /pattern/flags.
    Regex(String, String),
    Array(Vec<Expr>), Object(Vec<(String, Expr)>),
    Unary(String, Box<Expr>), Update(String, bool, Box<Expr>),
    Bin(String, Box<Expr>, Box<Expr>), Logic(String, Box<Expr>, Box<Expr>),
    Assign(String, Box<Expr>, Box<Expr>), Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>, bool), New(Box<Expr>, Vec<Expr>),
    Member(Box<Expr>, String, bool), Index(Box<Expr>, Box<Expr>),
    Func(Rc<FuncDef>), Seq(Vec<Expr>), Spread(Box<Expr>),
    // Template balisé `tag`a${x}b`` : (tag, quasis littéraux, expressions).
    TaggedTpl(Box<Expr>, Vec<String>, Vec<Expr>),
    // Méta-propriété `new.target`.
    NewTarget,
    // Expression de classe : `var X = class [extends Y] { ... }`.
    Class(Option<Box<Expr>>, Vec<(bool, String, Rc<FuncDef>)>),
}

/// Motif de liaison (déclarations `let/var/const`, paramètres, `for…of`).
/// Permet la déstructuration réelle `[a,b]=…` / `{a,b}=…` (au lieu de la
/// sauter), indispensable pour le code moderne (`for (let [k,v] of entries)`).
#[derive(Clone)]
pub enum Pat {
    Id(String),
    // éléments (None = trou `[ ,a]`), reste optionnel `...r`
    Array(Vec<Option<Pat>>, Option<Box<Pat>>),
    // (clé source, motif cible), reste optionnel `...r`
    Object(Vec<(String, Pat)>, Option<String>),
    // motif = valeur par défaut (appliquée si undefined)
    Default(Box<Pat>, Box<Expr>),
}

#[derive(Clone)]
pub enum Stmt {
    Expr(Expr), Var(bool, Vec<(Pat, Option<Expr>)>), Func(String, Rc<FuncDef>),
    Return(Option<Expr>), If(Expr, Box<Stmt>, Option<Box<Stmt>>), Block(Vec<Stmt>),
    While(Expr, Box<Stmt>), DoWhile(Box<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    ForIn(Pat, Expr, Box<Stmt>, bool), Break, Continue, Throw(Expr),
    Try(Vec<Stmt>, Option<(String, Vec<Stmt>)>, Option<Vec<Stmt>>), Empty,
    // name, extends_expr, [(is_static, method_name, def)]
    Class(String, Option<Expr>, Vec<(bool, String, Rc<FuncDef>)>),
    // discriminant, [(test_or_None_for_default, body_stmts)]
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    // --- Modules ES ---
    // import defaut / * as ns / {a, b as c} depuis une URL (specifier)
    Import { default_: Option<String>, ns: Option<String>, named: Vec<(String, String)>, from: String },
    ExportDecl(Box<Stmt>),                              // export const/let/var/function/class
    ExportDefault(Expr),                                // export default <expr>
    ExportNamed(Vec<(String, String)>, Option<String>), // export {local as exporte} [from "u"]
    ExportAll(String),                                  // export * from "u"
}
