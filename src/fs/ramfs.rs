//! RAMFS : systeme de fichiers en memoire a inodes fixes.
//!
//! Aucune allocation dynamique : un tableau statique de `Node` sert de table
//! d'inodes. Supporte fichiers et dossiers, permissions simples, uid/gid, et
//! une resolution de chemin de style Unix (`/`, `.`, `..`).

use crate::drivers::vga::{self, COLOR_CYAN, COLOR_DEFAULT};
use crate::users;
use alloc::string::String;

pub const MAX_NODES: usize = 96;
pub const NAME_LEN: usize = 32;
pub const CONTENT_LEN: usize = 768;

/// Droits, sur le modele Unix : lecture / ecriture / execution(-traversee).
pub const PERM_R: u16 = 4;
pub const PERM_W: u16 = 2;
pub const PERM_X: u16 = 1;

#[derive(Copy, Clone, PartialEq)]
pub enum NodeKind {
    File,
    Dir,
}

#[derive(Copy, Clone)]
pub struct Node {
    pub used: bool,
    pub kind: NodeKind,
    pub parent: usize,
    pub name: [u8; NAME_LEN],
    pub name_len: usize,
    pub content: [u8; CONTENT_LEN],
    pub content_len: usize,
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
}

impl Node {
    pub const fn empty() -> Self {
        Self {
            used: false,
            kind: NodeKind::File,
            parent: 0,
            name: [0; NAME_LEN],
            name_len: 0,
            content: [0; CONTENT_LEN],
            content_len: 0,
            mode: 0o644,
            uid: 0,
            gid: 0,
        }
    }

    pub fn name_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.name[..self.name_len]) }
    }

    pub fn name_eq(&self, name: &str) -> bool {
        if self.name_len != name.len() { return false; }
        let bytes = name.as_bytes();
        for i in 0..self.name_len {
            if self.name[i] != bytes[i] { return false; }
        }
        true
    }

    pub fn set_name(&mut self, name: &str) -> bool {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > NAME_LEN { return false; }
        for i in 0..NAME_LEN { self.name[i] = 0; }
        for i in 0..bytes.len() { self.name[i] = bytes[i]; }
        self.name_len = bytes.len();
        true
    }
}

pub struct FileSystem {
    pub nodes: [Node; MAX_NODES],
}

static mut FS: FileSystem = FileSystem { nodes: [Node::empty(); MAX_NODES] };

/// Accede au systeme de fichiers global.
pub fn fs() -> &'static mut FileSystem {
    unsafe { &mut FS }
}

impl FileSystem {
    /// Monte le RAMFS et cree l'arborescence de base.
    pub fn init(&mut self) {
        self.nodes = [Node::empty(); MAX_NODES];

        self.nodes[0].used = true;
        self.nodes[0].kind = NodeKind::Dir;
        self.nodes[0].parent = 0;
        self.nodes[0].mode = 0o755;
        self.nodes[0].uid = 0;
        self.nodes[0].gid = 0;

        let home = self.mkdir_at(0, "home").unwrap_or(0);
        let tmp = self.mkdir_at(0, "tmp").unwrap_or(0);
        let etc = self.mkdir_at(0, "etc").unwrap_or(0);
        let var = self.mkdir_at(0, "var").unwrap_or(0);
        let _log = self.mkdir_at(var, "log");
        let _ = home;

        // Catalogue d'applications natives (manifestes .bapp).
        let apps = self.mkdir_at(0, "apps").unwrap_or(0);
        if apps != 0 {
            let t = self.touch_at(apps, "terminal.bapp").unwrap_or(0);
            self.write_node(t, "name=Terminal\nexec=terminal\ntype=gui\npermission=normal");
            let f = self.touch_at(apps, "files.bapp").unwrap_or(0);
            self.write_node(f, "name=Fichiers\nexec=files\ntype=gui\npermission=normal");
            let b = self.touch_at(apps, "browser.bapp").unwrap_or(0);
            self.write_node(b, "name=Nautile\nexec=browser\ntype=gui\npermission=normal");
            let s = self.touch_at(apps, "sysinfo.bapp").unwrap_or(0);
            self.write_node(s, "name=Moniteur\nexec=monitor\ntype=gui\npermission=normal");
        }

        let readme = self.touch_at(0, "readme.txt").unwrap_or(0);
        self.write_node(readme, "Bienvenue dans Bouchaud OS. Connecte-toi (guest/guest ou root/root). Tape help, ou desktop pour le bureau graphique.");

        let passwd = self.touch_at(etc, "passwd").unwrap_or(0);
        self.write_node(passwd, "root:x:0:0:root:/:/bin/bsh\nguest:x:1000:1000:guest:/home/guest:/bin/bsh");

        // /tmp est ouvert a tous (comme sous Unix).
        if tmp != 0 {
            self.nodes[tmp].mode = 0o777;
        }
    }

    fn alloc_node(&mut self) -> Option<usize> {
        for i in 1..MAX_NODES {
            if !self.nodes[i].used {
                self.nodes[i] = Node::empty();
                self.nodes[i].used = true;
                return Some(i);
            }
        }
        None
    }

    pub fn find_child(&self, parent: usize, name: &str) -> Option<usize> {
        for i in 0..MAX_NODES {
            if self.nodes[i].used && self.nodes[i].parent == parent && self.nodes[i].name_eq(name) {
                return Some(i);
            }
        }
        None
    }

    pub fn mkdir_at(&mut self, parent: usize, name: &str) -> Result<usize, &'static str> {
        if self.nodes[parent].kind != NodeKind::Dir { return Err("parent not a directory"); }
        if self.find_child(parent, name).is_some() { return Err("already exists"); }
        let idx = self.alloc_node().ok_or("no free inode")?;
        self.nodes[idx].kind = NodeKind::Dir;
        self.nodes[idx].parent = parent;
        self.nodes[idx].mode = 0o755;
        self.nodes[idx].uid = users::session().uid();
        self.nodes[idx].gid = users::session().gid();
        if !self.nodes[idx].set_name(name) { return Err("invalid name"); }
        Ok(idx)
    }

    pub fn touch_at(&mut self, parent: usize, name: &str) -> Result<usize, &'static str> {
        if self.nodes[parent].kind != NodeKind::Dir { return Err("parent not a directory"); }
        if let Some(existing) = self.find_child(parent, name) { return Ok(existing); }
        let idx = self.alloc_node().ok_or("no free inode")?;
        self.nodes[idx].kind = NodeKind::File;
        self.nodes[idx].parent = parent;
        self.nodes[idx].mode = 0o644;
        self.nodes[idx].uid = users::session().uid();
        self.nodes[idx].gid = users::session().gid();
        if !self.nodes[idx].set_name(name) { return Err("invalid name"); }
        Ok(idx)
    }

    pub fn write_node(&mut self, idx: usize, text: &str) {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() && i < CONTENT_LEN {
            self.nodes[idx].content[i] = bytes[i];
            i += 1;
        }
        self.nodes[idx].content_len = i;
    }

    pub fn append_node(&mut self, idx: usize, text: &str) {
        let bytes = text.as_bytes();
        let mut pos = self.nodes[idx].content_len;
        if pos > 0 && pos < CONTENT_LEN {
            self.nodes[idx].content[pos] = b'\n';
            pos += 1;
        }
        let mut i = 0;
        while i < bytes.len() && pos < CONTENT_LEN {
            self.nodes[idx].content[pos] = bytes[i];
            pos += 1;
            i += 1;
        }
        self.nodes[idx].content_len = pos;
    }

    pub fn resolve(&self, path: &str, cwd: usize) -> Option<usize> {
        if path.is_empty() { return Some(cwd); }
        let mut current = if path.as_bytes()[0] == b'/' { 0 } else { cwd };
        let bytes = path.as_bytes();
        let mut i = 0usize;

        while i < bytes.len() {
            while i < bytes.len() && bytes[i] == b'/' { i += 1; }
            if i >= bytes.len() { break; }
            let start = i;
            while i < bytes.len() && bytes[i] != b'/' { i += 1; }
            let comp = &path[start..i];

            if comp == "." {
                continue;
            } else if comp == ".." {
                current = self.nodes[current].parent;
            } else {
                current = self.find_child(current, comp)?;
            }
        }
        Some(current)
    }

    pub fn resolve_parent_name<'a>(&self, path: &'a str, cwd: usize) -> Option<(usize, &'a str)> {
        let mut end = path.len();
        let bytes = path.as_bytes();
        while end > 1 && bytes[end - 1] == b'/' { end -= 1; }
        let path = &path[..end];
        if path.is_empty() || path == "/" { return None; }

        let bytes = path.as_bytes();
        let mut last_slash: Option<usize> = None;
        for i in 0..bytes.len() {
            if bytes[i] == b'/' { last_slash = Some(i); }
        }

        match last_slash {
            None => Some((cwd, path)),
            Some(0) => Some((0, &path[1..])),
            Some(pos) => {
                let parent_path = &path[..pos];
                let name = &path[pos + 1..];
                let parent = self.resolve(parent_path, cwd)?;
                Some((parent, name))
            }
        }
    }

    /// Verifie si l'utilisateur courant possede les droits `want` (PERM_R/W/X)
    /// sur l'inode `idx`. root contourne toutes les verifications.
    pub fn can(&self, idx: usize, want: u16) -> bool {
        let s = users::session();
        if s.is_root() { return true; }
        let n = &self.nodes[idx];
        let bits = if s.uid() == n.uid {
            (n.mode >> 6) & 0o7
        } else if s.gid() == n.gid {
            (n.mode >> 3) & 0o7
        } else {
            n.mode & 0o7
        };
        (bits & want) == want
    }

    /// Resout un chemin en verifiant le droit d'execution (traversee) sur chaque
    /// repertoire parcouru, comme sous Unix. C'est ce controle qui empeche
    /// `guest` d'atteindre le contenu de `/home/arthur` (mode 700).
    pub fn resolve_checked(&self, path: &str, cwd: usize) -> Result<usize, &'static str> {
        if path.is_empty() { return Ok(cwd); }
        let mut current = if path.as_bytes()[0] == b'/' { 0 } else { cwd };
        let bytes = path.as_bytes();
        let mut i = 0usize;

        while i < bytes.len() {
            while i < bytes.len() && bytes[i] == b'/' { i += 1; }
            if i >= bytes.len() { break; }
            let start = i;
            while i < bytes.len() && bytes[i] != b'/' { i += 1; }
            let comp = &path[start..i];

            if comp == "." {
                continue;
            }
            // Pour franchir le repertoire courant il faut le droit d'execution.
            if !self.can(current, PERM_X) {
                return Err("permission denied");
            }
            if comp == ".." {
                current = self.nodes[current].parent;
            } else {
                current = self.find_child(current, comp).ok_or("introuvable")?;
            }
        }
        Ok(current)
    }

    /// Variante verifiee de `resolve_parent_name` : controle la traversee.
    pub fn resolve_parent_name_checked<'a>(&self, path: &'a str, cwd: usize) -> Result<(usize, &'a str), &'static str> {
        let mut end = path.len();
        let bytes = path.as_bytes();
        while end > 1 && bytes[end - 1] == b'/' { end -= 1; }
        let path = &path[..end];
        if path.is_empty() || path == "/" { return Err("chemin invalide"); }

        let bytes = path.as_bytes();
        let mut last_slash: Option<usize> = None;
        for i in 0..bytes.len() {
            if bytes[i] == b'/' { last_slash = Some(i); }
        }

        match last_slash {
            None => Ok((cwd, path)),
            Some(0) => Ok((0, &path[1..])),
            Some(pos) => {
                let parent_path = &path[..pos];
                let name = &path[pos + 1..];
                let parent = self.resolve_checked(parent_path, cwd)?;
                Ok((parent, name))
            }
        }
    }

    pub fn is_empty_dir(&self, idx: usize) -> bool {
        for i in 0..MAX_NODES {
            if self.nodes[i].used && i != idx && self.nodes[i].parent == idx {
                return false;
            }
        }
        true
    }

    pub fn used_nodes(&self) -> usize {
        let mut n = 0;
        for i in 0..MAX_NODES {
            if self.nodes[i].used { n += 1; }
        }
        n
    }

    pub fn free_nodes(&self) -> usize {
        MAX_NODES - self.used_nodes()
    }
}

/// Affiche le chemin absolu d'un inode.
pub fn print_path(fs: &FileSystem, idx: usize) {
    if idx == 0 {
        print!("/");
        return;
    }
    print_path_rec(fs, idx);
}

fn print_path_rec(fs: &FileSystem, idx: usize) {
    if idx == 0 { return; }
    let parent = fs.nodes[idx].parent;
    print_path_rec(fs, parent);
    print!("/{}", fs.nodes[idx].name_str());
}

/// Construit le chemin absolu d'un inode sous forme de chaine.
pub fn path_string(fs: &FileSystem, idx: usize) -> String {
    let mut s = String::new();
    build_path(fs, idx, &mut s);
    if s.is_empty() { s.push('/'); }
    s
}

fn build_path(fs: &FileSystem, idx: usize, s: &mut String) {
    if idx == 0 { return; }
    build_path(fs, fs.nodes[idx].parent, s);
    s.push('/');
    s.push_str(fs.nodes[idx].name_str());
}

/// Affiche les droits de style `ls -l` (ex. `drwxr-xr-x`).
pub fn print_mode(kind: NodeKind, mode: u16) {
    print!("{}", if kind == NodeKind::Dir { 'd' } else { '-' });
    let bits = [0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001];
    let chars = ['r', 'w', 'x', 'r', 'w', 'x', 'r', 'w', 'x'];
    for i in 0..9 {
        print!("{}", if mode & bits[i] != 0 { chars[i] } else { '-' });
    }
}

/// Affiche une entree de repertoire (utilise par `ls`).
pub fn print_node_line(fs: &FileSystem, idx: usize, long: bool) {
    let node = &fs.nodes[idx];
    if long {
        print_mode(node.kind, node.mode);
        print!(" {}:{} {:>4} ", node.uid, node.gid, node.content_len);
    }
    if node.kind == NodeKind::Dir {
        vga::set_color(COLOR_CYAN);
        crate::println!("{}/", node.name_str());
        vga::set_color(COLOR_DEFAULT);
    } else {
        crate::println!("{}", node.name_str());
    }
}
