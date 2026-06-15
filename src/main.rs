#![no_std]
#![no_main]
#![allow(dead_code)]
#![allow(static_mut_refs)]

use bootloader::{entry_point, BootInfo};
use core::arch::asm;
use core::fmt;
use core::panic::PanicInfo;

entry_point!(kernel_main);

const VERSION: &str = "0.5.0";
const OS_NAME: &str = "Bouchaud OS";


#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::_print(format_args!($($arg)*))
    }};
}

#[macro_export]
macro_rules! println {
    () => {{
        $crate::print!("\n")
    }};
    ($fmt:expr) => {{
        $crate::print!(concat!($fmt, "\n"))
    }};
    ($fmt:expr, $($arg:tt)*) => {{
        $crate::print!(concat!($fmt, "\n"), $($arg)*)
    }};
}

fn kernel_main(_boot_info: &'static BootInfo) -> ! {
    vga_clear();

    println!("Bouchaud OS");
    println!("Version: {} - fondations systeme CLI", VERSION);
    println!("Clavier: AZERTY-FR | Shell: Unix-like | FS: RAMFS");
    println!("Modules: session, sysinfo, cpuinfo, devices, dmesg, permissions simples");
    println!("");

    unsafe {
        FS.init();
        DMESG.init();
        DMESG.push("kernel: boot Bouchaud OS V0.5");
        DMESG.push("vga: text mode initialised");
        DMESG.push("keyboard: ps2 polling azerty-fr active");
        DMESG.push("fs: ramfs mounted on /");
        DMESG.push("session: default user root");
        SESSION.login(User::Root);
    }

    shell_loop();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("");
    println!("*** KERNEL PANIC ***");
    println!("{}", info);
    halt_loop();
}

fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)); }
    }
}

// ================================================================================================
// VGA TEXT OUTPUT
// ================================================================================================

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

static mut VGA: VgaWriter = VgaWriter {
    row: 0,
    col: 0,
    color: COLOR_DEFAULT,
};

impl VgaWriter {
    fn clear(&mut self) {
        for row in 0..VGA_HEIGHT {
            for col in 0..VGA_WIDTH {
                self.write_cell(row, col, b' ', self.color);
            }
        }
        self.row = 0;
        self.col = 0;
    }

    fn set_color(&mut self, color: u8) {
        self.color = color;
    }

    fn write_cell(&self, row: usize, col: usize, byte: u8, color: u8) {
        let offset = (row * VGA_WIDTH + col) * 2;
        unsafe {
            let ptr = (VGA_BUFFER + offset) as *mut u8;
            core::ptr::write_volatile(ptr, byte);
            core::ptr::write_volatile(ptr.add(1), color);
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
                let src_offset = (row * VGA_WIDTH + col) * 2;
                let dst_offset = ((row - 1) * VGA_WIDTH + col) * 2;
                unsafe {
                    let src = (VGA_BUFFER + src_offset) as *const u8;
                    let dst = (VGA_BUFFER + dst_offset) as *mut u8;
                    let ch = core::ptr::read_volatile(src);
                    let color = core::ptr::read_volatile(src.add(1));
                    core::ptr::write_volatile(dst, ch);
                    core::ptr::write_volatile(dst.add(1), color);
                }
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

    fn byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
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
}

impl fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            match byte {
                0x08 | 0x20..=0x7e | b'\n' => self.byte(byte),
                _ => self.byte(b'?'),
            }
        }
        Ok(())
    }
}

fn vga_clear() {
    unsafe { VGA.clear(); }
}

fn vga_color(color: u8) {
    unsafe { VGA.set_color(color); }
}

fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    unsafe { let _ = VGA.write_fmt(args); }
}


// ================================================================================================
// PORT I/O + KEYBOARD AZERTY-FR
// ================================================================================================

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
    value
}

unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

fn keyboard_read_scancode() -> u8 {
    loop {
        let status = unsafe { inb(0x64) };
        if status & 1 != 0 {
            return unsafe { inb(0x60) };
        }
    }
}

fn ascii_letter(ch: u8, shift: bool) -> char {
    if shift && ch >= b'a' && ch <= b'z' {
        (ch - 32) as char
    } else {
        ch as char
    }
}

fn azerty_scancode_to_char(sc: u8, shift: bool) -> Option<char> {
    match sc {
        0x01 => Some('\x1b'),
        0x0e => Some('\x08'),
        0x0f => Some('\t'),
        0x1c => Some('\n'),
        0x39 => Some(' '),

        // Ligne numerique AZERTY. Les accents sont translitteres pour l'instant.
        0x02 => Some(if shift { '1' } else { '&' }),
        0x03 => Some(if shift { '2' } else { 'e' }),
        0x04 => Some(if shift { '3' } else { '"' }),
        0x05 => Some(if shift { '4' } else { '\'' }),
        0x06 => Some(if shift { '5' } else { '(' }),
        0x07 => Some(if shift { '6' } else { '-' }),
        0x08 => Some(if shift { '7' } else { 'e' }),
        0x09 => Some(if shift { '8' } else { '_' }),
        0x0a => Some(if shift { '9' } else { 'c' }),
        0x0b => Some(if shift { '0' } else { 'a' }),
        0x0c => Some(if shift { ')' } else { ')' }),
        0x0d => Some(if shift { '+' } else { '=' }),

        // AZERTY lettres principales
        0x10 => Some(ascii_letter(b'a', shift)),
        0x11 => Some(ascii_letter(b'z', shift)),
        0x12 => Some(ascii_letter(b'e', shift)),
        0x13 => Some(ascii_letter(b'r', shift)),
        0x14 => Some(ascii_letter(b't', shift)),
        0x15 => Some(ascii_letter(b'y', shift)),
        0x16 => Some(ascii_letter(b'u', shift)),
        0x17 => Some(ascii_letter(b'i', shift)),
        0x18 => Some(ascii_letter(b'o', shift)),
        0x19 => Some(ascii_letter(b'p', shift)),
        0x1a => Some(if shift { '^' } else { '^' }),
        0x1b => Some(if shift { '*' } else { '$' }),

        0x1e => Some(ascii_letter(b'q', shift)),
        0x1f => Some(ascii_letter(b's', shift)),
        0x20 => Some(ascii_letter(b'd', shift)),
        0x21 => Some(ascii_letter(b'f', shift)),
        0x22 => Some(ascii_letter(b'g', shift)),
        0x23 => Some(ascii_letter(b'h', shift)),
        0x24 => Some(ascii_letter(b'j', shift)),
        0x25 => Some(ascii_letter(b'k', shift)),
        0x26 => Some(ascii_letter(b'l', shift)),
        0x27 => Some(ascii_letter(b'm', shift)),
        0x28 => Some(if shift { '%' } else { 'u' }),
        0x2b => Some(if shift { '|' } else { '*' }),

        0x2c => Some(ascii_letter(b'w', shift)),
        0x2d => Some(ascii_letter(b'x', shift)),
        0x2e => Some(ascii_letter(b'c', shift)),
        0x2f => Some(ascii_letter(b'v', shift)),
        0x30 => Some(ascii_letter(b'b', shift)),
        0x31 => Some(ascii_letter(b'n', shift)),
        0x32 => Some(if shift { '?' } else { ',' }),
        0x33 => Some(if shift { '.' } else { ';' }),
        0x34 => Some(if shift { '/' } else { ':' }),
        0x35 => Some(if shift { '/' } else { '!' }),
        _ => None,
    }
}

fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    let mut shift = false;

    loop {
        let sc = keyboard_read_scancode();

        // Certaines touches PS/2 sont envoyees avec le prefixe etendu 0xE0.
        // Exemple : Suppr/Delete = E0 53. Comme l'editeur de ligne n'a pas encore
        // de curseur horizontal, on mappe Suppr sur le meme comportement que Backspace.
        if sc == 0xe0 {
            let ext = keyboard_read_scancode();
            if ext == 0x53 {
                if len > 0 {
                    len -= 1;
                    print!("\x08");
                }
            }
            continue;
        }

        match sc {
            0x2a | 0x36 => { shift = true; continue; }
            0xaa | 0xb6 => { shift = false; continue; }
            _ => {}
        }

        if sc & 0x80 != 0 {
            continue;
        }

        if let Some(ch) = azerty_scancode_to_char(sc, shift) {
            match ch {
                '\n' => {
                    println!("");
                    return len;
                }
                '\x08' => {
                    if len > 0 {
                        len -= 1;
                        print!("\x08");
                    }
                }
                '\t' => {
                    if len < buf.len() {
                        buf[len] = b' ';
                        len += 1;
                        print!(" ");
                    }
                }
                ch => {
                    if len < buf.len() && ch.is_ascii() {
                        buf[len] = ch as u8;
                        len += 1;
                        print!("{}", ch);
                    }
                }
            }
        }
    }
}

// ================================================================================================
// DMESG
// ================================================================================================

const DMESG_MAX: usize = 32;
const DMESG_LEN: usize = 96;

struct Dmesg {
    entries: [[u8; DMESG_LEN]; DMESG_MAX],
    lens: [usize; DMESG_MAX],
    count: usize,
}

static mut DMESG: Dmesg = Dmesg {
    entries: [[0; DMESG_LEN]; DMESG_MAX],
    lens: [0; DMESG_MAX],
    count: 0,
};

impl Dmesg {
    fn init(&mut self) {
        self.count = 0;
        self.push("dmesg: ring buffer initialized");
    }

    fn push(&mut self, msg: &str) {
        let index = if self.count < DMESG_MAX { self.count } else { DMESG_MAX - 1 };
        if self.count >= DMESG_MAX {
            for i in 1..DMESG_MAX {
                self.entries[i - 1] = self.entries[i];
                self.lens[i - 1] = self.lens[i];
            }
        }
        let bytes = msg.as_bytes();
        let mut i = 0;
        while i < bytes.len() && i < DMESG_LEN {
            self.entries[index][i] = bytes[i];
            i += 1;
        }
        self.lens[index] = i;
        if self.count < DMESG_MAX { self.count += 1; }
    }

    fn print(&self) {
        for i in 0..self.count {
            print!("[{:02}] ", i);
            for j in 0..self.lens[i] {
                print!("{}", self.entries[i][j] as char);
            }
            println!("");
        }
    }
}

// ================================================================================================
// SESSION / USERS
// ================================================================================================

#[derive(Copy, Clone, PartialEq)]
enum User {
    Root,
    Arthur,
    Guest,
}

struct Session {
    current: User,
}

static mut SESSION: Session = Session { current: User::Root };

impl Session {
    fn login(&mut self, user: User) {
        self.current = user;
    }

    fn username(&self) -> &'static str {
        match self.current {
            User::Root => "root",
            User::Arthur => "arthur",
            User::Guest => "guest",
        }
    }

    fn uid(&self) -> u16 {
        match self.current {
            User::Root => 0,
            User::Arthur => 1000,
            User::Guest => 65534,
        }
    }

    fn gid(&self) -> u16 {
        match self.current {
            User::Root => 0,
            User::Arthur => 1000,
            User::Guest => 65534,
        }
    }
}

fn user_from_name(name: &str) -> Option<User> {
    match name {
        "root" => Some(User::Root),
        "arthur" => Some(User::Arthur),
        "guest" => Some(User::Guest),
        _ => None,
    }
}

// ================================================================================================
// RAM FILESYSTEM
// ================================================================================================

const MAX_NODES: usize = 96;
const NAME_LEN: usize = 32;
const CONTENT_LEN: usize = 768;

#[derive(Copy, Clone, PartialEq)]
enum NodeKind {
    File,
    Dir,
}

#[derive(Copy, Clone)]
struct Node {
    used: bool,
    kind: NodeKind,
    parent: usize,
    name: [u8; NAME_LEN],
    name_len: usize,
    content: [u8; CONTENT_LEN],
    content_len: usize,
    mode: u16,
    uid: u16,
    gid: u16,
}

impl Node {
    const fn empty() -> Self {
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

    fn name_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.name[..self.name_len]) }
    }

    fn name_eq(&self, name: &str) -> bool {
        if self.name_len != name.len() { return false; }
        let bytes = name.as_bytes();
        for i in 0..self.name_len {
            if self.name[i] != bytes[i] { return false; }
        }
        true
    }

    fn set_name(&mut self, name: &str) -> bool {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > NAME_LEN { return false; }
        for i in 0..NAME_LEN { self.name[i] = 0; }
        for i in 0..bytes.len() { self.name[i] = bytes[i]; }
        self.name_len = bytes.len();
        true
    }
}

struct FileSystem {
    nodes: [Node; MAX_NODES],
}

static mut FS: FileSystem = FileSystem { nodes: [Node::empty(); MAX_NODES] };

impl FileSystem {
    fn init(&mut self) {
        self.nodes = [Node::empty(); MAX_NODES];

        self.nodes[0].used = true;
        self.nodes[0].kind = NodeKind::Dir;
        self.nodes[0].parent = 0;
        self.nodes[0].mode = 0o755;
        self.nodes[0].uid = 0;
        self.nodes[0].gid = 0;

        let home = self.mkdir_at(0, "home").unwrap_or(0);
        let arthur = self.mkdir_at(home, "arthur").unwrap_or(0);
        let tmp = self.mkdir_at(0, "tmp").unwrap_or(0);
        let etc = self.mkdir_at(0, "etc").unwrap_or(0);
        let var = self.mkdir_at(0, "var").unwrap_or(0);
        let _log = self.mkdir_at(var, "log");

        let readme = self.touch_at(0, "readme.txt").unwrap_or(0);
        self.write_node(readme, "Bienvenue dans Bouchaud OS V0.5. Tape help.");

        let passwd = self.touch_at(etc, "passwd").unwrap_or(0);
        self.write_node(passwd, "root:x:0:0:root:/root:/bin/bsh\narthur:x:1000:1000:arthur:/home/arthur:/bin/bsh\nguest:x:65534:65534:guest:/tmp:/bin/bsh");

        let note = self.touch_at(arthur, "note.txt").unwrap_or(0);
        self.write_node(note, "Session utilisateur presente. FS encore en RAM.");

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

    fn find_child(&self, parent: usize, name: &str) -> Option<usize> {
        for i in 0..MAX_NODES {
            if self.nodes[i].used && self.nodes[i].parent == parent && self.nodes[i].name_eq(name) {
                return Some(i);
            }
        }
        None
    }

    fn mkdir_at(&mut self, parent: usize, name: &str) -> Result<usize, &'static str> {
        if self.nodes[parent].kind != NodeKind::Dir { return Err("parent not a directory"); }
        if self.find_child(parent, name).is_some() { return Err("already exists"); }
        let idx = self.alloc_node().ok_or("no free inode")?;
        self.nodes[idx].kind = NodeKind::Dir;
        self.nodes[idx].parent = parent;
        self.nodes[idx].mode = 0o755;
        if !self.nodes[idx].set_name(name) { return Err("invalid name"); }
        Ok(idx)
    }

    fn touch_at(&mut self, parent: usize, name: &str) -> Result<usize, &'static str> {
        if self.nodes[parent].kind != NodeKind::Dir { return Err("parent not a directory"); }
        if let Some(existing) = self.find_child(parent, name) { return Ok(existing); }
        let idx = self.alloc_node().ok_or("no free inode")?;
        self.nodes[idx].kind = NodeKind::File;
        self.nodes[idx].parent = parent;
        self.nodes[idx].mode = 0o644;
        if !self.nodes[idx].set_name(name) { return Err("invalid name"); }
        Ok(idx)
    }

    fn write_node(&mut self, idx: usize, text: &str) {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() && i < CONTENT_LEN {
            self.nodes[idx].content[i] = bytes[i];
            i += 1;
        }
        self.nodes[idx].content_len = i;
    }

    fn append_node(&mut self, idx: usize, text: &str) {
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

    fn resolve(&self, path: &str, cwd: usize) -> Option<usize> {
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

    fn resolve_parent_name<'a>(&self, path: &'a str, cwd: usize) -> Option<(usize, &'a str)> {
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

    fn is_empty_dir(&self, idx: usize) -> bool {
        for i in 0..MAX_NODES {
            if self.nodes[i].used && i != idx && self.nodes[i].parent == idx {
                return false;
            }
        }
        true
    }

    fn used_nodes(&self) -> usize {
        let mut n = 0;
        for i in 0..MAX_NODES {
            if self.nodes[i].used { n += 1; }
        }
        n
    }

    fn free_nodes(&self) -> usize {
        MAX_NODES - self.used_nodes()
    }
}

fn print_path(fs: &FileSystem, idx: usize) {
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

fn print_mode(kind: NodeKind, mode: u16) {
    print!("{}", if kind == NodeKind::Dir { 'd' } else { '-' });
    let bits = [0o400,0o200,0o100,0o040,0o020,0o010,0o004,0o002,0o001];
    let chars = ['r','w','x','r','w','x','r','w','x'];
    for i in 0..9 {
        print!("{}", if mode & bits[i] != 0 { chars[i] } else { '-' });
    }
}

// ================================================================================================
// HARDWARE DISCOVERY
// ================================================================================================

#[cfg(target_arch = "x86_64")]
fn cpu_vendor() -> [u8; 12] {
    use core::arch::x86_64::__cpuid;
    let res = unsafe { __cpuid(0) };
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&res.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&res.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&res.ecx.to_le_bytes());
    vendor
}

#[cfg(target_arch = "x86_64")]
fn print_cpuinfo() {
    use core::arch::x86_64::__cpuid;
    let vendor = cpu_vendor();
    print!("vendor_id: ");
    for b in vendor { print!("{}", b as char); }
    println!("");

    let leaf1 = unsafe { __cpuid(1) };
    let family = ((leaf1.eax >> 8) & 0xf) as u32;
    let model = ((leaf1.eax >> 4) & 0xf) as u32;
    let stepping = (leaf1.eax & 0xf) as u32;
    println!("family: {}", family);
    println!("model: {}", model);
    println!("stepping: {}", stepping);
    println!("features:");
    println!("  sse3={} pclmulqdq={} vmx={} ssse3={}", bit(leaf1.ecx, 0), bit(leaf1.ecx, 1), bit(leaf1.ecx, 5), bit(leaf1.ecx, 9));
    println!("  sse={} sse2={} htt={}", bit(leaf1.edx, 25), bit(leaf1.edx, 26), bit(leaf1.edx, 28));
}

fn bit(value: u32, index: u32) -> &'static str {
    if value & (1u32 << index) != 0 { "yes" } else { "no" }
}

// ================================================================================================
// SHELL
// ================================================================================================

fn shell_loop() -> ! {
    let mut cwd = 0usize;
    let mut line_buf = [0u8; 256];

    loop {
        unsafe {
            vga_color(COLOR_GREEN);
            print!("{}@bouchaud-os:", SESSION.username());
            vga_color(COLOR_CYAN);
            print_path(&FS, cwd);
            vga_color(COLOR_GREEN);
            print!("$ ");
            vga_color(COLOR_DEFAULT);
        }

        let len = read_line(&mut line_buf);
        let line = unsafe { core::str::from_utf8_unchecked(&line_buf[..len]) };
        let trimmed = trim(line);
        if trimmed.is_empty() { continue; }

        unsafe { DMESG.push("shell: command executed"); }
        execute_command(trimmed, &mut cwd);
    }
}

fn trim(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && is_space(bytes[start]) { start += 1; }
    while end > start && is_space(bytes[end - 1]) { end -= 1; }
    &s[start..end]
}

fn is_space(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\r' || b == b'\n'
}

fn tokenize<'a>(line: &'a str, out: &mut [&'a str; 12]) -> usize {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut count = 0usize;
    while i < bytes.len() && count < out.len() {
        while i < bytes.len() && is_space(bytes[i]) { i += 1; }
        if i >= bytes.len() { break; }
        let start = i;
        while i < bytes.len() && !is_space(bytes[i]) { i += 1; }
        out[count] = &line[start..i];
        count += 1;
    }
    count
}

fn remainder_after_tokens<'a>(line: &'a str, n: usize) -> &'a str {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut count = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && is_space(bytes[i]) { i += 1; }
        if i >= bytes.len() { return ""; }
        if count == n { return trim(&line[i..]); }
        while i < bytes.len() && !is_space(bytes[i]) { i += 1; }
        count += 1;
    }
    ""
}

fn execute_command(line: &str, cwd: &mut usize) {
    let mut argv = [""; 12];
    let argc = tokenize(line, &mut argv);
    if argc == 0 { return; }

    match argv[0] {
        "help" => cmd_help(),
        "clear" => vga_clear(),
        "uname" => cmd_uname(),
        "sysinfo" => cmd_sysinfo(),
        "cpuinfo" => cmd_cpuinfo(),
        "meminfo" => cmd_meminfo(),
        "devices" => cmd_devices(),
        "dmesg" => unsafe { DMESG.print(); },
        "uptime" => println!("uptime: timer interrupts not enabled yet"),

        "whoami" => unsafe { println!("{}", SESSION.username()); },
        "id" => cmd_id(),
        "users" => cmd_users(),
        "login" => cmd_login(argc, &argv),
        "logout" => unsafe { SESSION.login(User::Guest); println!("session: guest"); },
        "su" => unsafe { SESSION.login(User::Root); println!("session: root"); },

        "pwd" => unsafe { print_path(&FS, *cwd); println!(""); },
        "ls" => cmd_ls(argc, &argv, *cwd),
        "tree" => cmd_tree(argc, &argv, *cwd),
        "cd" => cmd_cd(argc, &argv, cwd),
        "mkdir" => cmd_mkdir(argc, &argv, *cwd),
        "touch" => cmd_touch(argc, &argv, *cwd),
        "cat" => cmd_cat(argc, &argv, *cwd),
        "write" => cmd_write(line, argc, &argv, *cwd),
        "append" => cmd_append(line, argc, &argv, *cwd),
        "nano" => cmd_nano(argc, &argv, *cwd),
        "rm" => cmd_rm(argc, &argv, *cwd),
        "rmdir" => cmd_rmdir(argc, &argv, *cwd),
        "cp" => cmd_cp(argc, &argv, *cwd),
        "mv" => cmd_mv(argc, &argv, *cwd),
        "stat" => cmd_stat(argc, &argv, *cwd),
        "chmod" => cmd_chmod(argc, &argv, *cwd),
        "echo" => println!("{}", remainder_after_tokens(line, 1)),

        // Roadmap reseau : commandes visibles, pile pas encore implementee.
        "ifconfig" | "ip" | "route" | "arp" | "ping" | "dhcp" | "dns" | "wget" | "curl" => cmd_network_placeholder(argv[0]),
        _ => {
            vga_color(COLOR_RED);
            println!("{}: commande inconnue", argv[0]);
            vga_color(COLOR_DEFAULT);
        }
    }
}

fn cmd_help() {
    vga_color(COLOR_CYAN);
    println!("Commandes Bouchaud OS V0.5:");
    vga_color(COLOR_DEFAULT);
    println!("  help, clear, uname, sysinfo, cpuinfo, meminfo, devices, dmesg, uptime");
    println!("  whoami, id, users, login <root|arthur|guest>, logout, su");
    println!("  pwd, ls [-l] [path], tree [path], cd <path>, mkdir <path>");
    println!("  touch <file>, write <file> <texte>, append <file> <texte>, cat <file>");
    println!("  nano <file>, stat <path>, chmod <mode> <path>, cp <src> <dst>");
    println!("  mv <src> <dst>, rm <file>, rmdir <dir>, echo <texte>");
    println!("  ifconfig, ip, route, arp, ping, dhcp, dns, wget, curl  [roadmap]");
}

fn cmd_uname() {
    println!("Bouchaud OS {} x86_64 cli unix-like rust-no_std", VERSION);
}

fn cmd_sysinfo() {
    println!("os: {}", OS_NAME);
    println!("version: {}", VERSION);
    println!("arch: x86_64");
    println!("keyboard: AZERTY-FR");
    println!("display: VGA text mode");
    println!("filesystem: RAMFS mounted on /");
    println!("security: sessions + permissions simples, no user/kernel split yet");
    println!("network: OSI stack planned, driver not enabled yet");
}

fn cmd_cpuinfo() {
    #[cfg(target_arch = "x86_64")]
    print_cpuinfo();
}

fn cmd_meminfo() {
    unsafe {
        println!("memory model: static kernel memory + RAMFS fixed arrays");
        println!("ramfs inodes: used={} free={} total={}", FS.used_nodes(), FS.free_nodes(), MAX_NODES);
        println!("ramfs max file size: {} bytes", CONTENT_LEN);
        println!("heap allocator: not enabled yet");
        println!("paging/user isolation: roadmap V0.6+");
    }
}

fn cmd_devices() {
    println!("devices detected/configured:");
    println!("  cpu0      x86_64 via CPUID");
    println!("  vga0      legacy VGA text buffer 0xb8000");
    println!("  kbd0      PS/2 keyboard polling, AZERTY-FR mapping");
    println!("  ramfs0    in-memory filesystem mounted on /");
    println!("  serial0   planned");
    println!("  pci0      planned");
    println!("  net0      planned: e1000/virtio-net");
    println!("  disk0     planned: virtio-blk/BFS persistent FS");
}

fn cmd_id() {
    unsafe {
        println!("uid={}({}) gid={}({})", SESSION.uid(), SESSION.username(), SESSION.gid(), SESSION.username());
    }
}

fn cmd_users() {
    println!("root:x:0:0:/root");
    println!("arthur:x:1000:1000:/home/arthur");
    println!("guest:x:65534:65534:/tmp");
}

fn cmd_login(argc: usize, argv: &[&str; 12]) {
    if argc < 2 {
        println!("usage: login <root|arthur|guest>");
        return;
    }
    match user_from_name(argv[1]) {
        Some(user) => unsafe {
            SESSION.login(user);
            println!("session ouverte: {}", SESSION.username());
        },
        None => println!("login: utilisateur inconnu"),
    }
}

fn cmd_ls(argc: usize, argv: &[&str; 12], cwd: usize) {
    let mut long = false;
    let mut path = ".";
    if argc >= 2 {
        if argv[1] == "-l" {
            long = true;
            if argc >= 3 { path = argv[2]; }
        } else {
            path = argv[1];
        }
    }

    unsafe {
        match FS.resolve(path, cwd) {
            Some(idx) => {
                if FS.nodes[idx].kind == NodeKind::File {
                    print_node_line(&FS, idx, long);
                } else {
                    for i in 0..MAX_NODES {
                        if FS.nodes[i].used && i != idx && FS.nodes[i].parent == idx {
                            print_node_line(&FS, i, long);
                        }
                    }
                }
            }
            None => println!("ls: chemin introuvable"),
        }
    }
}

fn print_node_line(fs: &FileSystem, idx: usize, long: bool) {
    let node = &fs.nodes[idx];
    if long {
        print_mode(node.kind, node.mode);
        print!(" {}:{} {:>4} ", node.uid, node.gid, node.content_len);
    }
    if node.kind == NodeKind::Dir {
        vga_color(COLOR_CYAN);
        println!("{}/", node.name_str());
        vga_color(COLOR_DEFAULT);
    } else {
        println!("{}", node.name_str());
    }
}

fn cmd_tree(argc: usize, argv: &[&str; 12], cwd: usize) {
    let path = if argc >= 2 { argv[1] } else { "." };
    unsafe {
        match FS.resolve(path, cwd) {
            Some(idx) => {
                print_path(&FS, idx);
                println!("");
                tree_rec(&FS, idx, 0);
            }
            None => println!("tree: chemin introuvable"),
        }
    }
}

fn tree_rec(fs: &FileSystem, idx: usize, depth: usize) {
    if fs.nodes[idx].kind != NodeKind::Dir { return; }
    for i in 0..MAX_NODES {
        if fs.nodes[i].used && i != idx && fs.nodes[i].parent == idx {
            for _ in 0..depth { print!("  "); }
            if fs.nodes[i].kind == NodeKind::Dir {
                vga_color(COLOR_CYAN);
                println!("|- {}/", fs.nodes[i].name_str());
                vga_color(COLOR_DEFAULT);
                tree_rec(fs, i, depth + 1);
            } else {
                println!("|- {}", fs.nodes[i].name_str());
            }
        }
    }
}

fn cmd_cd(argc: usize, argv: &[&str; 12], cwd: &mut usize) {
    if argc < 2 { *cwd = 0; return; }
    unsafe {
        match FS.resolve(argv[1], *cwd) {
            Some(idx) if FS.nodes[idx].kind == NodeKind::Dir => *cwd = idx,
            Some(_) => println!("cd: pas un dossier"),
            None => println!("cd: chemin introuvable"),
        }
    }
}

fn cmd_mkdir(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: mkdir <path>"); return; }
    unsafe {
        match FS.resolve_parent_name(argv[1], cwd) {
            Some((parent, name)) => match FS.mkdir_at(parent, name) {
                Ok(_) => {},
                Err(e) => println!("mkdir: {}", e),
            },
            None => println!("mkdir: chemin invalide"),
        }
    }
}

fn cmd_touch(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: touch <file>"); return; }
    unsafe {
        match FS.resolve_parent_name(argv[1], cwd) {
            Some((parent, name)) => match FS.touch_at(parent, name) {
                Ok(_) => {},
                Err(e) => println!("touch: {}", e),
            },
            None => println!("touch: chemin invalide"),
        }
    }
}

fn cmd_cat(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: cat <file>"); return; }
    unsafe {
        match FS.resolve(argv[1], cwd) {
            Some(idx) if FS.nodes[idx].kind == NodeKind::File => {
                for i in 0..FS.nodes[idx].content_len {
                    print!("{}", FS.nodes[idx].content[i] as char);
                }
                println!("");
            }
            Some(_) => println!("cat: dossier"),
            None => println!("cat: fichier introuvable"),
        }
    }
}

fn cmd_write(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: write <file> <texte>"); return; }
    let text = remainder_after_tokens(line, 2);
    unsafe {
        match FS.resolve(argv[1], cwd) {
            Some(idx) if FS.nodes[idx].kind == NodeKind::File => FS.write_node(idx, text),
            Some(_) => println!("write: dossier"),
            None => println!("write: fichier introuvable"),
        }
    }
}

fn cmd_append(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: append <file> <texte>"); return; }
    let text = remainder_after_tokens(line, 2);
    unsafe {
        match FS.resolve(argv[1], cwd) {
            Some(idx) if FS.nodes[idx].kind == NodeKind::File => FS.append_node(idx, text),
            Some(_) => println!("append: dossier"),
            None => println!("append: fichier introuvable"),
        }
    }
}

fn cmd_nano(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: nano <file>"); return; }
    println!("nano minimal: ecris une ligne puis Entree");
    print!("> ");
    let mut buf = [0u8; 256];
    let len = read_line(&mut buf);
    let text = unsafe { core::str::from_utf8_unchecked(&buf[..len]) };
    unsafe {
        let idx = match FS.resolve(argv[1], cwd) {
            Some(idx) => idx,
            None => match FS.resolve_parent_name(argv[1], cwd) {
                Some((parent, name)) => match FS.touch_at(parent, name) {
                    Ok(idx) => idx,
                    Err(e) => { println!("nano: {}", e); return; }
                },
                None => { println!("nano: chemin invalide"); return; }
            }
        };
        if FS.nodes[idx].kind == NodeKind::File { FS.write_node(idx, text); }
    }
}

fn cmd_rm(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: rm <file>"); return; }
    unsafe {
        match FS.resolve(argv[1], cwd) {
            Some(idx) if idx != 0 && FS.nodes[idx].kind == NodeKind::File => FS.nodes[idx].used = false,
            Some(_) => println!("rm: pas un fichier"),
            None => println!("rm: introuvable"),
        }
    }
}

fn cmd_rmdir(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: rmdir <dir>"); return; }
    unsafe {
        match FS.resolve(argv[1], cwd) {
            Some(idx) if idx != 0 && FS.nodes[idx].kind == NodeKind::Dir && FS.is_empty_dir(idx) => FS.nodes[idx].used = false,
            Some(idx) if FS.nodes[idx].kind == NodeKind::Dir => println!("rmdir: dossier non vide"),
            Some(_) => println!("rmdir: pas un dossier"),
            None => println!("rmdir: introuvable"),
        }
    }
}

fn cmd_cp(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: cp <src> <dst>"); return; }
    unsafe {
        let src = match FS.resolve(argv[1], cwd) {
            Some(idx) if FS.nodes[idx].kind == NodeKind::File => idx,
            _ => { println!("cp: source invalide"); return; }
        };
        let (parent, name) = match FS.resolve_parent_name(argv[2], cwd) {
            Some(v) => v,
            None => { println!("cp: destination invalide"); return; }
        };
        let dst = match FS.touch_at(parent, name) {
            Ok(idx) => idx,
            Err(e) => { println!("cp: {}", e); return; }
        };
        let len = FS.nodes[src].content_len;
        for i in 0..len { FS.nodes[dst].content[i] = FS.nodes[src].content[i]; }
        FS.nodes[dst].content_len = len;
    }
}

fn cmd_mv(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: mv <src> <dst>"); return; }
    unsafe {
        let src = match FS.resolve(argv[1], cwd) {
            Some(idx) if idx != 0 => idx,
            _ => { println!("mv: source invalide"); return; }
        };
        let (parent, name) = match FS.resolve_parent_name(argv[2], cwd) {
            Some(v) => v,
            None => { println!("mv: destination invalide"); return; }
        };
        if FS.find_child(parent, name).is_some() {
            println!("mv: destination existe deja");
            return;
        }
        FS.nodes[src].parent = parent;
        if !FS.nodes[src].set_name(name) { println!("mv: nom invalide"); }
    }
}

fn cmd_stat(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: stat <path>"); return; }
    unsafe {
        match FS.resolve(argv[1], cwd) {
            Some(idx) => {
                let n = &FS.nodes[idx];
                print!("path: "); print_path(&FS, idx); println!("");
                print!("type: "); println!("{}", if n.kind == NodeKind::Dir { "directory" } else { "file" });
                print!("mode: "); print_mode(n.kind, n.mode); println!("  octal={:o}", n.mode);
                println!("uid: {}", n.uid);
                println!("gid: {}", n.gid);
                println!("size: {}", n.content_len);
            }
            None => println!("stat: introuvable"),
        }
    }
}

fn parse_octal(s: &str) -> Option<u16> {
    let mut value: u16 = 0;
    if s.is_empty() { return None; }
    for b in s.bytes() {
        if b < b'0' || b > b'7' { return None; }
        value = value * 8 + (b - b'0') as u16;
    }
    Some(value)
}

fn cmd_chmod(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: chmod <mode-octal> <path>"); return; }
    let mode = match parse_octal(argv[1]) { Some(m) => m, None => { println!("chmod: mode invalide"); return; } };
    unsafe {
        match FS.resolve(argv[2], cwd) {
            Some(idx) => FS.nodes[idx].mode = mode,
            None => println!("chmod: introuvable"),
        }
    }
}

fn cmd_network_placeholder(cmd: &str) {
    println!("{}: pile reseau non activee dans V0.5", cmd);
    println!("roadmap OSI: PCI -> driver e1000/virtio-net -> Ethernet -> ARP -> IPv4 -> ICMP -> UDP -> DHCP/DNS -> TCP -> HTTP");
}
