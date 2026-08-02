//! Point d'entree du binaire `rustpython.wasm` embarque par le noyau
//! (`src/lang/python.rs`). Lit un script Python sur l'entree standard (fd 0),
//! le compile et l'execute avec RustPython, stdlib figee (`freeze-stdlib`).
//!
//! Pas de systeme de fichiers reel cote OS (aucun repertoire "preouvert"
//! WASI) : le script arrive via stdin, jamais via un chemin `path_open`.

use rustpython_vm::Interpreter;
use std::io::Read;

fn main() {
    let mut src = String::new();
    std::io::stdin().read_to_string(&mut src).expect("lecture stdin");

    let interp = Interpreter::builder(Default::default())
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
        .build();
    let exit_code = interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        match vm.compile(&src, rustpython_vm::compiler::Mode::Exec, "<script>".to_owned()) {
            Ok(code) => match vm.run_code_obj(code, scope) {
                Ok(_) => 0,
                Err(e) => {
                    vm.print_exception(e);
                    1
                }
            },
            Err(e) => {
                println!("SyntaxError: {:?}", e);
                1
            }
        }
    });
    std::process::exit(exit_code);
}
