//! Modules ES : chargement (cache par URL, resolution relative), execution
//! dans une portee isolee, import() dynamique et stubs de bare specifiers.

use super::*;

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
            let ns = new_obj(Obj::plain());
            set(&ns, "default", new_obj(Obj::plain()));
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
        match self.run_module(&src, &url, ns.clone()) {
            Ok(()) => { crate::dlog!(crate::diag::Cat::Js, "module ES {}o OK {}", bytes.len(), url); Ok(ns) }
            Err(e) => { crate::dlog!(crate::diag::Cat::Err, "module ES {} : {}", url, e); Err(e) }
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
        let ns = new_obj(Obj::plain());
        self.run_module(src, url, ns)
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
