//! RAMFS : systeme de fichiers en memoire a inodes fixes.
//!
//! La table d'inodes reste un tableau statique de `Node`, mais le contenu des
//! fichiers est desormais dynamique (`Vec<u8>` sur le tas) : necessaire pour
//! les scripts Python, les paquets installes par `pip` et tout fichier
//! depassant les ~768 octets de l'ancien tampon fixe. Supporte fichiers et
//! dossiers, permissions simples, uid/gid, et une resolution de chemin de
//! style Unix (`/`, `.`, `..`).

use crate::drivers::vga::{self, COLOR_CYAN, COLOR_DEFAULT};
use crate::users;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Nombre d'inodes. Une distribution minimale (libc, `ld.so`, quelques
/// bibliotheques, des polices) depasse largement le millier de fichiers.
pub const MAX_NODES: usize = 4096;
/// Longueur maximale d'un composant de chemin, en octets.
///
/// C'est `NAME_MAX`, et sa valeur n'est pas libre : Linux la fixe a 255 sur
/// tous ses systemes de fichiers, et le code applicatif compte dessus sans
/// jamais l'interroger. Ladybird, par exemple, ecrit un telechargement dans un
/// fichier temporaire nomme `<fichier>.<numero>.<uuid>.download` -- l'UUID a
/// lui seul fait 36 octets, le suffixe complet 48. Avec l'ancienne limite de
/// 64, `preuve-bouchaud.bin` donnait 67 octets et la creation echouait, donc
/// AUCUN telechargement n'aboutissait (run 32427953935).
///
/// Le cout est un tableau de 255 octets par inode au lieu de 64, soit environ
/// 1 Mio pour les 4096 inodes : sans commune mesure avec les centaines de Mio
/// du tas noyau.
pub const NAME_LEN: usize = 255;
/// Taille maximale d'un fichier (garde-fou du tas noyau).
///
/// L'ancienne limite de 4 Mio suffisait aux scripts Python mais rendait
/// impossible le depot d'un vrai binaire : une pile graphique liee
/// statiquement pese couramment plusieurs dizaines de mega-octets. Le contenu
/// vit sur le tas noyau, qui fait plusieurs centaines de Mio : 64 Mio par
/// fichier reste un garde-fou, pas une contrainte de conception.
pub const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;

/// Droits, sur le modele Unix : lecture / ecriture / execution(-traversee).
pub const PERM_R: u16 = 4;
pub const PERM_W: u16 = 2;
pub const PERM_X: u16 = 1;

#[derive(Copy, Clone, PartialEq)]
pub enum NodeKind {
    File,
    Dir,
}

#[derive(Clone)]
pub struct Node {
    pub used: bool,
    pub kind: NodeKind,
    pub parent: usize,
    pub name: [u8; NAME_LEN],
    pub name_len: usize,
    pub content: Vec<u8>,
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
            content: Vec::new(),
            mode: 0o644,
            uid: 0,
            gid: 0,
        }
    }

    /// Taille du contenu en octets (remplace l'ancien champ `content_len`).
    pub fn content_len(&self) -> usize {
        self.content.len()
    }

    /// Contenu interprete octet par octet (latin-1), comme les anciens
    /// lecteurs qui faisaient `content[i] as char`.
    pub fn content_str(&self) -> String {
        let mut s = String::with_capacity(self.content.len());
        for &b in &self.content {
            s.push(b as char);
        }
        s
    }

    pub fn name_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.name[..self.name_len]) }
    }

    pub fn name_eq(&self, name: &str) -> bool {
        if self.name_len != name.len() {
            return false;
        }
        let bytes = name.as_bytes();
        for i in 0..self.name_len {
            if self.name[i] != bytes[i] {
                return false;
            }
        }
        true
    }

    /// Le nom commence-t-il par ce prefixe ? Sert a reconnaitre l'etiquette
    /// `memfd:` sans allouer.
    pub fn name_starts_with(&self, prefixe: &[u8]) -> bool {
        if self.name_len < prefixe.len() {
            return false;
        }
        for i in 0..prefixe.len() {
            if self.name[i] != prefixe[i] {
                return false;
            }
        }
        true
    }

    /// Un nom que `set_name` acceptera. Permet de le verifier sans avoir
    /// deja un inode sous la main.
    pub fn nom_acceptable(name: &str) -> bool {
        let bytes = name.as_bytes();
        !bytes.is_empty() && bytes.len() <= NAME_LEN
    }

    pub fn set_name(&mut self, name: &str) -> bool {
        let bytes = name.as_bytes();
        if !Self::nom_acceptable(name) {
            return false;
        }
        for i in 0..NAME_LEN {
            self.name[i] = 0;
        }
        for i in 0..bytes.len() {
            self.name[i] = bytes[i];
        }
        self.name_len = bytes.len();
        true
    }
}

pub struct FileSystem {
    pub nodes: [Node; MAX_NODES],
}

// BOUCHAUD_C1_RAMFS_VERROU_PROPRE_V1
//
// Le systeme de fichiers etait un `static mut`. Rien ne le protegeait : c'est
// le gros verrou, pris par ses appelants, qui le rendait sur -- et c'est de lui
// qu'on sort. Il a maintenant le sien.
//
// `SpinLock` refuse la reprise par le meme coeur, et le dit : en construction
// de debogage -- celle de l'integration -- une reacquisition recursive PANIQUE
// en nommant le CPU, au lieu de boucler en silence. C'est ce qui rend cette
// migration conduisible : une chaine d'appels qui rentre deux fois se signale
// tout de suite, a l'endroit exact, plutot que de figer la machine.
static FS: crate::kernel::sync::SpinLock<FileSystem> =
    crate::kernel::sync::SpinLock::new(FileSystem {
        nodes: [const { Node::empty() }; MAX_NODES],
    });
/// Lock-free mirror used by interrupt/panic-safe journal prefixes.
static USED_NODES_RELAXED: AtomicUsize = AtomicUsize::new(0);

pub fn used_nodes_relaxed() -> usize {
    USED_NODES_RELAXED.load(Ordering::Relaxed)
}

/// Accede au systeme de fichiers global.
/// Le systeme de fichiers, sous son propre verrou.
///
/// Rend un GARDE, et non plus `&'static mut FileSystem`. La difference n'est
/// pas cosmetique : la reference precedente promettait une exclusivite que
/// seul le gros verrou faisait respecter, et elle la promettait a tout le
/// monde en meme temps.
///
/// La duree de vie du garde est celle de l'expression qui l'utilise. Un
/// `let fs = fs();` le retient donc pour tout le bloc -- ce qui est correct
/// tant que ce bloc ne rappelle pas `fs()`.
pub fn fs() -> crate::kernel::sync::SpinLockGuard<'static, FileSystem> {
    FS.lock()
}

impl FileSystem {
    /// Monte le RAMFS et cree l'arborescence de base.
    pub fn init(&mut self) {
        for n in self.nodes.iter_mut() {
            *n = Node::empty();
        }

        self.nodes[0].used = true;
        self.nodes[0].kind = NodeKind::Dir;
        self.nodes[0].parent = 0;
        self.nodes[0].mode = 0o755;
        self.nodes[0].uid = 0;
        self.nodes[0].gid = 0;
        USED_NODES_RELAXED.store(1, Ordering::Relaxed);

        let home = self.mkdir_at(0, "home").unwrap_or(0);
        let tmp = self.mkdir_at(0, "tmp").unwrap_or(0);
        let etc = self.mkdir_at(0, "etc").unwrap_or(0);
        let var = self.mkdir_at(0, "var").unwrap_or(0);
        let _log = self.mkdir_at(var, "log");
        let _ = home;

        // `/dev/shm` : c'est la que `shm_open` cree ses segments. Sans ce
        // repertoire, toute la memoire partagee POSIX echoue a l'ouverture —
        // et c'est sur elle que reposent les moteurs web multi-processus, qui
        // s'y passent leurs tampons d'image sans les copier.
        let dev = self.mkdir_at(0, "dev").unwrap_or(0);
        if dev != 0 {
            let _ = self.mkdir_at(dev, "shm");
        }

        // Catalogue d'applications natives (manifestes .bapp).
        let apps = self.mkdir_at(0, "apps").unwrap_or(0);
        if apps != 0 {
            let t = self.touch_at(apps, "terminal.bapp").unwrap_or(0);
            self.write_node(
                t,
                "name=Terminal\nexec=terminal\ntype=gui\npermission=normal",
            );
            let f = self.touch_at(apps, "files.bapp").unwrap_or(0);
            self.write_node(f, "name=Fichiers\nexec=files\ntype=gui\npermission=normal");
            let b = self.touch_at(apps, "browser.bapp").unwrap_or(0);
            self.write_node(b, "name=Ladybird\nexec=browser\ntype=gui\npermission=normal");
            let s = self.touch_at(apps, "sysinfo.bapp").unwrap_or(0);
            self.write_node(
                s,
                "name=Moniteur\nexec=monitor\ntype=gui\npermission=normal",
            );
        }

        let readme = self.touch_at(0, "readme.txt").unwrap_or(0);
        self.write_node(readme, "Bienvenue dans Bouchaud OS. Connecte-toi (guest/guest ou root/root). Tape help, ou desktop pour le bureau graphique.");

        let passwd = self.touch_at(etc, "passwd").unwrap_or(0);
        self.write_node(
            passwd,
            "root:x:0:0:root:/:/bin/bsh\nguest:x:1000:1000:guest:/home/guest:/bin/bsh",
        );

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
                let old = USED_NODES_RELAXED.fetch_add(1, Ordering::Relaxed);
                assert!(old < MAX_NODES, "ramfs: used-node accounting overflow");
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
        if self.nodes[parent].kind != NodeKind::Dir {
            return Err("parent not a directory");
        }
        if self.find_child(parent, name).is_some() {
            return Err("already exists");
        }
        // Le nom est valide AVANT de prendre un inode : le refuser apres
        // l'allocation laissait un inode marque occupe que plus aucun
        // repertoire ne nommait, donc definitivement perdu. Un programme qui
        // reessaie -- ce que fait un navigateur apres un telechargement
        // refuse -- epuisait ainsi la table.
        if !Node::nom_acceptable(name) {
            return Err("invalid name");
        }
        let idx = self.alloc_node().ok_or("no free inode")?;
        self.nodes[idx].kind = NodeKind::Dir;
        self.nodes[idx].parent = parent;
        self.nodes[idx].mode = 0o755;
        self.nodes[idx].uid = users::session().uid();
        self.nodes[idx].gid = users::session().gid();
        if !self.nodes[idx].set_name(name) {
            return Err("invalid name");
        }
        Ok(idx)
    }

    /// Cree un fichier sans parent ni nom visible : un `memfd`.
    ///
    /// Il n'apparait dans aucun repertoire — c'est ce qui le distingue d'un
    /// fichier ordinaire — mais il occupe un inode et se comporte comme tel
    /// pour la lecture, l'ecriture et surtout `mmap`. Le partage de memoire
    /// entre processus repose entierement la-dessus : le cache de pages est
    /// indexe par nœud, deux mappages `MAP_SHARED` du meme nœud voient donc
    /// les memes frames physiques.
    pub fn cree_anonyme(&mut self, nom: &str) -> Result<usize, &'static str> {
        let idx = self.alloc_node().ok_or("no free inode")?;
        self.nodes[idx].kind = NodeKind::File;
        // Son propre parent : la remontee d'arborescence s'arrete sur lui, et
        // il ne peut donc pas etre pris pour un descendant de la racine.
        self.nodes[idx].parent = idx;
        self.nodes[idx].mode = 0o600;
        self.nodes[idx].uid = users::session().uid();
        self.nodes[idx].gid = users::session().gid();
        let mut etiquette = String::from("memfd:");
        etiquette.push_str(nom);
        self.nodes[idx].set_name(&etiquette);
        Ok(idx)
    }

    /// Ce nœud est-il un `memfd` — donc destructible des que plus rien ne le
    /// designe ?
    ///
    /// Deux marques, pas une : etre son propre parent, et porter l'etiquette
    /// posee par `cree_anonyme`. La seconde est redondante aujourd'hui ; elle
    /// evite qu'un futur nœud auto-parent soit detruit par surprise.
    pub fn est_anonyme(&self, idx: usize) -> bool {
        idx != 0
            && idx < MAX_NODES
            && self.nodes[idx].used
            && self.nodes[idx].parent == idx
            && self.nodes[idx].name_starts_with(b"memfd:")
    }

    /// Rend l'inode d'un `memfd` dont plus aucun descripteur ni mappage ne
    /// depend.
    ///
    /// N'agit que sur un nœud anonyme : appele par erreur sur un fichier nomme,
    /// il ne ferait rien plutot que d'effacer quelque chose qui a un chemin.
    pub fn libere_anonyme(&mut self, idx: usize) -> bool {
        if !self.est_anonyme(idx) {
            return false;
        }
        self.nodes[idx] = Node::empty();
        let old = USED_NODES_RELAXED.fetch_sub(1, Ordering::Relaxed);
        assert!(old != 0, "ramfs: used-node accounting underflow");
        true
    }

    pub fn touch_at(&mut self, parent: usize, name: &str) -> Result<usize, &'static str> {
        if self.nodes[parent].kind != NodeKind::Dir {
            return Err("parent not a directory");
        }
        if let Some(existing) = self.find_child(parent, name) {
            return Ok(existing);
        }
        // Meme raison qu'au-dessus : valider avant d'allouer.
        if !Node::nom_acceptable(name) {
            return Err("invalid name");
        }
        let idx = self.alloc_node().ok_or("no free inode")?;
        self.nodes[idx].kind = NodeKind::File;
        self.nodes[idx].parent = parent;
        self.nodes[idx].mode = 0o644;
        self.nodes[idx].uid = users::session().uid();
        self.nodes[idx].gid = users::session().gid();
        if !self.nodes[idx].set_name(name) {
            return Err("invalid name");
        }
        Ok(idx)
    }

    pub fn write_node(&mut self, idx: usize, text: &str) {
        self.write_node_bytes(idx, text.as_bytes());
    }

    /// Remplace le contenu par des octets bruts. Renvoie `false` (sans rien
    /// ecrire) si la taille depasse `MAX_FILE_SIZE`.
    pub fn write_node_bytes(&mut self, idx: usize, data: &[u8]) -> bool {
        if data.len() > MAX_FILE_SIZE {
            return false;
        }
        // Une ecriture explicite remplace le backing immutable eventuel.
        crate::fs::backing::unregister(idx);
        self.nodes[idx].content = data.to_vec();
        true
    }

    pub fn append_node(&mut self, idx: usize, text: &str) {
        // Les gros fichiers de l'archive sont immuables pendant cette etape.
        if crate::fs::backing::is_disk_backed(idx) {
            return;
        }
        let node = &mut self.nodes[idx];
        let extra = text.len() + 1;
        if node.content.len() + extra > MAX_FILE_SIZE {
            return;
        }
        if !node.content.is_empty() {
            node.content.push(b'\n');
        }
        node.content.extend_from_slice(text.as_bytes());
    }

    pub fn resolve(&self, path: &str, cwd: usize) -> Option<usize> {
        if path.is_empty() {
            return Some(cwd);
        }
        let mut current = if path.as_bytes()[0] == b'/' { 0 } else { cwd };
        let bytes = path.as_bytes();
        let mut i = 0usize;

        while i < bytes.len() {
            while i < bytes.len() && bytes[i] == b'/' {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            let start = i;
            while i < bytes.len() && bytes[i] != b'/' {
                i += 1;
            }
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
        while end > 1 && bytes[end - 1] == b'/' {
            end -= 1;
        }
        let path = &path[..end];
        if path.is_empty() || path == "/" {
            return None;
        }

        let bytes = path.as_bytes();
        let mut last_slash: Option<usize> = None;
        for i in 0..bytes.len() {
            if bytes[i] == b'/' {
                last_slash = Some(i);
            }
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
        if s.is_root() {
            return true;
        }
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
        if path.is_empty() {
            return Ok(cwd);
        }
        let mut current = if path.as_bytes()[0] == b'/' { 0 } else { cwd };
        let bytes = path.as_bytes();
        let mut i = 0usize;

        while i < bytes.len() {
            while i < bytes.len() && bytes[i] == b'/' {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            let start = i;
            while i < bytes.len() && bytes[i] != b'/' {
                i += 1;
            }
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
    pub fn resolve_parent_name_checked<'a>(
        &self,
        path: &'a str,
        cwd: usize,
    ) -> Result<(usize, &'a str), &'static str> {
        let mut end = path.len();
        let bytes = path.as_bytes();
        while end > 1 && bytes[end - 1] == b'/' {
            end -= 1;
        }
        let path = &path[..end];
        if path.is_empty() || path == "/" {
            return Err("chemin invalide");
        }

        let bytes = path.as_bytes();
        let mut last_slash: Option<usize> = None;
        for i in 0..bytes.len() {
            if bytes[i] == b'/' {
                last_slash = Some(i);
            }
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
            if self.nodes[i].used {
                n += 1;
            }
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
    if idx == 0 {
        return;
    }
    let parent = fs.nodes[idx].parent;
    print_path_rec(fs, parent);
    print!("/{}", fs.nodes[idx].name_str());
}

/// Construit le chemin absolu d'un inode sous forme de chaine.
pub fn path_string(fs: &FileSystem, idx: usize) -> String {
    let mut s = String::new();
    build_path(fs, idx, &mut s);
    if s.is_empty() {
        s.push('/');
    }
    s
}

fn build_path(fs: &FileSystem, idx: usize, s: &mut String) {
    if idx == 0 {
        return;
    }
    build_path(fs, fs.nodes[idx].parent, s);
    s.push('/');
    s.push_str(fs.nodes[idx].name_str());
}

/// Affiche les droits de style `ls -l` (ex. `drwxr-xr-x`).
pub fn print_mode(kind: NodeKind, mode: u16) {
    print!("{}", if kind == NodeKind::Dir { 'd' } else { '-' });
    let bits = [
        0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001,
    ];
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
        let taille = crate::fs::backing::disk_len(idx).unwrap_or(node.content.len());
        print!(" {}:{} {:>4} ", node.uid, node.gid, taille);
    }
    if node.kind == NodeKind::Dir {
        vga::set_color(COLOR_CYAN);
        crate::println!("{}/", node.name_str());
        vga::set_color(COLOR_DEFAULT);
    } else {
        crate::println!("{}", node.name_str());
    }
}
