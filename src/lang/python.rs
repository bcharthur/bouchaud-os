//! Python complet (RustPython) embarque en WebAssembly, execute par le
//! runtime `wasmi` de `crate::wasm`. A la difference de `mini_rust` (un
//! interprete "maison" d'un sous-ensemble du langage), il s'agit d'un vrai
//! interprete Python : classes, exceptions, f-strings, comprehensions,
//! generateurs... via <https://github.com/RustPython/RustPython>.
//!
//! Le binaire est precompile hors-ligne (voir `tools/python-wasm/README.md`)
//! car ce noyau ne peut pas heberger la chaine de compilation Rust ->
//! wasm32-wasip1 de RustPython (des centaines de crates, ~2 min de build) au
//! demarrage : il est simplement embarque via `include_bytes!`, comme les
//! polices (`src/assets/fonts/`) ou les certificats CA (`net/security/tls/ca/`).
//!
//! Limitation assumee : pas de `pip`, pas d'acces disque/reseau depuis les
//! scripts (`os`, `socket`...) — le module WASM n'a aucun repertoire
//! "preouvert" (voir `fd_prestat_get` dans `wasm/mod.rs`). Le script est
//! transmis via l'entree standard (fd 0), pas via un fichier WASI.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

static RUSTPYTHON_WASM: &[u8] = include_bytes!("../assets/python/rustpython.wasm");

/// Resultat de l'execution d'un script Python.
pub struct PyRunResult {
    pub output: String,
    pub exit_code: i32,
    /// Erreur d'infrastructure (module WASM invalide, trap inattendu...),
    /// distincte d'une exception Python normale (celle-ci apparait dans `output`
    /// sous forme de traceback, avec `exit_code` a 1).
    pub error: Option<String>,
}

/// Execute un script Python via l'interpreteur RustPython embarque.
/// `script_name` n'est utilise que pour `argv[0]`/messages d'erreur cote VM.
pub fn run(script: &str, script_name: &str) -> PyRunResult {
    let args: Vec<String> = vec![String::from("python"), String::from(script_name)];
    let stdin = script.as_bytes().to_vec();
    let res = crate::wasm::run_bytes_with_io(RUSTPYTHON_WASM, args, stdin);
    let exit_code = res.exit_code.unwrap_or(if res.error.is_some() { 1 } else { 0 });
    PyRunResult { output: res.output, exit_code, error: res.error }
}
