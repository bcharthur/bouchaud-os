//! Langages de programmation embarqués dans Bouchaud OS.
//!
//! `mini_rust` : sous-ensemble Rust interprété (fn, let, if/else, while,
//! for..in, println!, arithmétique) pour exécuter des programmes Rust simples
//! directement dans l'OS sans compilateur externe.
//!
//! `python` : vrai interpréteur Python (RustPython), précompilé en WASM et
//! exécuté par le runtime `wasmi` (`crate::wasm`). Pas de `pip` ni d'accès
//! disque/réseau depuis les scripts (voir `python.rs`).

pub mod mini_rust;
pub mod python;
