//! Modules ES : chargement (cache par URL, resolution relative), execution
//! dans une portee isolee, import() dynamique et stubs de bare specifiers.

use super::*;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// Valeur de secours pour un export de module que le parseur n'a pas pu
// materialiser. Le but n'est pas de simuler la librairie entiere, mais
// d'eviter qu'un seul export minifie manque (ex: `bold`) bloque tout le
// boot d'une SPA. La fonction est callable, chainable, et porte quelques
// methodes usuelles pour survivre aux bundles UI.
fn module_noop(_it: &mut Interp, _t: Value, _a: &[Value]) -> Result<Value, Value> { Ok(Value::Undefined) }

pub(super) fn module_export_stub(name: &str) -> Value {
    let f = native_val(module_noop);
    set(&f, "name", str_val(name.to_string()));
    set(&f, "default", f.clone());
    set(&f, "then", native_val(|_it, t, _a| Ok(t)));
    set(&f, "catch", native_val(|_it, t, _a| Ok(t)));
    set(&f, "toString", native_val(|_it, _t, _a| Ok(str_val("function () { [module fallback] }"))));
    f
}

fn export_is_missing(v: Option<Value>) -> bool {
    match v {
        None | Some(Value::Undefined) | Some(Value::Null) => true,
        _ => false,
    }
}

fn ensure_export(ns: &Value, name: &str) {
    if name.is_empty() { return; }
    let missing = match ns {
        Value::Obj(o) => export_is_missing(o.borrow().props.get(name).cloned()),
        _ => false,
    };
    if missing { set(ns, name, module_export_stub(name)); }
}

fn bare_module_stub(spec: &str) -> Value {
    let ns = new_obj(Obj::plain());
    let def = new_obj(Obj::plain());
    // Stubs pragmatiques pour bundles React/Preact/Vue/Svelte/Vite.
    let f = module_export_stub("component");
    for k in &["createElement","jsx","jsxs","Fragment","hydrate","hydrateRoot","createRoot","render",
               "h","createApp","defineComponent","ref","computed","watch","onMounted","useState","useEffect",
               "useMemo","useRef","memo","forwardRef"] {
        set(&ns, k, f.clone());
        set(&def, k, f.clone());
    }
    set(&ns, "default", def);
    set(&ns, "__esModule", Value::Bool(true));
    set(&ns, "__specifier", str_val(spec.to_string()));
    ns
}

fn is_ident_byte(b: u8) -> bool { b.is_ascii_alphanumeric() || b == b'_' || b == b'$' }
fn skip_ws_bytes(src: &[u8], mut i: usize) -> usize { while i < src.len() && src[i].is_ascii_whitespace() { i += 1; } i }

fn parse_export_brace_names(body: &str, out: &mut Vec<String>) {
    for part in body.split(',') {
        let p = part.trim();
        if p.is_empty() { continue; }
        let toks: Vec<&str> = p.split_whitespace().collect();
        if toks.len() >= 3 && toks[toks.len()-2] == "as" { out.push(toks[toks.len()-1].trim().to_string()); }
        else if let Some(last) = toks.first() { out.push(last.trim().to_string()); }
    }
}

fn parse_decl_export_names(rest: &str, out: &mut Vec<String>) {
    // `export const a=..., b=...` : on recupere les identifiants de premier
    // niveau avant `=`, `,`, `;`. Les destructurations sont ignorees proprement.
    let mut cur = String::new();
    let mut depth = 0i32;
    for ch in rest.chars() {
        match ch {
            '(' | '[' | '{' => { depth += 1; cur.clear(); }
            ')' | ']' | '}' => { depth -= 1; cur.clear(); }
            '=' | ',' | ';' if depth == 0 => {
                let name = cur.trim();
                if !name.is_empty() && name.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$').unwrap_or(false) { out.push(name.to_string()); }
                cur.clear();
                if ch == ';' { break; }
            }
            c if depth == 0 && (c.is_ascii_alphanumeric() || c == '_' || c == '$') => cur.push(c),
            _ if depth == 0 => { if !cur.trim().is_empty() && ch.is_whitespace() { cur.push(' '); } }
            _ => {}
        }
    }
    let name = cur.trim();
    if !name.is_empty() && !name.contains(' ') { out.push(name.to_string()); }
}

fn scan_export_names(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = src[i..].find("export") {
        let start = i + rel;
        let before_ok = start == 0 || !is_ident_byte(b[start-1]);
        let after = start + 6;
        let after_ok = after >= b.len() || !is_ident_byte(b[after]);
        i = after;
        if !before_ok || !after_ok { continue; }
        let j = skip_ws_bytes(b, after);
        let rest = &src[j..];
        if rest.starts_with("default") { out.push("default".to_string()); continue; }
        if rest.starts_with("{") {
            if let Some(end) = rest.find('}') { parse_export_brace_names(&rest[1..end], &mut out); }
            continue;
        }
        if rest.starts_with("*") {
            let r = rest[1..].trim_start();
            if let Some(r) = r.strip_prefix("as") {
                let r = r.trim_start();
                let name: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$').collect();
                if !name.is_empty() { out.push(name); }
            }
            continue;
        }
        for kw in &["const", "let", "var"] {
            if let Some(r) = rest.strip_prefix(kw) { parse_decl_export_names(r, &mut out); }
        }
        for kw in &["function", "class"] {
            if let Some(r) = rest.strip_prefix(kw) {
                let r = r.trim_start().trim_start_matches('*').trim_start();
                let name: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$').collect();
                if !name.is_empty() { out.push(name); }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn preseed_module_exports(src: &str, ns: &Value) {
    for name in scan_export_names(src) { ensure_export(ns, &name); }
    set(ns, "__esModule", Value::Bool(true));
}

fn finalize_module_exports(src: &str, ns: &Value) {
    for name in scan_export_names(src) { ensure_export(ns, &name); }
    if export_is_missing(match ns { Value::Obj(o) => o.borrow().props.get("default").cloned(), _ => None }) {
        // Beaucoup de chunks Vite n'ont pas de default ; le laisser absent casse
        // certains wrappers CommonJS. On fournit un objet namespace par defaut.
        set(ns, "default", new_obj(Obj::plain()));
    }
    set(ns, "__esModule", Value::Bool(true));
}

impl Interp {

    /// Charge (avec cache) un module ES et renvoie son objet namespace.
    /// Le specifier est resolu contre l'URL du module courant (imports
    /// relatifs `./x.js`, `../x.js`) ou l'URL de la page.
    pub(super) fn load_module(&mut self, spec: &str) -> Result<Value, String> {
        // Bare specifier (`react`, `@vite/client`...) : pas de node_modules dans
        // l'OS. Namespace vide EXPLICITEMENT loggue — le bundle continue et
        // revele la couche suivante au lieu de mourir sur la resolution.
        // (Un vrai support passera par les import maps de la page.)
        let is_url = spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') || spec.contains("://");
        if !is_url {
            if let Some(v) = self.modules.get(spec) { return Ok(v.clone()); }
            crate::dlog!(crate::diag::Cat::Js, "module stub: {}", spec);
            let ns = bare_module_stub(spec);
            self.modules.insert(spec.to_string(), ns.clone());
            return Ok(ns);
        }
        let base = self.module_stack.last().cloned().unwrap_or_else(|| self.base_url.clone());
        let url = resolve_url(&base, spec);
        if let Some(v) = self.modules.get(&url) { return Ok(v.clone()); }
        if self.module_stack.len() >= 32 { return Err(format!("imports trop profonds: {}", url)); }
        // Namespace enregistre AVANT execution : les cycles d'import voient un
        // namespace partiel (semantique proche des vrais modules ES).
        let ns = new_obj(Obj::plain());
        self.modules.insert(url.clone(), ns.clone());
        let bytes = crate::net::fetch_cached(&url).ok_or_else(|| format!("module introuvable: {}", url))?;
        if bytes.len() > MAX_SCRIPT { return Err(format!("module trop gros ({}o): {}", bytes.len(), url)); }
        let src = alloc::string::String::from_utf8_lossy(&bytes).into_owned();
        preseed_module_exports(&src, &ns);
        match self.run_module(&src, &url, ns.clone()) {
            Ok(()) => { finalize_module_exports(&src, &ns); crate::dlog!(crate::diag::Cat::Js, "module ES {}o OK {}", bytes.len(), url); Ok(ns) }
            Err(e) => { finalize_module_exports(&src, &ns); crate::dlog!(crate::diag::Cat::Err, "module ES {} : {}", url, e); Err(e) }
        }
    }

    /// Execute une source comme module ES : portee propre (les declarations ne
    /// fuient pas dans le global, mais voient le global), exports collectes
    /// dans `ns`, imports relatifs resolus contre `url`.
    pub fn run_module(&mut self, src: &str, url: &str, ns: Value) -> Result<(), String> {
        let toks = Lexer::new(src.as_bytes()).lex_all()?;
        let mut parser = Parser::new(toks);
        let prog = parser.parse_program()?;
        if parser.recovered > 0 {
            crate::dlog!(crate::diag::Cat::Js, "module: {} stmt(s) recuperes/sautes ({}o); 1re erreur: {} | ctx: {}",
                parser.recovered, src.len(),
                parser.first_err.as_deref().unwrap_or("?"),
                parser.first_ctx.as_deref().unwrap_or("?"));
        }
        let menv = new_fn_scope(Some(self.global.clone()));
        // `import.meta.url` du module courant (l'identifiant `import` reste
        // aussi appelable : import dynamique).
        let imp = native_val(native_dynamic_import);
        let meta = new_obj(Obj::plain());
        set(&meta, "url", str_val(url.to_string()));
        set(&imp, "meta", meta);
        scope_declare(&menv, "import", imp);
        self.module_stack.push(url.to_string());
        self.exports_stack.push(ns);
        self.hoist(&prog, &menv);
        self.hoist_vars_deep(&prog, &menv);
        let mut err = None;
        for st in &prog {
            match self.exec(st, &menv) {
                Flow::Throw(v) => { err = Some(format!("Uncaught {}", self.to_string(&v))); break; }
                Flow::Return(_) => break,
                _ => {}
            }
        }
        self.exports_stack.pop();
        self.module_stack.pop();
        match err { Some(e) => Err(e), None => Ok(()) }
    }

    /// Point d'entree d'un `<script type="module">` de page (inline ou externe).
    pub fn run_module_script(&mut self, src: &str, url: &str) -> Result<(), String> {
        let ns = self.modules.get(url).cloned().unwrap_or_else(|| new_obj(Obj::plain()));
        self.modules.insert(url.to_string(), ns.clone());
        preseed_module_exports(src, &ns);
        let r = self.run_module(src, url, ns.clone());
        finalize_module_exports(src, &ns);
        r
    }
}


/// `import("u")` dynamique : charge le module (synchrone dans cet OS) et
/// renvoie une promesse resolue avec son namespace. En cas d'echec, promesse
/// resolue avec undefined + log (pas de rejet non gere qui casserait la page).
pub(super) fn native_dynamic_import(it: &mut Interp, _t: Value, a: &[Value]) -> Result<Value, Value> {
    let spec = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
    match it.load_module(&spec) {
        Ok(ns) => Ok(make_resolved_thenable(ns)),
        Err(e) => { crate::dlog!(crate::diag::Cat::Err, "import() {} : {}", spec, e); Ok(make_resolved_thenable(new_obj(Obj::plain()))) }
    }
}
