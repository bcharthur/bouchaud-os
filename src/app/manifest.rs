//! Lecture des manifestes d'application `.bapp`.
//!
//! Format texte simple `cle=valeur`, une par ligne :
//! ```text
//! name=Terminal
//! exec=terminal
//! type=gui
//! permission=normal
//! ```

use alloc::string::{String, ToString};

#[derive(Default)]
pub struct Manifest {
    pub name: String,
    pub exec: String,
    pub kind: String,       // "gui" | "cli"
    pub permission: String, // "normal" | "root"
}

/// Analyse le contenu d'un fichier `.bapp`.
pub fn parse(content: &str) -> Manifest {
    let mut m = Manifest::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim();
            let val = line[eq + 1..].trim().to_string();
            match key {
                "name" => m.name = val,
                "exec" => m.exec = val,
                "type" => m.kind = val,
                "permission" => m.permission = val,
                _ => {}
            }
        }
    }
    m
}
