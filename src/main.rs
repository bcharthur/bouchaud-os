#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use core::arch::asm;
use core::fmt::{self, Write};
use core::panic::PanicInfo;

const VGA_BUFFER: usize = 0xb8000;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

const COLOR_DEFAULT: u8 = 0x0f;
const COLOR_GREEN: u8 = 0x0a;
const COLOR_CYAN: u8 = 0x0b;
const COLOR_RED: u8 = 0x0c;
const COLOR_YELLOW: u8 = 0x0e;

struct VgaWriter {
    row: usize,
    col: usize,
    color: u8,
}

static mut WRITER: VgaWriter = VgaWriter {
    row: 0,
    col: 0,
    color: COLOR_DEFAULT,
};

impl VgaWriter {
    fn set_color(&mut self, color: u8) {
        self.color = color;
    }

    fn clear(&mut self) {
        for row in 0..VGA_HEIGHT {
            for col in 0..VGA_WIDTH {
                self.write_cell(row, col, b' ', self.color);
            }
        }
        self.row = 0;
        self.col = 0;
    }

    fn write_cell(&self, row: usize, col: usize, byte: u8, color: u8) {
        let offset = (row * VGA_WIDTH + col) * 2;
        unsafe {
            core::ptr::write_volatile((VGA_BUFFER + offset) as *mut u8, byte);
            core::ptr::write_volatile((VGA_BUFFER + offset + 1) as *mut u8, color);
        }
    }

    fn read_cell(&self, row: usize, col: usize) -> (u8, u8) {
        let offset = (row * VGA_WIDTH + col) * 2;
        unsafe {
            let ch = core::ptr::read_volatile((VGA_BUFFER + offset) as *const u8);
            let color = core::ptr::read_volatile((VGA_BUFFER + offset + 1) as *const u8);
            (ch, color)
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 >= VGA_HEIGHT {
            self.scroll();
        } else {
            self.row += 1;
        }
    }

    fn scroll(&mut self) {
        for row in 1..VGA_HEIGHT {
            for col in 0..VGA_WIDTH {
                let (ch, color) = self.read_cell(row, col);
                self.write_cell(row - 1, col, ch, color);
            }
        }
        for col in 0..VGA_WIDTH {
            self.write_cell(VGA_HEIGHT - 1, col, b' ', self.color);
        }
        self.row = VGA_HEIGHT - 1;
        self.col = 0;
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
            self.write_cell(self.row, self.col, b' ', self.color);
        }
    }

    fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            b'\r' => self.col = 0,
            8 => self.backspace(),
            byte => {
                if self.col >= VGA_WIDTH {
                    self.newline();
                }
                self.write_cell(self.row, self.col, byte, self.color);
                self.col += 1;
            }
        }
    }

    fn write_str_raw(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                0x20..=0x7e | b'\n' | b'\r' => self.write_byte(byte),
                _ => self.write_byte(b'?'),
            }
        }
    }
}

impl Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str_raw(s);
        Ok(())
    }
}

macro_rules! kprint {
    ($($arg:tt)*) => ({
        unsafe {
            let _ = write!(WRITER, $($arg)*);
        }
    });
}

macro_rules! kprintln {
    () => (kprint!("\n"));
    ($($arg:tt)*) => ({
        kprint!($($arg)*);
        kprint!("\n");
    });
}

fn set_color(color: u8) {
    unsafe { WRITER.set_color(color); }
}

fn clear_screen() {
    unsafe { WRITER.clear(); }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    set_color(COLOR_RED);
    kprintln!("\n[KERNEL PANIC] {}", info);
    loop {
        unsafe { asm!("hlt"); }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    clear_screen();
    set_color(COLOR_GREEN);
    kprintln!("Bouchaud OS");
    set_color(COLOR_DEFAULT);
    kprintln!("Version: 0.3.0 - mini Unix-like + RAMFS");
    kprintln!("Commandes: help, uname, ls, tree, pwd, cd, mkdir, touch, write, append,");
    kprintln!("           cat, echo, nano, stat, cp, mv, rm, rmdir, clear");
    kprintln!();

    unsafe { FS.init(); }
    shell_loop();
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!(
        "in al, dx",
        out("al") value,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    value
}

static mut SHIFT: bool = false;

fn keyboard_read_char() -> u8 {
    loop {
        unsafe {
            let status = inb(0x64);
            if status & 1 == 0 {
                continue;
            }

            let scancode = inb(0x60);
            if let Some(ch) = decode_scancode(scancode) {
                return ch;
            }
        }
    }
}

fn decode_scancode(scancode: u8) -> Option<u8> {
    unsafe {
        match scancode {
            0x2a | 0x36 => {
                SHIFT = true;
                return None;
            }
            0xaa | 0xb6 => {
                SHIFT = false;
                return None;
            }
            _ => {}
        }

        if scancode & 0x80 != 0 {
            return None;
        }

        let shift = SHIFT;
        let ch = match scancode {
            0x01 => 27,
            0x02 => if shift { b'!' } else { b'1' },
            0x03 => if shift { b'@' } else { b'2' },
            0x04 => if shift { b'#' } else { b'3' },
            0x05 => if shift { b'$' } else { b'4' },
            0x06 => if shift { b'%' } else { b'5' },
            0x07 => if shift { b'^' } else { b'6' },
            0x08 => if shift { b'&' } else { b'7' },
            0x09 => if shift { b'*' } else { b'8' },
            0x0a => if shift { b'(' } else { b'9' },
            0x0b => if shift { b')' } else { b'0' },
            0x0c => if shift { b'_' } else { b'-' },
            0x0d => if shift { b'+' } else { b'=' },
            0x0e => 8,
            0x0f => b'\t',
            0x10 => letter(b'q', shift),
            0x11 => letter(b'w', shift),
            0x12 => letter(b'e', shift),
            0x13 => letter(b'r', shift),
            0x14 => letter(b't', shift),
            0x15 => letter(b'y', shift),
            0x16 => letter(b'u', shift),
            0x17 => letter(b'i', shift),
            0x18 => letter(b'o', shift),
            0x19 => letter(b'p', shift),
            0x1a => if shift { b'{' } else { b'[' },
            0x1b => if shift { b'}' } else { b']' },
            0x1c => b'\n',
            0x1e => letter(b'a', shift),
            0x1f => letter(b's', shift),
            0x20 => letter(b'd', shift),
            0x21 => letter(b'f', shift),
            0x22 => letter(b'g', shift),
            0x23 => letter(b'h', shift),
            0x24 => letter(b'j', shift),
            0x25 => letter(b'k', shift),
            0x26 => letter(b'l', shift),
            0x27 => if shift { b':' } else { b';' },
            0x28 => if shift { b'"' } else { b'\'' },
            0x29 => if shift { b'~' } else { b'`' },
            0x2b => if shift { b'|' } else { b'\\' },
            0x2c => letter(b'z', shift),
            0x2d => letter(b'x', shift),
            0x2e => letter(b'c', shift),
            0x2f => letter(b'v', shift),
            0x30 => letter(b'b', shift),
            0x31 => letter(b'n', shift),
            0x32 => letter(b'm', shift),
            0x33 => if shift { b'<' } else { b',' },
            0x34 => if shift { b'>' } else { b'.' },
            0x35 => if shift { b'?' } else { b'/' },
            0x39 => b' ',
            _ => return None,
        };

        Some(ch)
    }
}

fn letter(c: u8, shift: bool) -> u8 {
    if shift { c - 32 } else { c }
}

fn read_line(buffer: &mut [u8]) -> usize {
    let mut len = 0;
    loop {
        let ch = keyboard_read_char();
        match ch {
            b'\n' => {
                kprintln!();
                return len;
            }
            8 => {
                if len > 0 {
                    len -= 1;
                    unsafe { WRITER.backspace(); }
                }
            }
            b'\t' => {}
            27 => {}
            0x20..=0x7e => {
                if len < buffer.len() {
                    buffer[len] = ch;
                    len += 1;
                    kprint!("{}", ch as char);
                }
            }
            _ => {}
        }
    }
}

const MAX_NODES: usize = 128;
const MAX_NAME: usize = 32;
const MAX_CONTENT: usize = 1024;
const MAX_COMPONENTS: usize = 16;
const ROOT: usize = 0;

const KIND_FREE: u8 = 0;
const KIND_FILE: u8 = 1;
const KIND_DIR: u8 = 2;

#[derive(Clone, Copy)]
struct Node {
    used: bool,
    kind: u8,
    parent: usize,
    name: [u8; MAX_NAME],
    name_len: usize,
    content: [u8; MAX_CONTENT],
    content_len: usize,
}

const EMPTY_NODE: Node = Node {
    used: false,
    kind: KIND_FREE,
    parent: 0,
    name: [0; MAX_NAME],
    name_len: 0,
    content: [0; MAX_CONTENT],
    content_len: 0,
};

struct FileSystem {
    nodes: [Node; MAX_NODES],
    cwd: usize,
}

static mut FS: FileSystem = FileSystem::new();

impl FileSystem {
    const fn new() -> Self {
        Self {
            nodes: [EMPTY_NODE; MAX_NODES],
            cwd: ROOT,
        }
    }

    fn init(&mut self) {
        self.nodes = [EMPTY_NODE; MAX_NODES];
        self.cwd = ROOT;
        self.nodes[ROOT] = Node {
            used: true,
            kind: KIND_DIR,
            parent: ROOT,
            name: [0; MAX_NAME],
            name_len: 0,
            content: [0; MAX_CONTENT],
            content_len: 0,
        };

        let _ = self.mkdir("/home");
        let _ = self.mkdir("/tmp");
        let _ = self.mkdir("/etc");
        let _ = self.write_file("/readme.txt", "Bienvenue dans Bouchaud OS. Tape help pour les commandes.");
        let _ = self.write_file("/etc/os-release", "NAME=Bouchaud OS\nVERSION=0.3.0\nTYPE=mini-unix-like-ramfs");
    }

    fn alloc_node(&mut self) -> Option<usize> {
        let mut i = 1;
        while i < MAX_NODES {
            if !self.nodes[i].used {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn name_eq(&self, idx: usize, name: &[u8]) -> bool {
        if self.nodes[idx].name_len != name.len() {
            return false;
        }
        let mut i = 0;
        while i < name.len() {
            if self.nodes[idx].name[i] != name[i] {
                return false;
            }
            i += 1;
        }
        true
    }

    fn find_child(&self, parent: usize, name: &[u8]) -> Option<usize> {
        let mut i = 0;
        while i < MAX_NODES {
            if self.nodes[i].used && self.nodes[i].parent == parent && self.name_eq(i, name) {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn is_dir(&self, idx: usize) -> bool {
        self.nodes[idx].used && self.nodes[idx].kind == KIND_DIR
    }

    fn is_file(&self, idx: usize) -> bool {
        self.nodes[idx].used && self.nodes[idx].kind == KIND_FILE
    }

    fn has_children(&self, idx: usize) -> bool {
        let mut i = 0;
        while i < MAX_NODES {
            if self.nodes[i].used && self.nodes[i].parent == idx && i != idx {
                return true;
            }
            i += 1;
        }
        false
    }

    fn child_count(&self, idx: usize) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < MAX_NODES {
            if self.nodes[i].used && self.nodes[i].parent == idx && i != idx {
                count += 1;
            }
            i += 1;
        }
        count
    }

    fn resolve(&self, path: &str) -> Result<usize, &'static str> {
        let path = trim(path);
        if path.is_empty() || path == "." {
            return Ok(self.cwd);
        }
        if path == "/" {
            return Ok(ROOT);
        }

        let mut comps = [[0u8; MAX_NAME]; MAX_COMPONENTS];
        let mut lens = [0usize; MAX_COMPONENTS];
        let count = tokenize_path(path, &mut comps, &mut lens)?;
        let mut current = if path.as_bytes()[0] == b'/' { ROOT } else { self.cwd };

        let mut i = 0;
        while i < count {
            let comp = &comps[i][..lens[i]];
            if comp_eq(comp, b".") {
                i += 1;
                continue;
            }
            if comp_eq(comp, b"..") {
                current = self.nodes[current].parent;
                i += 1;
                continue;
            }
            if !self.is_dir(current) {
                return Err("chemin parent non dossier");
            }
            match self.find_child(current, comp) {
                Some(child) => current = child,
                None => return Err("chemin introuvable"),
            }
            i += 1;
        }

        Ok(current)
    }

    fn resolve_parent_name(&self, path: &str) -> Result<(usize, [u8; MAX_NAME], usize), &'static str> {
        let path = trim(path);
        if path.is_empty() || path == "/" {
            return Err("nom invalide");
        }

        let mut comps = [[0u8; MAX_NAME]; MAX_COMPONENTS];
        let mut lens = [0usize; MAX_COMPONENTS];
        let count = tokenize_path(path, &mut comps, &mut lens)?;
        if count == 0 {
            return Err("nom invalide");
        }

        let last = &comps[count - 1][..lens[count - 1]];
        if comp_eq(last, b".") || comp_eq(last, b"..") {
            return Err("nom invalide");
        }

        let mut parent = if path.as_bytes()[0] == b'/' { ROOT } else { self.cwd };
        let mut i = 0;
        while i + 1 < count {
            let comp = &comps[i][..lens[i]];
            if comp_eq(comp, b".") {
                i += 1;
                continue;
            }
            if comp_eq(comp, b"..") {
                parent = self.nodes[parent].parent;
                i += 1;
                continue;
            }
            match self.find_child(parent, comp) {
                Some(idx) if self.is_dir(idx) => parent = idx,
                Some(_) => return Err("parent non dossier"),
                None => return Err("parent introuvable"),
            }
            i += 1;
        }

        let mut name = [0u8; MAX_NAME];
        let mut j = 0;
        while j < last.len() {
            name[j] = last[j];
            j += 1;
        }
        Ok((parent, name, last.len()))
    }

    fn create_node(&mut self, path: &str, kind: u8) -> Result<usize, &'static str> {
        let (parent, name, name_len) = self.resolve_parent_name(path)?;
        if !self.is_dir(parent) {
            return Err("parent non dossier");
        }
        if self.find_child(parent, &name[..name_len]).is_some() {
            return Err("existe deja");
        }
        let idx = self.alloc_node().ok_or("RAMFS plein")?;
        self.nodes[idx] = Node {
            used: true,
            kind,
            parent,
            name,
            name_len,
            content: [0; MAX_CONTENT],
            content_len: 0,
        };
        Ok(idx)
    }

    fn mkdir(&mut self, path: &str) -> Result<(), &'static str> {
        let _ = self.create_node(path, KIND_DIR)?;
        Ok(())
    }

    fn touch(&mut self, path: &str) -> Result<(), &'static str> {
        match self.resolve(path) {
            Ok(idx) if self.is_file(idx) => Ok(()),
            Ok(_) => Err("existe mais n'est pas un fichier"),
            Err(_) => {
                let _ = self.create_node(path, KIND_FILE)?;
                Ok(())
            }
        }
    }

    fn get_or_create_file(&mut self, path: &str) -> Result<usize, &'static str> {
        match self.resolve(path) {
            Ok(idx) if self.is_file(idx) => Ok(idx),
            Ok(_) => Err("ce chemin n'est pas un fichier"),
            Err(_) => self.create_node(path, KIND_FILE),
        }
    }

    fn write_file(&mut self, path: &str, text: &str) -> Result<(), &'static str> {
        let idx = self.get_or_create_file(path)?;
        self.nodes[idx].content_len = 0;
        self.append_to_idx(idx, text)
    }

    fn append_file(&mut self, path: &str, text: &str) -> Result<(), &'static str> {
        let idx = self.get_or_create_file(path)?;
        self.append_to_idx(idx, text)
    }

    fn append_to_idx(&mut self, idx: usize, text: &str) -> Result<(), &'static str> {
        let bytes = text.as_bytes();
        if self.nodes[idx].content_len + bytes.len() > MAX_CONTENT {
            return Err("fichier trop grand");
        }
        let mut i = 0;
        while i < bytes.len() {
            self.nodes[idx].content[self.nodes[idx].content_len] = bytes[i];
            self.nodes[idx].content_len += 1;
            i += 1;
        }
        Ok(())
    }

    fn cat(&self, path: &str) -> Result<(), &'static str> {
        let idx = self.resolve(path)?;
        if !self.is_file(idx) {
            return Err("ce n'est pas un fichier");
        }
        let content = &self.nodes[idx].content[..self.nodes[idx].content_len];
        let s = unsafe { core::str::from_utf8_unchecked(content) };
        kprintln!("{}", s);
        Ok(())
    }

    fn ls(&self, path: &str) -> Result<(), &'static str> {
        let idx = if trim(path).is_empty() { self.cwd } else { self.resolve(path)? };
        if self.is_file(idx) {
            self.print_node_line(idx);
            return Ok(());
        }
        if !self.is_dir(idx) {
            return Err("chemin invalide");
        }

        let mut found = false;
        let mut i = 0;
        while i < MAX_NODES {
            if self.nodes[i].used && self.nodes[i].parent == idx && i != idx {
                self.print_node_line(i);
                found = true;
            }
            i += 1;
        }
        if !found {
            kprintln!("(vide)");
        }
        Ok(())
    }

    fn print_node_line(&self, idx: usize) {
        if self.nodes[idx].kind == KIND_DIR {
            set_color(COLOR_CYAN);
            kprint!("[DIR]  ");
        } else {
            set_color(COLOR_YELLOW);
            kprint!("[FILE] ");
        }
        set_color(COLOR_DEFAULT);
        self.print_name(idx);
        if self.nodes[idx].kind == KIND_FILE {
            kprint!("  {} octets", self.nodes[idx].content_len);
        }
        kprintln!();
    }

    fn print_name(&self, idx: usize) {
        let name = &self.nodes[idx].name[..self.nodes[idx].name_len];
        let s = unsafe { core::str::from_utf8_unchecked(name) };
        kprint!("{}", s);
    }

    fn print_path(&self, idx: usize) {
        if idx == ROOT {
            kprint!("/");
            return;
        }
        let mut chain = [0usize; MAX_COMPONENTS];
        let mut count = 0;
        let mut cur = idx;
        while cur != ROOT && count < MAX_COMPONENTS {
            chain[count] = cur;
            count += 1;
            cur = self.nodes[cur].parent;
        }
        let mut i = count;
        while i > 0 {
            i -= 1;
            kprint!("/");
            self.print_name(chain[i]);
        }
    }

    fn pwd(&self) {
        self.print_path(self.cwd);
        kprintln!();
    }

    fn cd(&mut self, path: &str) -> Result<(), &'static str> {
        let idx = self.resolve(path)?;
        if !self.is_dir(idx) {
            return Err("ce n'est pas un dossier");
        }
        self.cwd = idx;
        Ok(())
    }

    fn rm(&mut self, path: &str) -> Result<(), &'static str> {
        let idx = self.resolve(path)?;
        if idx == ROOT {
            return Err("impossible de supprimer /");
        }
        if !self.is_file(idx) {
            return Err("utilise rmdir pour un dossier");
        }
        self.nodes[idx] = EMPTY_NODE;
        Ok(())
    }

    fn rmdir(&mut self, path: &str) -> Result<(), &'static str> {
        let idx = self.resolve(path)?;
        if idx == ROOT {
            return Err("impossible de supprimer /");
        }
        if !self.is_dir(idx) {
            return Err("ce n'est pas un dossier");
        }
        if self.has_children(idx) {
            return Err("dossier non vide");
        }
        if self.cwd == idx {
            self.cwd = self.nodes[idx].parent;
        }
        self.nodes[idx] = EMPTY_NODE;
        Ok(())
    }

    fn stat(&self, path: &str) -> Result<(), &'static str> {
        let idx = self.resolve(path)?;
        kprint!("Path: ");
        self.print_path(idx);
        kprintln!();
        kprintln!("Index: {}", idx);
        if self.is_dir(idx) {
            kprintln!("Type: directory");
            kprintln!("Children: {}", self.child_count(idx));
        } else if self.is_file(idx) {
            kprintln!("Type: regular file");
            kprintln!("Size: {} octets", self.nodes[idx].content_len);
        }
        Ok(())
    }

    fn cp(&mut self, src: &str, dst: &str) -> Result<(), &'static str> {
        let src_idx = self.resolve(src)?;
        if !self.is_file(src_idx) {
            return Err("cp supporte seulement les fichiers pour l'instant");
        }
        let dst_idx = self.get_or_create_file(dst)?;
        self.nodes[dst_idx].content_len = 0;
        let mut i = 0;
        while i < self.nodes[src_idx].content_len {
            self.nodes[dst_idx].content[i] = self.nodes[src_idx].content[i];
            i += 1;
        }
        self.nodes[dst_idx].content_len = self.nodes[src_idx].content_len;
        Ok(())
    }

    fn mv(&mut self, src: &str, dst: &str) -> Result<(), &'static str> {
        let src_idx = self.resolve(src)?;
        if src_idx == ROOT {
            return Err("impossible de deplacer /");
        }
        let (parent, name, name_len) = self.resolve_parent_name(dst)?;
        if self.find_child(parent, &name[..name_len]).is_some() {
            return Err("destination existe deja");
        }
        if self.is_dir(src_idx) && self.is_descendant(parent, src_idx) {
            return Err("deplacement recursif interdit");
        }
        self.nodes[src_idx].parent = parent;
        self.nodes[src_idx].name = name;
        self.nodes[src_idx].name_len = name_len;
        Ok(())
    }

    fn is_descendant(&self, mut candidate: usize, ancestor: usize) -> bool {
        while candidate != ROOT {
            if candidate == ancestor {
                return true;
            }
            candidate = self.nodes[candidate].parent;
        }
        ancestor == ROOT
    }

    fn tree(&self, path: &str) -> Result<(), &'static str> {
        let idx = if trim(path).is_empty() { self.cwd } else { self.resolve(path)? };
        self.print_path(idx);
        kprintln!();
        self.tree_rec(idx, 0);
        Ok(())
    }

    fn tree_rec(&self, parent: usize, depth: usize) {
        let mut i = 0;
        while i < MAX_NODES {
            if self.nodes[i].used && self.nodes[i].parent == parent && i != parent {
                let mut d = 0;
                while d < depth {
                    kprint!("  ");
                    d += 1;
                }
                kprint!("|- ");
                self.print_name(i);
                if self.is_dir(i) {
                    kprintln!("/");
                    if depth < 8 {
                        self.tree_rec(i, depth + 1);
                    }
                } else {
                    kprintln!(" ({} octets)", self.nodes[i].content_len);
                }
            }
            i += 1;
        }
    }
}

fn tokenize_path(
    path: &str,
    comps: &mut [[u8; MAX_NAME]; MAX_COMPONENTS],
    lens: &mut [usize; MAX_COMPONENTS],
) -> Result<usize, &'static str> {
    let bytes = path.as_bytes();
    let mut count = 0;
    let mut pos = 0;

    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos] == b'/' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        let start = pos;
        while pos < bytes.len() && bytes[pos] != b'/' {
            pos += 1;
        }
        let len = pos - start;
        if len == 0 {
            continue;
        }
        if len > MAX_NAME {
            return Err("nom trop long");
        }
        if count >= MAX_COMPONENTS {
            return Err("chemin trop profond");
        }
        let mut j = 0;
        while j < len {
            comps[count][j] = bytes[start + j];
            j += 1;
        }
        lens[count] = len;
        count += 1;
    }

    Ok(count)
}

fn comp_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn trim(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut end = bytes.len();
    while start < end && is_space(bytes[start]) {
        start += 1;
    }
    while end > start && is_space(bytes[end - 1]) {
        end -= 1;
    }
    &s[start..end]
}

fn is_space(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n' || b == b'\r'
}

fn split_first(s: &str) -> (&str, &str) {
    let s = trim(s);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !is_space(bytes[i]) {
        i += 1;
    }
    let first = &s[..i];
    let rest = if i < bytes.len() { trim(&s[i..]) } else { "" };
    (first, rest)
}

fn shell_loop() -> ! {
    let mut line_buf = [0u8; 256];
    loop {
        prompt();
        let len = read_line(&mut line_buf);
        let line = unsafe { core::str::from_utf8_unchecked(&line_buf[..len]) };
        execute_command(trim(line));
    }
}

fn prompt() {
    set_color(COLOR_GREEN);
    kprint!("bouchaud-os:");
    set_color(COLOR_CYAN);
    unsafe { FS.print_path(FS.cwd); }
    set_color(COLOR_GREEN);
    kprint!("$ ");
    set_color(COLOR_DEFAULT);
}

fn execute_command(line: &str) {
    if line.is_empty() {
        return;
    }

    let (cmd, rest) = split_first(line);
    match cmd {
        "help" => cmd_help(),
        "uname" => kprintln!("Bouchaud OS 0.3.0 x86_64 no_std ramfs"),
        "clear" => clear_screen(),
        "pwd" => unsafe { FS.pwd(); },
        "ls" => run_result(unsafe { FS.ls(rest) }),
        "dir" => run_result(unsafe { FS.ls(rest) }),
        "cd" => run_result(unsafe { FS.cd(if rest.is_empty() { "/" } else { rest }) }),
        "mkdir" => run_result_arg(rest, |arg| unsafe { FS.mkdir(arg) }),
        "touch" => run_result_arg(rest, |arg| unsafe { FS.touch(arg) }),
        "cat" => run_result_arg(rest, |arg| unsafe { FS.cat(arg) }),
        "type" => run_result_arg(rest, |arg| unsafe { FS.cat(arg) }),
        "rm" => run_result_arg(rest, |arg| unsafe { FS.rm(arg) }),
        "rmdir" => run_result_arg(rest, |arg| unsafe { FS.rmdir(arg) }),
        "stat" => run_result_arg(rest, |arg| unsafe { FS.stat(arg) }),
        "tree" => run_result(unsafe { FS.tree(rest) }),
        "echo" => kprintln!("{}", rest),
        "write" => cmd_write(rest, false),
        "append" => cmd_write(rest, true),
        "nano" => cmd_nano(rest),
        "cp" => cmd_cp(rest),
        "mv" => cmd_mv(rest),
        _ => {
            set_color(COLOR_RED);
            kprintln!("commande inconnue: {}", cmd);
            set_color(COLOR_DEFAULT);
        }
    }
}

fn run_result_arg<F>(arg: &str, f: F)
where
    F: FnOnce(&str) -> Result<(), &'static str>,
{
    if trim(arg).is_empty() {
        print_error("argument manquant");
        return;
    }
    run_result(f(arg));
}

fn run_result(result: Result<(), &'static str>) {
    if let Err(e) = result {
        print_error(e);
    }
}

fn print_error(e: &str) {
    set_color(COLOR_RED);
    kprintln!("erreur: {}", e);
    set_color(COLOR_DEFAULT);
}

fn cmd_write(rest: &str, append: bool) {
    let (file, text) = split_first(rest);
    if file.is_empty() {
        print_error("usage: write <file> <texte> ou append <file> <texte>");
        return;
    }
    let result = if append {
        unsafe { FS.append_file(file, text) }
    } else {
        unsafe { FS.write_file(file, text) }
    };
    run_result(result);
}

fn cmd_cp(rest: &str) {
    let (src, rest2) = split_first(rest);
    let (dst, _) = split_first(rest2);
    if src.is_empty() || dst.is_empty() {
        print_error("usage: cp <source> <destination>");
        return;
    }
    run_result(unsafe { FS.cp(src, dst) });
}

fn cmd_mv(rest: &str) {
    let (src, rest2) = split_first(rest);
    let (dst, _) = split_first(rest2);
    if src.is_empty() || dst.is_empty() {
        print_error("usage: mv <source> <destination>");
        return;
    }
    run_result(unsafe { FS.mv(src, dst) });
}

fn cmd_nano(rest: &str) {
    let file = trim(rest);
    if file.is_empty() {
        print_error("usage: nano <file>");
        return;
    }
    kprintln!("nano minimal - saisis une ligne puis Entree");
    kprint!("> ");
    let mut buf = [0u8; 512];
    let len = read_line(&mut buf);
    let text = unsafe { core::str::from_utf8_unchecked(&buf[..len]) };
    run_result(unsafe { FS.write_file(file, text) });
}

fn cmd_help() {
    set_color(COLOR_CYAN);
    kprintln!("Commandes Unix-like disponibles:");
    set_color(COLOR_DEFAULT);
    kprintln!("  help                    affiche cette aide");
    kprintln!("  uname                   infos systeme");
    kprintln!("  clear                   nettoie l'ecran");
    kprintln!("  pwd                     affiche le chemin courant");
    kprintln!("  ls [path]               liste un dossier");
    kprintln!("  tree [path]             affiche l'arborescence");
    kprintln!("  cd <path>               change de dossier");
    kprintln!("  mkdir <path>            cree un dossier");
    kprintln!("  touch <path>            cree un fichier vide");
    kprintln!("  write <file> <text>     ecrit/remplace un fichier");
    kprintln!("  append <file> <text>    ajoute du texte a un fichier");
    kprintln!("  cat <file>              affiche un fichier");
    kprintln!("  echo <text>             affiche du texte");
    kprintln!("  nano <file>             mini editeur une ligne");
    kprintln!("  stat <path>             infos fichier/dossier");
    kprintln!("  cp <src> <dst>          copie un fichier");
    kprintln!("  mv <src> <dst>          renomme/deplace");
    kprintln!("  rm <file>               supprime un fichier");
    kprintln!("  rmdir <dir>             supprime un dossier vide");
    kprintln!();
    kprintln!("Chemins supportes: /, ., .., /home/test.txt, ../readme.txt");
    kprintln!("Limite actuelle: fichiers en RAM seulement, non persistants.");
}
