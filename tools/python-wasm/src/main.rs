//! Binaire `rustpython.wasm` embarque par le noyau (`src/lang/python.rs`).
//!
//! Un vrai Python : RustPython avec sa stdlib **native** (`os`, `io`/`open()`,
//! `json`, `re` via `_sre`, `math`, `random`, `hashlib`, `zlib`, `csv`...) en
//! plus de la stdlib Python figee dans le binaire (`freeze-stdlib`). La feature
//! `host_env` active le module `posix` et les imports depuis le systeme de
//! fichiers : cote noyau, chaque appel WASI (`path_open`, `fd_read`,
//! `fd_readdir`...) est traduit vers le RAMFS (voir `src/wasm/mod.rs`).
//!
//! Modes, comme le vrai binaire python :
//!   (aucun argument)   REPL interactif (stdin/stdout cables sur le clavier/VGA)
//!   -c "code"          execute le code passe en argument
//!   -                  execute le script lu sur stdin
//!   chemin.py [args]   execute le fichier (ouvert via WASI -> RAMFS)

use rustpython_vm::builtins::PyBaseExceptionRef;
use rustpython_vm::scope::Scope;
use rustpython_vm::{AsObject, Interpreter, Settings, VirtualMachine};
use std::io::{BufRead, Read, Write};
use std::process::ExitCode;

fn make_interpreter(argv: Vec<String>) -> Interpreter {
    let mut settings = Settings::default();
    settings.argv = argv;
    settings.install_signal_handlers = false; // pas de signaux sous WASI
    // PYTHONPATH (transmis par le noyau via l'environ WASI) -> sys.path :
    // c'est ce qui rend importables les paquets installes par `pip` dans
    // /usr/lib/python/site-packages du RAMFS.
    if let Ok(pypath) = std::env::var("PYTHONPATH") {
        for p in pypath.split(':').filter(|p| !p.is_empty()) {
            settings.path_list.push(p.to_owned());
        }
    }
    let builder = Interpreter::builder(settings);
    let defs = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
    builder
        .add_native_modules(&defs)
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
        .build()
}

/// Vide les tampons de sys.stdout/sys.stderr (TextIOWrapper est bufferise :
/// sans flush, la sortie serait perdue au `proc_exit`).
fn flush_io(vm: &VirtualMachine) {
    for name in ["stdout", "stderr"] {
        if let Ok(f) = vm.sys_module.get_attr(name, vm) {
            let _ = vm.call_method(&f, "flush", ());
        }
    }
}

/// Exceptions non-SystemExit : traceback sur stderr ; SystemExit : code de sortie.
fn exception_to_exit(vm: &VirtualMachine, exc: PyBaseExceptionRef) -> u8 {
    if exc.fast_isinstance(vm.ctx.exceptions.system_exit) {
        return vm.handle_exit_exception(exc) as u8;
    }
    vm.print_exception(exc);
    1
}

fn run_source(vm: &VirtualMachine, scope: Scope, source: &str, path: &str) -> u8 {
    let code = match vm.compile(source, rustpython_vm::compiler::Mode::Exec, path.to_owned()) {
        Ok(code) => code,
        Err(e) => {
            let exc = vm.new_syntax_error(&e, Some(source));
            vm.print_exception(exc);
            return 1;
        }
    };
    let r = match vm.run_code_obj(code, scope) {
        Ok(_) => 0,
        Err(e) => exception_to_exit(vm, e),
    };
    flush_io(vm);
    r
}

// ---------------------------------------------------------------------------
// REPL
// ---------------------------------------------------------------------------

/// Compte les delimiteurs ouverts hors chaines pour savoir si l'entree est
/// syntaxiquement "en attente" (parentheses/crochets/accolades non fermes).
fn open_delimiters(src: &str) -> i32 {
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut escape = false;
    for c in src.chars() {
        if let Some(q) = in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => in_str = Some(c),
            '#' => break, // approximation : commentaire jusqu'a la fin de ligne
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn repl(vm: &VirtualMachine, scope: Scope) -> u8 {
    println!("Python 3.13 (RustPython 0.5) sur Bouchaud OS");
    println!("Tape exit() ou quit() pour sortir.");
    let stdin = std::io::stdin();
    let mut block = String::new();
    let mut in_block = false;
    loop {
        print!("{}", if in_block { "... " } else { ">>> " });
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                println!();
                return 0; // EOF (fin d'entree)
            }
            Ok(_) => {}
            Err(_) => return 0,
        }
        let trimmed = line.trim_end();

        if !in_block && trimmed.is_empty() {
            continue;
        }
        block.push_str(trimmed);
        block.push('\n');

        // Continuation : bloc indente (ligne finissant par ':'), ligne coupee
        // par '\', ou delimiteurs non fermes. Un bloc se termine sur une ligne
        // vide, comme dans le python interactif.
        let wants_more = if in_block {
            !trimmed.is_empty()
        } else {
            trimmed.ends_with(':') || trimmed.ends_with('\\') || open_delimiters(&block) > 0
        };
        if wants_more {
            in_block = true;
            continue;
        }

        let source = core::mem::take(&mut block);
        in_block = false;
        // Mode::Single : les expressions tapees au prompt affichent leur valeur.
        match vm.compile(&source, rustpython_vm::compiler::Mode::Single, "<stdin>".to_owned()) {
            Ok(code) => {
                if let Err(e) = vm.run_code_obj(code, scope.clone()) {
                    if e.fast_isinstance(vm.ctx.exceptions.system_exit) {
                        flush_io(vm);
                        return vm.handle_exit_exception(e) as u8;
                    }
                    vm.print_exception(e);
                }
            }
            Err(e) => {
                let exc = vm.new_syntax_error(&e, Some(&source));
                vm.print_exception(exc);
            }
        }
        flush_io(vm);
    }
}

// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // sys.argv comme le vrai python : ["-c"|""|chemin, args utilisateur...]
    let (mode, py_argv): (Mode, Vec<String>) = match args.get(1).map(|s| s.as_str()) {
        None => (Mode::Repl, vec![String::new()]),
        Some("-c") => {
            let code = args.get(2).cloned().unwrap_or_default();
            let mut v = vec!["-c".to_owned()];
            v.extend(args.iter().skip(3).cloned());
            (Mode::Command(code), v)
        }
        Some("-") => (Mode::Stdin, args.iter().skip(1).cloned().collect()),
        Some(path) => (Mode::File(path.to_owned()), args.iter().skip(1).cloned().collect()),
    };

    let interp = make_interpreter(py_argv);
    let code = interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        match &mode {
            Mode::Repl => repl(vm, scope),
            Mode::Command(code) => run_source(vm, scope, code, "<string>"),
            Mode::Stdin => {
                let mut src = String::new();
                if std::io::stdin().read_to_string(&mut src).is_err() {
                    eprintln!("python: erreur de lecture de stdin");
                    return 1;
                }
                run_source(vm, scope, &src, "<stdin>")
            }
            Mode::File(path) => match std::fs::read_to_string(path) {
                Ok(src) => run_source(vm, scope, &src, path),
                Err(e) => {
                    eprintln!("python: impossible d'ouvrir {}: {}", path, e);
                    2
                }
            },
        }
    });
    ExitCode::from(code)
}

enum Mode {
    Repl,
    Command(String),
    Stdin,
    File(String),
}
