//! Langages de programmation embarqués dans Bouchaud OS.
//!
//! `mini_rust` : sous-ensemble Rust interprété (fn, let, if/else, while,
//! for..in, println!, arithmétique) pour exécuter des programmes Rust simples
//! directement dans l'OS sans compilateur externe.
//!
//! `python` : vrai interpréteur Python (RustPython + stdlib), précompilé en
//! WASM et exécuté par le runtime `wasmi` (`crate::wasm`), avec accès fichiers
//! RAMFS via le pont WASI. REPL, scripts, `-c` (voir `python.rs`).
//!
//! `pyweb` : pont réseau pour les scripts Python (`/dev/web`) et navigateur
//! de test `browser.py` (voir `pyweb.rs`).
//!
//! `js` : interpréteur d'expressions et de scripts, utilisé par la
//! calculatrice du bureau et par la commande `js-selftest`.
//!
//! `pip` : installeur de paquets Python purs depuis PyPI, côté noyau
//! (réseau TLS maison + unzip maison), vers `/usr/lib/python/site-packages`
//! (voir `pip.rs`).

pub mod expr;
pub mod mini_rust;
pub mod pip;
pub mod pyweb;
pub mod python;
