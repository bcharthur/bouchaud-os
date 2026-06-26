//! Client Git minimal pour Bouchaud OS.
//!
//! Implémente les commandes de base de Git sur le RAMFS local.
//! Supporte : init, status, add, commit, log, clone (HTTP), branch, diff.

use crate::drivers::vga::{self, COLOR_GREEN, COLOR_CYAN, COLOR_RED, COLOR_YELLOW, COLOR_DEFAULT};
use crate::fs::ramfs::{self, NodeKind};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;

// ─── SHA-1 (RFC 3174) ────────────────────────────────────────────────────────

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    let bit_len = (data.len() as u64) * 8;
    let remainder = data.len() % 64;
    let pad_bytes = if remainder < 56 { 56 - remainder } else { 120 - remainder };
    let mut padded: Vec<u8> = Vec::new();
    padded.extend_from_slice(data);
    padded.push(0x80);
    for _ in 0..pad_bytes - 1 { padded.push(0); }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for block in padded.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 { w[i] = u32::from_be_bytes([block[i*4], block[i*4+1], block[i*4+2], block[i*4+3]]); }
        for i in 16..80 { w[i] = (w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16]).rotate_left(1); }
        let [mut a, mut b, mut c, mut d, mut e] = [h[0], h[1], h[2], h[3], h[4]];
        for i in 0..80 {
            let (f, k) = match i {
                0..=19  => ((b & c) | (!b & d), 0x5a827999u32),
                20..=39 => (b ^ c ^ d,           0x6ed9eba1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _       => (b ^ c ^ d,           0xca62c1d6u32),
            };
            let tmp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(w[i]);
            e = d; d = c; c = b.rotate_left(30); b = a; a = tmp;
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d); h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for i in 0..5 { out[i*4..i*4+4].copy_from_slice(&h[i].to_be_bytes()); }
    out
}

fn hex40(b: &[u8; 20]) -> String {
    let mut s = String::new();
    for byte in b {
        s.push(char::from_digit((*byte >> 4) as u32, 16).unwrap_or('0'));
        s.push(char::from_digit((*byte & 0xf) as u32, 16).unwrap_or('0'));
    }
    s
}
fn short_sha(b: &[u8; 20]) -> String { hex40(b)[..7].to_string() }

// ─── Accès RAMFS ─────────────────────────────────────────────────────────────
//
// Structure :  /.git-repos/<repo>/
//   config   → "branch=main\norigin=<url>\n"
//   HEAD     → sha40 hex du dernier commit
//   index    → "fichier1\nfichier2\n…"
//   commits  → lignes "sha40|auteur|ticks|message\n"

const GIT_ROOT: &str = ".git-repos";   // sous la racine RAMFS

fn ensure_git_root() -> Option<usize> {
    let fs = ramfs::fs();
    if let Some(idx) = fs.find_child(0, GIT_ROOT) { return Some(idx); }
    fs.mkdir_at(0, GIT_ROOT).ok()
}

fn get_or_create_repo(name: &str) -> Option<usize> {
    let root = ensure_git_root()?;
    let fs = ramfs::fs();
    if let Some(idx) = fs.find_child(root, name) { return Some(idx); }
    fs.mkdir_at(root, name).ok()
}

fn get_repo(name: &str) -> Option<usize> {
    let fs = ramfs::fs();
    let root = fs.find_child(0, GIT_ROOT)?;
    fs.find_child(root, name)
}

fn read_child(dir_idx: usize, name: &str) -> String {
    let fs = ramfs::fs();
    let fidx = match fs.find_child(dir_idx, name) { Some(i) => i, None => return String::new() };
    if fs.nodes[fidx].kind != NodeKind::File { return String::new(); }
    let mut s = String::new();
    for k in 0..fs.nodes[fidx].content_len { s.push(fs.nodes[fidx].content[k] as char); }
    s
}

fn write_child(dir_idx: usize, name: &str, content: &str) {
    let fs = ramfs::fs();
    match fs.touch_at(dir_idx, name) {
        Ok(fidx) => {
            let bytes = content.as_bytes();
            let n = bytes.len().min(ramfs::CONTENT_LEN);
            fs.nodes[fidx].content_len = n;
            for i in 0..n { fs.nodes[fidx].content[i] = bytes[i]; }
        }
        Err(_) => {}
    }
}

fn current_repo_name(cwd: usize) -> String {
    let fs = ramfs::fs();
    let name = fs.nodes[cwd].name_str();
    if name.is_empty() { "repo".into() } else { name.to_string() }
}

// ─── API publique ─────────────────────────────────────────────────────────────

pub fn cmd(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 { print_usage(); return 0; }
    match argv[1] {
        "init"         => cmd_init(cwd),
        "status"       => cmd_status(cwd),
        "add"          => cmd_add(argc, argv, cwd),
        "commit"       => cmd_commit(argc, argv, cwd),
        "log"          => cmd_log(cwd),
        "clone"        => cmd_clone(argc, argv),
        "diff"         => cmd_diff(cwd),
        "branch"       => cmd_branch(cwd),
        "help"|"--help"=> { print_usage(); 0 }
        s => {
            vga::set_color(COLOR_RED);
            println!("git: sous-commande inconnue '{}'", s);
            vga::set_color(COLOR_DEFAULT);
            1
        }
    }
}

fn print_usage() {
    vga::set_color(COLOR_CYAN);
    println!("git — controle de version");
    vga::set_color(COLOR_DEFAULT);
    println!("  git init              initialise un depot");
    println!("  git status            etat du depot");
    println!("  git add <f>           stage un fichier (. pour tout)");
    println!("  git commit -m <msg>   cree un commit");
    println!("  git log               journal des commits");
    println!("  git branch            branches");
    println!("  git diff              fichiers modifies");
    println!("  git clone <url>       clone distant (HTTP)");
}

fn cmd_init(cwd: usize) -> i32 {
    let name = current_repo_name(cwd);
    match get_or_create_repo(&name) {
        None => { println!("git init: RAMFS plein ou erreur"); return 1; }
        Some(idx) => {
            // Initialise les fichiers seulement si vides
            let existing = read_child(idx, "config");
            if !existing.is_empty() {
                vga::set_color(COLOR_YELLOW);
                println!("Depot '{}' deja initialise.", name);
                vga::set_color(COLOR_DEFAULT);
                return 0;
            }
            write_child(idx, "config", "branch=main\n");
            write_child(idx, "HEAD",   "");
            write_child(idx, "index",  "");
            write_child(idx, "commits","");
            vga::set_color(COLOR_GREEN);
            println!("Depot Git initialise : '{}'", name);
            vga::set_color(COLOR_DEFAULT);
            println!("  git add <fichier>       pour stager");
            println!("  git commit -m 'message' pour valider");
            0
        }
    }
}

fn cmd_status(cwd: usize) -> i32 {
    let name = current_repo_name(cwd);
    let repo = match get_repo(&name) {
        Some(i) => i, None => { println!("git status: pas de depot (git init)"); return 1; }
    };
    let config  = read_child(repo, "config");
    let head    = read_child(repo, "HEAD");
    let index   = read_child(repo, "index");
    let branch  = config.lines().find_map(|l| l.strip_prefix("branch=")).unwrap_or("main");

    vga::set_color(COLOR_CYAN);
    print!("Sur la branche {}", branch);
    vga::set_color(COLOR_DEFAULT);
    if head.trim().is_empty() { println!("\n\nCommit initial."); }
    else { println!("\nDernier commit : {}", head.trim()); }

    let staged: Vec<&str> = index.lines().filter(|l| !l.is_empty()).collect();
    if !staged.is_empty() {
        vga::set_color(COLOR_GREEN);
        println!("\nModifications pret pour commit :");
        for f in &staged { println!("        nouveau fichier : {}", f); }
        vga::set_color(COLOR_DEFAULT);
    }

    // Fichiers non suivis dans le répertoire courant
    let fs = ramfs::fs();
    let mut untracked = Vec::new();
    for i in 0..ramfs::MAX_NODES {
        if fs.nodes[i].used && fs.nodes[i].parent == cwd && fs.nodes[i].kind == NodeKind::File {
            let n = fs.nodes[i].name_str().to_string();
            if !staged.contains(&n.as_str()) { untracked.push(n); }
        }
    }
    if !untracked.is_empty() {
        println!("\nFichiers non suivis :");
        for f in &untracked { println!("        {}", f); }
        println!("  (git add <fichier> pour inclure)");
    }
    if staged.is_empty() && untracked.is_empty() {
        println!("\nArbre de travail propre.");
    }
    0
}

fn cmd_add(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 { println!("usage: git add <fichier | .>"); return 1; }
    let name = current_repo_name(cwd);
    let repo = match get_repo(&name) {
        Some(i) => i, None => { println!("git add: pas de depot (git init)"); return 1; }
    };
    let target = argv[2];
    let mut index = read_child(repo, "index");
    let all = target == "." || target == "-A" || target == "--all";

    if all {
        let fs = ramfs::fs();
        for i in 0..ramfs::MAX_NODES {
            if fs.nodes[i].used && fs.nodes[i].parent == cwd && fs.nodes[i].kind == NodeKind::File {
                let n = fs.nodes[i].name_str().to_string();
                if !index.lines().any(|l| l == n) {
                    index.push_str(&n); index.push('\n');
                }
            }
        }
        vga::set_color(COLOR_GREEN);
        println!("Tous les fichiers stages.");
        vga::set_color(COLOR_DEFAULT);
    } else {
        // Vérifie que le fichier existe dans cwd
        let fs = ramfs::fs();
        let found = (0..ramfs::MAX_NODES).any(|i| {
            fs.nodes[i].used && fs.nodes[i].parent == cwd && fs.nodes[i].kind == NodeKind::File
            && fs.nodes[i].name_eq(target)
        });
        if !found { println!("git add: '{}' introuvable", target); return 1; }
        if !index.lines().any(|l| l == target) {
            index.push_str(target); index.push('\n');
        }
        vga::set_color(COLOR_GREEN);
        println!("+ {}", target);
        vga::set_color(COLOR_DEFAULT);
    }
    write_child(repo, "index", &index);
    0
}

fn cmd_commit(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 4 || argv[2] != "-m" { println!("usage: git commit -m \"message\""); return 1; }
    let msg = argv[3];
    let name = current_repo_name(cwd);
    let repo = match get_repo(&name) {
        Some(i) => i, None => { println!("git commit: pas de depot"); return 1; }
    };
    let index = read_child(repo, "index");
    if index.trim().is_empty() { println!("git commit: rien a committer (git add d'abord)"); return 1; }

    let author = crate::users::session().username().to_string();
    let ts = crate::kernel::timer::ticks();
    let data = format!("{}|{}|{}|{}", author, ts, msg, index);
    let hash_bytes = sha1(data.as_bytes());
    let hash = hex40(&hash_bytes);

    write_child(repo, "HEAD", &hash);

    // Ajoute au journal (tronqué si nécessaire)
    let entry = format!("{}|{}|{}\n", hash, author, msg);
    let mut log = read_child(repo, "commits");
    if log.len() + entry.len() + 5 > ramfs::CONTENT_LEN {
        // Garde la moitié du journal
        let half = log.len() / 2;
        if let Some(nl) = log[half..].find('\n') { log = log[half + nl + 1..].to_string(); }
    }
    log.push_str(&entry);
    write_child(repo, "commits", &log);
    write_child(repo, "index", "");

    let n_files = index.lines().filter(|l| !l.is_empty()).count();
    vga::set_color(COLOR_CYAN);
    println!("[main {}] {}", short_sha(&hash_bytes), msg);
    vga::set_color(COLOR_DEFAULT);
    println!("  {} fichier(s) | auteur: {}", n_files, author);
    0
}

fn cmd_log(cwd: usize) -> i32 {
    let name = current_repo_name(cwd);
    let repo = match get_repo(&name) {
        Some(i) => i, None => { println!("git log: pas de depot"); return 1; }
    };
    let log = read_child(repo, "commits");
    if log.trim().is_empty() { println!("Aucun commit."); return 0; }
    for entry in log.lines().rev().filter(|l| !l.is_empty()) {
        let p: Vec<&str> = entry.splitn(3, '|').collect();
        if p.len() < 3 { continue; }
        vga::set_color(COLOR_YELLOW);
        println!("commit {}", p[0]);
        vga::set_color(COLOR_DEFAULT);
        println!("Auteur : {}", p[1]);
        println!("");
        println!("    {}", p[2]);
        println!("");
    }
    0
}

fn cmd_diff(cwd: usize) -> i32 {
    let name = current_repo_name(cwd);
    let repo = match get_repo(&name) {
        Some(i) => i, None => { println!("git diff: pas de depot"); return 1; }
    };
    let head = read_child(repo, "HEAD");
    if head.trim().is_empty() { println!("(aucun commit — tout est nouveau)"); return 0; }
    // Sans stockage d'objets, on liste les fichiers présents
    println!("Fichiers dans le repertoire courant :");
    let fs = ramfs::fs();
    let mut any = false;
    for i in 0..ramfs::MAX_NODES {
        if fs.nodes[i].used && fs.nodes[i].parent == cwd && fs.nodes[i].kind == NodeKind::File {
            vga::set_color(COLOR_GREEN);
            println!("  {}", fs.nodes[i].name_str());
            vga::set_color(COLOR_DEFAULT);
            any = true;
        }
    }
    if !any { println!("  (aucun fichier)"); }
    println!("(diff complet indisponible sans stockage d'objets)");
    0
}

fn cmd_branch(cwd: usize) -> i32 {
    let name = current_repo_name(cwd);
    let repo = match get_repo(&name) {
        Some(i) => i, None => { println!("git branch: pas de depot"); return 1; }
    };
    let config = read_child(repo, "config");
    let branch = config.lines().find_map(|l| l.strip_prefix("branch=")).unwrap_or("main");
    vga::set_color(COLOR_GREEN);
    println!("* {}", branch);
    vga::set_color(COLOR_DEFAULT);
    0
}

fn cmd_clone(argc: usize, argv: &[&str; 12]) -> i32 {
    if argc < 3 { println!("usage: git clone <url>"); return 1; }
    let url = argv[2];
    vga::set_color(COLOR_CYAN);
    println!("Clonage de {}...", url);
    vga::set_color(COLOR_DEFAULT);

    // Tente une connexion HTTP pour récupérer les refs
    let refs_url = if url.ends_with('/') {
        format!("{}info/refs?service=git-upload-pack", url)
    } else {
        format!("{}/info/refs?service=git-upload-pack", url)
    };
    let doc = crate::net::fetch_document(&refs_url);

    // Nom du dépôt local à partir de l'URL
    let repo_name = url.trim_end_matches('/').rsplit('/').next()
        .unwrap_or("repo").trim_end_matches(".git");
    let repo_name = if repo_name.is_empty() { "cloned" } else { repo_name };

    let repo_idx = match get_or_create_repo(repo_name) {
        Some(i) => i,
        None => { println!("git clone: RAMFS plein"); return 1; }
    };

    write_child(repo_idx, "config", &format!("branch=main\norigin={}\n", url));
    write_child(repo_idx, "HEAD",   "");
    write_child(repo_idx, "index",  "");
    write_child(repo_idx, "commits","");

    if doc.ok {
        vga::set_color(COLOR_GREEN);
        println!("Connexion reussie ({} octets)", doc.body.len());
        vga::set_color(COLOR_DEFAULT);
        println!("Depot '{}' cree (refs recuperes).", repo_name);
        println!("Note : le transfert pack-file complet est en cours de dev.");
    } else {
        vga::set_color(COLOR_YELLOW);
        println!("Connexion impossible — depot vide cree localement.");
        if !doc.banner.is_empty() { println!("  {}", doc.banner[0]); }
        vga::set_color(COLOR_DEFAULT);
        return 1;
    }
    0
}
