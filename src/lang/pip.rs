//! `pip` de Bouchaud OS : installeur de paquets Python purs depuis PyPI.
//!
//! Le vrai programme `pip` est un client reseau Python : il ne peut pas
//! tourner dans le bac a sable WASM (pas de sockets WASI). Cette commande
//! noyau fait le meme travail pour les paquets **purs Python** :
//!
//!   1. interroge l'index simple de PyPI (`https://pypi.org/simple/<paquet>/`)
//!      via la pile TLS 1.3 maison (la meme que `https`/Nautile) ;
//!   2. choisit la derniere wheel universelle (`*-py3-none-any.whl` ou
//!      `*-py2.py3-none-any.whl`) -- les paquets a extensions C compilees
//!      (numpy...) sont hors de portee, comme sur toute plateforme sans cc ;
//!   3. telecharge la wheel (une archive ZIP), la decompresse avec l'inflate
//!      maison (`net::encoding::inflate`) ;
//!   4. extrait les fichiers dans `/usr/lib/python/site-packages` du RAMFS ;
//!   5. installe recursivement les dependances non conditionnelles
//!      (`Requires-Dist` du METADATA), purs Python egalement.
//!
//! Les paquets deviennent immediatement importables : l'interpreteur recoit
//! `PYTHONPATH=/usr/lib/python/site-packages` (voir `python.rs`) et lit les
//! fichiers via le pont WASI -> RAMFS.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::fs::ramfs::{self, NodeKind};
use crate::lang::python::SITE_PACKAGES;

/// Taille maximale d'une wheel acceptee (garde-fou memoire).
const MAX_WHEEL_SIZE: usize = 8 * 1024 * 1024;
/// Profondeur maximale de resolution des dependances.
const MAX_DEPTH: usize = 4;

pub fn cmd(argc: usize, argv: &[&str; 12]) -> i32 {
    if argc < 2 {
        usage();
        return 1;
    }
    match argv[1] {
        "install" => {
            if argc < 3 {
                println!("usage: pip install <paquet> [paquet...]");
                return 1;
            }
            let mut installed: Vec<String> = Vec::new();
            let mut code = 0;
            for i in 2..argc {
                if install(argv[i], 0, &mut installed).is_err() {
                    code = 1;
                }
            }
            if !installed.is_empty() {
                crate::drivers::vga::set_color(crate::drivers::vga::COLOR_GREEN);
                println!("Successfully installed {}", installed.join(" "));
                crate::drivers::vga::set_color(crate::drivers::vga::COLOR_DEFAULT);
            }
            code
        }
        "list" => list(),
        _ => {
            usage();
            1
        }
    }
}

fn usage() {
    println!("pip de Bouchaud OS - paquets Python purs depuis PyPI");
    println!("usage: pip install <paquet> [paquet...]   (wheels py3-none-any uniquement)");
    println!("       pip list");
    println!("note: reseau requis (ifup + dhcp) ; paquets a extensions C non supportes");
}

/// Normalisation PEP 503 : minuscules, sequences de `-_.` -> `-`.
fn normalize(name: &str) -> String {
    let mut out = String::new();
    let mut prev_sep = false;
    for c in name.chars() {
        if c == '-' || c == '_' || c == '.' {
            if !prev_sep {
                out.push('-');
            }
            prev_sep = true;
        } else {
            out.push(c.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

fn install(pkg: &str, depth: usize, installed: &mut Vec<String>) -> Result<(), ()> {
    let norm = normalize(pkg);
    if installed.iter().any(|p| normalize(p.split(' ').next().unwrap_or(p)) == norm) {
        return Ok(());
    }
    if already_installed(&norm) {
        println!("Requirement already satisfied: {}", norm);
        return Ok(());
    }

    println!("Collecting {}", norm);
    let index_url = format!("https://pypi.org/simple/{}/", norm);
    let doc = crate::net::fetch_document(&index_url);
    if !doc.ok {
        for l in &doc.banner {
            println!("  {}", l);
        }
        println!("pip: echec de connexion a PyPI (reseau initialise ? 'ifup' puis 'dhcp')");
        return Err(());
    }
    let html = String::from_utf8_lossy(&doc.body).to_string();
    let wheel_url = match find_universal_wheel(&html) {
        Some(u) => u,
        None => {
            println!("pip: aucune wheel pure-Python (py3-none-any) pour {} :", norm);
            println!("  paquet a extensions natives (numpy, pandas...) -> non supporte ici");
            return Err(());
        }
    };
    let filename = wheel_url.rsplit('/').next().unwrap_or("").to_string();
    println!("  Downloading {}", filename);

    let wheel = crate::net::fetch_document(&wheel_url);
    if !wheel.ok || wheel.body.is_empty() {
        println!("pip: telechargement echoue pour {}", filename);
        return Err(());
    }
    if wheel.body.len() > MAX_WHEEL_SIZE {
        println!("pip: wheel trop volumineuse ({} o > {} o)", wheel.body.len(), MAX_WHEEL_SIZE);
        return Err(());
    }

    let site = ensure_site_packages().ok_or(())?;
    let mut metadata = String::new();
    let n = unzip_into(&wheel.body, site, &mut metadata)?;
    println!("  {} fichier(s) installes dans {}", n, SITE_PACKAGES);

    // Version depuis le nom de wheel : <dist>-<version>-...whl
    let version = filename.split('-').nth(1).unwrap_or("?").to_string();
    installed.push(format!("{}-{}", norm, version));

    // Dependances non conditionnelles (sans marqueur `;`).
    if depth < MAX_DEPTH {
        for dep in parse_requires(&metadata) {
            if install(&dep, depth + 1, installed).is_err() {
                println!("pip: dependance {} non installee (voir ci-dessus)", dep);
            }
        }
    }
    Ok(())
}

fn already_installed(norm: &str) -> bool {
    let fs = ramfs::fs();
    let site = match fs.resolve(SITE_PACKAGES, 0) {
        Some(i) => i,
        None => return false,
    };
    let prefix = {
        // les dist-info utilisent `_` a la place de `-`
        let mut p = norm.replace('-', "_");
        p.push('-');
        p
    };
    for i in 0..ramfs::MAX_NODES {
        if fs.nodes[i].used && fs.nodes[i].parent == site && fs.nodes[i].kind == NodeKind::Dir {
            let name = fs.nodes[i].name_str();
            if name.ends_with(".dist-info") && name.to_ascii_lowercase().starts_with(&prefix) {
                return true;
            }
        }
    }
    false
}

/// Cherche la derniere wheel universelle dans la page simple de PyPI.
/// Les liens y sont tries par version croissante : on garde le dernier.
fn find_universal_wheel(html: &str) -> Option<String> {
    let mut best: Option<String> = None;
    let mut rest = html;
    while let Some(start) = rest.find("href=\"") {
        rest = &rest[start + 6..];
        let end = rest.find('"')?;
        let href = &rest[..end];
        let clean = href.split('#').next().unwrap_or(href);
        if clean.ends_with("-py3-none-any.whl") || clean.ends_with("-py2.py3-none-any.whl") {
            best = Some(clean.to_string());
        }
        rest = &rest[end..];
    }
    best
}

/// `Requires-Dist: nom (>=1.0)` -> `nom` ; ignore les deps conditionnelles
/// (`; extra == ...`, `; python_version < ...`).
fn parse_requires(metadata: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in metadata.lines() {
        if let Some(v) = line.strip_prefix("Requires-Dist:") {
            let v = v.trim();
            if v.contains(';') {
                continue;
            }
            let name: String = v
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
                .collect();
            if !name.is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

fn list() -> i32 {
    let fs = ramfs::fs();
    let site = match fs.resolve(SITE_PACKAGES, 0) {
        Some(i) => i,
        None => {
            println!("(aucun paquet installe)");
            return 0;
        }
    };
    println!("Package            Version");
    println!("------------------ -------");
    let mut none = true;
    for i in 0..ramfs::MAX_NODES {
        if fs.nodes[i].used && fs.nodes[i].parent == site && fs.nodes[i].kind == NodeKind::Dir {
            let name = fs.nodes[i].name_str();
            if let Some(base) = name.strip_suffix(".dist-info") {
                let mut parts = base.rsplitn(2, '-');
                let ver = parts.next().unwrap_or("?");
                let dist = parts.next().unwrap_or(base);
                println!("{:<18} {}", dist, ver);
                none = false;
            }
        }
    }
    if none {
        println!("(aucun paquet installe)");
    }
    0
}

// ---------------------------------------------------------------------------
// Extraction de wheel (archive ZIP)
// ---------------------------------------------------------------------------

fn u16le(b: &[u8], o: usize) -> usize {
    u16::from_le_bytes([b[o], b[o + 1]]) as usize
}
fn u32le(b: &[u8], o: usize) -> usize {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize
}

/// Decompresse chaque entree du ZIP dans `site` (chemins relatifs surs
/// uniquement). Remplit `metadata` avec le contenu du fichier
/// `*.dist-info/METADATA`. Renvoie le nombre de fichiers ecrits.
fn unzip_into(zip: &[u8], site: usize, metadata: &mut String) -> Result<usize, ()> {
    // End Of Central Directory : signature PK\x05\x06, scannee depuis la fin.
    let mut eocd = None;
    let lo = zip.len().saturating_sub(66_000);
    let mut i = zip.len().saturating_sub(22);
    while i >= lo {
        if zip[i..].starts_with(&[0x50, 0x4B, 0x05, 0x06]) {
            eocd = Some(i);
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    let eocd = match eocd {
        Some(e) => e,
        None => {
            println!("pip: archive wheel invalide (EOCD introuvable)");
            return Err(());
        }
    };
    let count = u16le(zip, eocd + 10);
    let mut off = u32le(zip, eocd + 16);

    let mut written = 0usize;
    for _ in 0..count {
        if off + 46 > zip.len() || !zip[off..].starts_with(&[0x50, 0x4B, 0x01, 0x02]) {
            println!("pip: entree ZIP corrompue");
            return Err(());
        }
        let method = u16le(zip, off + 10);
        let comp_size = u32le(zip, off + 20);
        let name_len = u16le(zip, off + 28);
        let extra_len = u16le(zip, off + 30);
        let comment_len = u16le(zip, off + 32);
        let local_off = u32le(zip, off + 42);
        let name = String::from_utf8_lossy(&zip[off + 46..off + 46 + name_len]).to_string();
        off += 46 + name_len + extra_len + comment_len;

        if name.ends_with('/') {
            continue; // repertoire pur : cree a la demande par les fichiers
        }
        // Zip-slip : refuse les chemins absolus ou remontants.
        if name.starts_with('/') || name.split('/').any(|c| c == "..") {
            println!("pip: chemin suspect ignore : {}", name);
            continue;
        }

        // En-tete local -> position des donnees.
        if local_off + 30 > zip.len() || !zip[local_off..].starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
            println!("pip: en-tete local corrompu pour {}", name);
            return Err(());
        }
        let lnl = u16le(zip, local_off + 26);
        let lel = u16le(zip, local_off + 28);
        let data_start = local_off + 30 + lnl + lel;
        if data_start + comp_size > zip.len() {
            println!("pip: donnees tronquees pour {}", name);
            return Err(());
        }
        let raw = &zip[data_start..data_start + comp_size];
        let data = match method {
            0 => raw.to_vec(),
            8 => match crate::net::encoding::inflate::inflate(raw) {
                Ok(d) => d,
                Err(_) => {
                    println!("pip: decompression echouee pour {}", name);
                    return Err(());
                }
            },
            m => {
                println!("pip: methode de compression {} non geree ({})", m, name);
                return Err(());
            }
        };

        if name.ends_with(".dist-info/METADATA") {
            *metadata = String::from_utf8_lossy(&data).to_string();
        }

        if write_path(site, &name, &data) {
            written += 1;
        } else {
            println!("pip: ecriture echouee pour {} ({} o)", name, data.len());
        }
    }
    Ok(written)
}

/// Cree les repertoires intermediaires et ecrit le fichier `rel` sous `base`.
fn write_path(base: usize, rel: &str, data: &[u8]) -> bool {
    let fs = ramfs::fs();
    let mut dir = base;
    let mut parts = rel.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            // Feuille : le fichier lui-meme.
            let idx = match fs.touch_at(dir, part) {
                Ok(i) => i,
                Err(_) => return false,
            };
            return fs.write_node_bytes(idx, data);
        }
        dir = match fs.find_child(dir, part) {
            Some(d) if fs.nodes[d].kind == NodeKind::Dir => d,
            Some(_) => return false,
            None => match fs.mkdir_at(dir, part) {
                Ok(d) => d,
                Err(_) => return false,
            },
        };
    }
    false
}

/// Cree `/usr/lib/python/site-packages` si besoin et renvoie son inode.
pub fn ensure_site_packages() -> Option<usize> {
    let fs = ramfs::fs();
    let mut dir = 0usize;
    for part in ["usr", "lib", "python", "site-packages"] {
        dir = match fs.find_child(dir, part) {
            Some(d) => d,
            None => fs.mkdir_at(dir, part).ok()?,
        };
    }
    Some(dir)
}
