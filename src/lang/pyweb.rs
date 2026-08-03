//! Pont Web minimal pour les programmes Python.
//!
//! RustPython est compile sans `socket` ni `ssl` (impossibles en WASI), donc
//! un script Python ne peut pas ouvrir de connexion lui-meme. Le noyau lui
//! prete sa propre pile : un pseudo-fichier `/dev/web` ou l'on ecrit une URL
//! et d'ou l'on relit la reponse.
//!
//! C'est le strict necessaire pour verifier que les couches de l'OS
//! s'enchainent : interpreteur Python -> pont WASI -> RAMFS -> pile TCP/TLS
//! -> retour a Python -> console.

use alloc::string::String;
use alloc::vec::Vec;

use crate::fs::ramfs;

/// Pseudo-fichier de requetes Web.
pub const DEVICE: &str = "/dev/web";
/// Emplacement du navigateur de test dans le RAMFS.
pub const SCRIPT: &str = "/usr/lib/python/browser.py";

/// Inode du peripherique (0 tant qu'il n'est pas installe).
static mut DEV_WEB: usize = 0;

/// Le navigateur de test, embarque dans le binaire comme les polices.
static BROWSER_PY: &str = include_str!("../assets/python/browser.py");

/// Cree `/dev/web` et depose `browser.py` dans le RAMFS. Idempotent.
pub fn install() {
    let dev = mkdir_path("/dev");
    let lib = mkdir_path("/usr/lib/python");
    let fs = ramfs::fs();

    if dev != 0 {
        let node = match fs.find_child(dev, "web") {
            Some(existing) => existing,
            None => fs.touch_at(dev, "web").unwrap_or(0),
        };
        if node != 0 {
            // Lecture/ecriture pour tous : le navigateur tourne en session
            // utilisateur, pas en root.
            fs.nodes[node].mode = 0o666;
            unsafe { DEV_WEB = node };
        }
    }

    if lib != 0 {
        let node = match fs.find_child(lib, "browser.py") {
            Some(existing) => existing,
            None => fs.touch_at(lib, "browser.py").unwrap_or(0),
        };
        if node != 0 {
            fs.write_node_bytes(node, BROWSER_PY.as_bytes());
        }
    }
}

fn mkdir_path(path: &str) -> usize {
    let fs = ramfs::fs();
    let mut current = 0usize;
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        current = match fs.find_child(current, segment) {
            Some(existing) => existing,
            None => match fs.mkdir_at(current, segment) {
                Ok(created) => created,
                Err(_) => return 0,
            },
        };
    }
    current
}

/// Interception d'une ecriture WASI. Renvoie `true` si c'etait `/dev/web` :
/// l'appelant ne doit alors rien ecrire dans le RAMFS.
pub fn device_write(node: usize, bytes: &[u8]) -> bool {
    if node == 0 || node != unsafe { DEV_WEB } {
        return false;
    }
    let url = core::str::from_utf8(bytes).unwrap_or("").trim();
    if url.is_empty() {
        store(node, 0, "", "", b"URL vide");
        return true;
    }

    let document = crate::net::fetch_document(url);
    if !document.ok {
        let mut detail = String::new();
        for line in &document.banner {
            detail.push_str(line);
            detail.push('\n');
        }
        store(node, 0, url, "", detail.as_bytes());
        return true;
    }

    store(
        node,
        200,
        &document.final_url,
        &document.content_type,
        &document.body,
    );
    true
}

/// Depose la reponse sous une forme triviale a relire depuis Python :
/// trois lignes d'en-tete, une ligne vide, puis le corps.
fn store(node: usize, status: u16, url: &str, content_type: &str, body: &[u8]) {
    let head = alloc::format!(
        "STATUS {}\nURL {}\nTYPE {}\n\n",
        status,
        url,
        if content_type.is_empty() { "?" } else { content_type }
    );
    let mut raw: Vec<u8> = Vec::with_capacity(head.len() + body.len());
    raw.extend_from_slice(head.as_bytes());
    raw.extend_from_slice(body);
    ramfs::fs().write_node_bytes(node, &raw);
}
