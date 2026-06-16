//! Lanceur d'applications : enumere `/apps/*.bapp` et lance par nom.

use crate::app::manifest;
use crate::app::runtime::AppKind;
use crate::fs::ramfs::{self, NodeKind, MAX_NODES};
use alloc::string::String;

/// Liste les applications installees (commande `apps`).
pub fn list() {
    let fs = ramfs::fs();
    let apps = match fs.resolve("/apps", 0) {
        Some(i) => i,
        None => { crate::println!("apps: /apps introuvable"); return; }
    };
    crate::println!("Applications installees (/apps):");
    let mut found = false;
    for i in 0..MAX_NODES {
        if fs.nodes[i].used && fs.nodes[i].parent == apps && fs.nodes[i].kind == NodeKind::File {
            let name = fs.nodes[i].name_str();
            if !name.ends_with(".bapp") { continue; }
            let mut content = String::new();
            for k in 0..fs.nodes[i].content_len { content.push(fs.nodes[i].content[k] as char); }
            let m = manifest::parse(&content);
            crate::println!("  {:<16} exec={:<10} [{}]", m.name, m.exec, m.kind);
            found = true;
        }
    }
    if !found { crate::println!("  (aucune)"); }
}

/// Lance une application par son `exec` (ou nom de manifeste).
pub fn launch(name: &str) {
    let fs = ramfs::fs();
    let apps = match fs.resolve("/apps", 0) {
        Some(i) => i,
        None => { crate::println!("launch: /apps introuvable"); return; }
    };
    for i in 0..MAX_NODES {
        if fs.nodes[i].used && fs.nodes[i].parent == apps && fs.nodes[i].kind == NodeKind::File {
            let mut content = String::new();
            for k in 0..fs.nodes[i].content_len { content.push(fs.nodes[i].content[k] as char); }
            let m = manifest::parse(&content);
            if m.exec == name || m.name == name {
                let kind = AppKind::from_exec(&m.exec);
                if kind.is_gui() {
                    crate::println!("launch: '{}' est une app graphique -> lance 'desktop' puis le menu Demarrer", m.name);
                } else {
                    crate::println!("launch: type d'app non supporte en CLI: {}", m.kind);
                }
                return;
            }
        }
    }
    crate::println!("launch: application '{}' introuvable (voir 'apps')", name);
}
