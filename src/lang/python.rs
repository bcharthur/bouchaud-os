//! Python (RustPython) embarque en WebAssembly, execute par le runtime `wasmi`
//! de `crate::wasm`. Un vrai interprete Python 3 : classes, exceptions,
//! f-strings, comprehensions, generateurs, et la stdlib -- a la fois figee
//! dans le binaire (`freeze-stdlib`) et native (`json`, `re`, `math`, `os`,
//! `open()`, `hashlib`, `zlib`, `csv`, `random`...).
//!
//! Le binaire est precompile hors-ligne (voir `tools/python-wasm/README.md`)
//! et embarque via `include_bytes!`, comme les polices (`src/assets/fonts/`)
//! ou les certificats CA. Les acces fichiers du module WASM (`open()`,
//! `import` depuis `sys.path`...) sont traduits vers le RAMFS par le pont
//! WASI de `src/wasm/mod.rs`, avec les permissions Unix de la session.
//!
//! Les paquets purs-Python installes par la commande `pip` (voir `pip.rs`)
//! sont deposes dans `/usr/lib/python/site-packages`, transmis a
//! l'interpreteur via `PYTHONPATH` -> importables normalement.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

static RUSTPYTHON_WASM: &[u8] = include_bytes!("../assets/python/rustpython.wasm");

/// Repertoire des paquets installes par `pip`.
pub const SITE_PACKAGES: &str = "/usr/lib/python/site-packages";

fn base_env(cwd_path: &str) -> Vec<(String, String)> {
    vec![
        ("PYTHONPATH".to_string(), SITE_PACKAGES.to_string()),
        ("PWD".to_string(), cwd_path.to_string()),
        ("HOME".to_string(), crate::users::session().home().to_string()),
        ("USER".to_string(), crate::users::session().username().to_string()),
        // La console VGA n'interprete pas les sequences ANSI : tracebacks
        // et REPL doivent rester en texte brut (convention CPython 3.13).
        ("PYTHON_COLORS".to_string(), "0".to_string()),
        ("NO_COLOR".to_string(), "1".to_string()),
        ("TERM".to_string(), "dumb".to_string()),
    ]
}

/// Execute l'interpreteur avec `args` et rapporte le code de sortie.
/// `cwd_node` sert de repertoire courant (preouverture WASI `.`) : les chemins
/// relatifs des scripts (`open("data.txt")`) s'y resolvent.
fn exec(args: Vec<String>, cwd_node: usize, cwd_path: &str) -> i32 {
    let res = crate::wasm::run_program(
        RUSTPYTHON_WASM,
        args,
        base_env(cwd_path),
        Vec::new(),
        true, // interactif : sortie au fil de l'eau, stdin clavier (input()...)
        crate::wasm::PYTHON_FUEL,
        cwd_node,
    );
    if let Some(e) = res.error {
        crate::println!("python: {}", e);
        return 1;
    }
    res.exit_code.unwrap_or(0)
}

/// REPL interactif (`python` sans argument).
pub fn run_repl(cwd_node: usize, cwd_path: &str) -> i32 {
    exec(vec!["python".to_string()], cwd_node, cwd_path)
}

/// `python -c "code"`.
pub fn run_code(code: &str, cwd_node: usize, cwd_path: &str) -> i32 {
    exec(vec!["python".to_string(), "-c".to_string(), code.to_string()], cwd_node, cwd_path)
}

/// `python fichier.py [args...]` : le chemin doit etre absolu (resolu par le
/// shell), le module WASM l'ouvre lui-meme via WASI -> RAMFS.
pub fn run_file(abs_path: &str, extra_args: &[&str], cwd_node: usize, cwd_path: &str) -> i32 {
    let mut args = vec!["python".to_string(), abs_path.to_string()];
    for a in extra_args {
        args.push((*a).to_string());
    }
    exec(args, cwd_node, cwd_path)
}

/// Selftest hors-REPL : execute un petit programme et verifie la sortie.
pub fn selftest() -> Result<(), String> {
    let res = crate::wasm::run_program(
        RUSTPYTHON_WASM,
        vec!["python".to_string(), "-c".to_string(), "print(sum(x*x for x in range(5)))".to_string()],
        Vec::new(),
        Vec::new(),
        false,
        crate::wasm::PYTHON_FUEL,
        0,
    );
    if let Some(e) = res.error {
        return Err(e);
    }
    if res.output.trim() != "30" {
        return Err(format!("sortie inattendue : {:?}", res.output));
    }
    Ok(())
}
