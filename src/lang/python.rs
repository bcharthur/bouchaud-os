//! Python (RustPython) embarque en WebAssembly, execute par le runtime `wasmi`
//! de `crate::wasm`. Un vrai interprete Python 3 : classes, exceptions,
//! f-strings, comprehensions, generateurs, et la stdlib -- a la fois figee
//! dans le binaire (`freeze-stdlib`) et native (`json`, `re`, `math`, `os`,
//! `open()`, `hashlib`, `zlib`, `csv`, `random`...).
//!
//! Le binaire est precompile hors-ligne (voir `tools/python-wasm/README.md`).
//! Il pese 16,4 Mo : l'embarquer dans le noyau doublait le temps de demarrage
//! avant la premiere ligne du noyau (le BIOS charge toute l'image secteur par
//! secteur : 10,0-10,7 s avec, 4,8-5,2 s sans, QEMU TCG, mesure du 13/11/2026).
//! Il vit donc par defaut sur le disque de donnees, `INTERPRETE` ci-dessous, et
//! n'est lu qu'au premier `python`. La feature `python-embarque` retablit
//! l'ancien `include_bytes!` pour une image sans disque de donnees.
//!
//! Les acces fichiers du module WASM (`open()`,
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

#[cfg(feature = "python-embarque")]
static RUSTPYTHON_WASM: &[u8] = include_bytes!("../assets/python/rustpython.wasm");

/// Chemin de l'interpreteur sur le disque de donnees (`tools/userland/mkdisk.sh`
/// et `run.ps1` l'y deposent).
pub const INTERPRETE: &str = "/usr/lib/python/rustpython.wasm";

/// Plafond de lecture : l'interpreteur actuel pese 16,4 Mo. Au-dela, ce n'est
/// plus lui ; on refuse plutot que d'allouer sans borne dans le tas noyau.
#[cfg(not(feature = "python-embarque"))]
const TAILLE_MAX_INTERPRETE: usize = 64 * 1024 * 1024;

/// Octets de l'interpreteur, lus UNE fois puis conserves : le cache de module
/// de `crate::wasm` est indexe par l'adresse des octets, qui doit donc rester
/// stable. Le cout memoire est celui qu'avait l'image embarquee, mais il n'est
/// paye qu'apres le premier `python`, et plus du tout au demarrage.
#[cfg(not(feature = "python-embarque"))]
static CHARGE: crate::kernel::sync::SpinLock<Option<&'static [u8]>> = crate::kernel::sync::SpinLock::new(None);

#[cfg(feature = "python-embarque")]
fn interprete() -> Result<&'static [u8], String> {
    Ok(RUSTPYTHON_WASM)
}

#[cfg(not(feature = "python-embarque"))]
fn interprete() -> Result<&'static [u8], String> {
    // Le verrou n'est JAMAIS tenu pendant la lecture disque (16 Mo).
    if let Some(octets) = *CHARGE.lock() {
        return Ok(octets);
    }
    let idx = {
        let fs = crate::fs::ramfs::fs();
        fs.resolve(INTERPRETE, 0).ok_or_else(|| {
            format!("interpreteur absent : {} (disque de donnees sans Python, voir tools/python-wasm/README.md)", INTERPRETE)
        })?
    };
    let debut_ns = crate::kernel::timer::monotonic_ns();
    let taille = crate::fs::backing::logical_len(idx);
    if taille == 0 || taille > TAILLE_MAX_INTERPRETE {
        return Err(format!("{} : taille {} octets refusee", INTERPRETE, taille));
    }
    let mut octets = vec![0u8; taille];
    let lus = crate::fs::backing::read_at(idx, 0, &mut octets);
    if lus != taille {
        return Err(format!("{} : lecture incomplete ({} sur {} octets)", INTERPRETE, lus, taille));
    }
    if !octets.starts_with(b"\0asm") {
        return Err(format!("{} : pas un module WebAssembly", INTERPRETE));
    }
    let mut charge = CHARGE.lock();
    if let Some(deja) = *charge {
        // Un autre appel l'a charge pendant notre lecture : on garde le sien.
        return Ok(deja);
    }
    let fige: &'static [u8] = alloc::boxed::Box::leak(octets.into_boxed_slice());
    *charge = Some(fige);
    // Une seule fois par demarrage : le cout deplace du BIOS vers le premier
    // `python`, mesure plutot que suppose.
    crate::println!(
        "BOUCHAUD_PYTHON_INTERPRETE_CHARGE octets={} lecture_ms={}",
        taille,
        crate::kernel::timer::monotonic_ns().saturating_sub(debut_ns) / 1_000_000
    );
    Ok(fige)
}

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
    let octets = match interprete() {
        Ok(o) => o,
        Err(e) => {
            crate::println!("python: {}", e);
            return 1;
        }
    };
    let res = crate::wasm::run_program(
        octets,
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
        interprete()?,
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
