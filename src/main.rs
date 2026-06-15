#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use core::arch::asm;
use core::panic::PanicInfo;
use core::ptr::write_volatile;
use core::str;

const VGA_BUFFER: usize = 0xb8000;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

const COLOR_GREEN: u8 = 0x0A;
const COLOR_RED: u8 = 0x0C;
const COLOR_WHITE: u8 = 0x0F;
const COLOR_CYAN: u8 = 0x0B;
const COLOR_YELLOW: u8 = 0x0E;

const LINE_MAX: usize = 128;
const MAX_NODES: usize = 32;
const NAME_MAX: usize = 16;
const CONTENT_MAX: usize = 256;

static mut CURSOR_ROW: usize = 0;
static mut CURSOR_COL: usize = 0;
static mut SHIFT: bool = false;

static mut LINE: [u8; LINE_MAX] = [0; LINE_MAX];
static mut LINE_LEN: usize = 0;

static mut CURRENT_DIR: usize = 0;
static mut FS: [Node; MAX_NODES] = [Node::empty(); MAX_NODES];
static mut INPUT_MODE: InputMode = InputMode::Shell;
static mut EDIT_NAME: [u8; NAME_MAX] = [0; NAME_MAX];
static mut EDIT_NAME_LEN: usize = 0;

#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Shell,
    Editor,
}

#[derive(Clone, Copy, PartialEq)]
enum NodeKind {
    Empty,
    File,
    Dir,
}

#[derive(Clone, Copy)]
struct Node {
    kind: NodeKind,
    parent: usize,
    name: [u8; NAME_MAX],
    name_len: usize,
    content: [u8; CONTENT_MAX],
    content_len: usize,
}

impl Node {
    const fn empty() -> Self {
        Self {
            kind: NodeKind::Empty,
            parent: 0,
            name: [0; NAME_MAX],
            name_len: 0,
            content: [0; CONTENT_MAX],
            content_len: 0,
        }
    }
}

#[derive(Clone, Copy)]
enum Key {
    Char(u8),
    Enter,
    Backspace,
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    clear_screen();
    fs_init();

    print_colored("Bouchaud OS\n", COLOR_GREEN);
    print("Version: 0.2.0 - shell + clavier + RAMFS\n");
    print("Commandes: help, ls, pwd, cd, mkdir, touch, write, cat, nano, clear\n\n");
    prompt();

    loop {
        if let Some(key) = keyboard_poll() {
            handle_key(key);
        }
        cpu_pause();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    print_colored("\n\n[KERNEL PANIC]\n", COLOR_RED);

    if let Some(location) = info.location() {
        print("Fichier: ");
        print(location.file());
        print("\nLigne: ");
        print_usize(location.line() as usize);
        print("\n");
    }

    loop {
        cpu_hlt();
    }
}

fn handle_key(key: Key) {
    match key {
        Key::Char(byte) => unsafe {
            if LINE_LEN < LINE_MAX - 1 {
                LINE[LINE_LEN] = byte;
                LINE_LEN += 1;
                put_byte(byte, COLOR_WHITE);
            }
        },
        Key::Backspace => unsafe {
            if LINE_LEN > 0 {
                LINE_LEN -= 1;
                screen_backspace();
            }
        },
        Key::Enter => {
            print("\n");
            let mut local = [0u8; LINE_MAX];
            let len = unsafe {
                let len = LINE_LEN;
                let mut i = 0;
                while i < len {
                    local[i] = LINE[i];
                    i += 1;
                }
                LINE_LEN = 0;
                len
            };

            let text = unsafe { str::from_utf8_unchecked(&local[..len]) };
            handle_line(text.trim());
        }
    }
}

fn handle_line(line: &str) {
    unsafe {
        if INPUT_MODE == InputMode::Editor {
            save_editor_line(line);
            INPUT_MODE = InputMode::Shell;
            prompt();
            return;
        }
    }

    if line.is_empty() {
        prompt();
        return;
    }

    let (cmd, rest) = split_first(line);

    match cmd {
        "help" => cmd_help(),
        "ls" => cmd_ls(),
        "pwd" => cmd_pwd(),
        "cd" => cmd_cd(rest),
        "mkdir" => cmd_mkdir(rest),
        "touch" => cmd_touch(rest),
        "write" => cmd_write(rest),
        "cat" => cmd_cat(rest),
        "nano" => cmd_nano(rest),
        "clear" => clear_screen(),
        _ => {
            print_colored("Commande inconnue: ", COLOR_RED);
            print(cmd);
            print("\n");
        }
    }

    prompt();
}

fn split_first(input: &str) -> (&str, &str) {
    let input = input.trim();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b' ' {
            let cmd = &input[..i];
            let rest = input[i + 1..].trim();
            return (cmd, rest);
        }
        i += 1;
    }
    (input, "")
}

fn cmd_help() {
    print_colored("Commandes disponibles:\n", COLOR_CYAN);
    print("  help                  affiche cette aide\n");
    print("  ls                    liste le dossier courant\n");
    print("  pwd                   affiche le chemin courant\n");
    print("  cd /                  retourne a la racine\n");
    print("  cd ..                 remonte d'un dossier\n");
    print("  cd <dir>              entre dans un dossier\n");
    print("  mkdir <dir>           cree un dossier en RAM\n");
    print("  touch <file>          cree un fichier vide\n");
    print("  write <file> <texte>  ecrit/remplace un fichier\n");
    print("  cat <file>            affiche un fichier\n");
    print("  nano <file>           mini-editeur: une ligne puis Entree\n");
    print("  clear                 nettoie l'ecran\n");
}

fn cmd_ls() {
    unsafe {
        let mut found = false;
        for i in 0..MAX_NODES {
            if FS[i].kind != NodeKind::Empty && i != 0 && FS[i].parent == CURRENT_DIR {
                found = true;
                match FS[i].kind {
                    NodeKind::Dir => {
                        print_colored("[DIR]  ", COLOR_CYAN);
                        print_node_name(i);
                        print("/\n");
                    }
                    NodeKind::File => {
                        print_colored("[FILE] ", COLOR_YELLOW);
                        print_node_name(i);
                        print("  ");
                        print_usize(FS[i].content_len);
                        print(" octets\n");
                    }
                    NodeKind::Empty => {}
                }
            }
        }
        if !found {
            print("dossier vide\n");
        }
    }
}

fn cmd_pwd() {
    unsafe {
        print_path(CURRENT_DIR);
        print("\n");
    }
}

fn cmd_cd(arg: &str) {
    if arg.is_empty() {
        print_colored("usage: cd <dir>\n", COLOR_RED);
        return;
    }

    unsafe {
        if arg == "/" {
            CURRENT_DIR = 0;
            return;
        }

        if arg == ".." {
            CURRENT_DIR = FS[CURRENT_DIR].parent;
            return;
        }

        if let Some(index) = find_child(CURRENT_DIR, arg) {
            if FS[index].kind == NodeKind::Dir {
                CURRENT_DIR = index;
            } else {
                print_colored("cd: ce n'est pas un dossier\n", COLOR_RED);
            }
        } else {
            print_colored("cd: dossier introuvable\n", COLOR_RED);
        }
    }
}

fn cmd_mkdir(name: &str) {
    if !valid_name(name) {
        print_colored("usage: mkdir <nom court>\n", COLOR_RED);
        return;
    }

    unsafe {
        if find_child(CURRENT_DIR, name).is_some() {
            print_colored("mkdir: existe deja\n", COLOR_RED);
            return;
        }

        match alloc_node() {
            Some(index) => {
                FS[index] = Node::empty();
                FS[index].kind = NodeKind::Dir;
                FS[index].parent = CURRENT_DIR;
                set_node_name(index, name);
            }
            None => print_colored("mkdir: plus de place dans le RAMFS\n", COLOR_RED),
        }
    }
}

fn cmd_touch(name: &str) {
    if !valid_name(name) {
        print_colored("usage: touch <nom court>\n", COLOR_RED);
        return;
    }

    unsafe {
        if find_child(CURRENT_DIR, name).is_some() {
            return;
        }
        create_file(name);
    }
}

fn cmd_write(rest: &str) {
    let (name, content) = split_first(rest);
    if !valid_name(name) {
        print_colored("usage: write <file> <texte>\n", COLOR_RED);
        return;
    }

    unsafe {
        let index = match find_child(CURRENT_DIR, name) {
            Some(index) => index,
            None => match create_file(name) {
                Some(index) => index,
                None => {
                    print_colored("write: plus de place dans le RAMFS\n", COLOR_RED);
                    return;
                }
            },
        };

        if FS[index].kind != NodeKind::File {
            print_colored("write: cible non fichier\n", COLOR_RED);
            return;
        }

        write_content(index, content);
    }
}

fn cmd_cat(name: &str) {
    if !valid_name(name) {
        print_colored("usage: cat <file>\n", COLOR_RED);
        return;
    }

    unsafe {
        if let Some(index) = find_child(CURRENT_DIR, name) {
            if FS[index].kind != NodeKind::File {
                print_colored("cat: ce n'est pas un fichier\n", COLOR_RED);
                return;
            }
            let mut i = 0;
            while i < FS[index].content_len {
                put_byte(FS[index].content[i], COLOR_WHITE);
                i += 1;
            }
            print("\n");
        } else {
            print_colored("cat: fichier introuvable\n", COLOR_RED);
        }
    }
}

fn cmd_nano(name: &str) {
    if !valid_name(name) {
        print_colored("usage: nano <file>\n", COLOR_RED);
        return;
    }

    unsafe {
        EDIT_NAME = [0; NAME_MAX];
        EDIT_NAME_LEN = 0;
        for (i, byte) in name.bytes().enumerate() {
            if i >= NAME_MAX {
                break;
            }
            EDIT_NAME[i] = byte;
            EDIT_NAME_LEN += 1;
        }
        INPUT_MODE = InputMode::Editor;
    }

    print_colored("mini-nano ", COLOR_CYAN);
    print(name);
    print(": tape une ligne puis Entree pour sauvegarder\n");
    print_colored("nano> ", COLOR_CYAN);
}

fn save_editor_line(content: &str) {
    unsafe {
        let name = str::from_utf8_unchecked(&EDIT_NAME[..EDIT_NAME_LEN]);
        let index = match find_child(CURRENT_DIR, name) {
            Some(index) => index,
            None => match create_file(name) {
                Some(index) => index,
                None => {
                    print_colored("nano: plus de place dans le RAMFS\n", COLOR_RED);
                    return;
                }
            },
        };

        if FS[index].kind != NodeKind::File {
            print_colored("nano: cible non fichier\n", COLOR_RED);
            return;
        }

        write_content(index, content);
        print_colored("[sauvegarde OK]\n", COLOR_GREEN);
    }
}

fn prompt() {
    print_colored("bouchaud-os:", COLOR_GREEN);
    unsafe { print_path(CURRENT_DIR); }
    print_colored("$ ", COLOR_GREEN);
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() < NAME_MAX && !name.as_bytes().contains(&b'/')
}

fn fs_init() {
    unsafe {
        FS = [Node::empty(); MAX_NODES];
        FS[0].kind = NodeKind::Dir;
        FS[0].parent = 0;
        FS[0].name[0] = b'/';
        FS[0].name_len = 1;
        CURRENT_DIR = 0;

        create_file("readme.txt");
        if let Some(index) = find_child(0, "readme.txt") {
            write_content(index, "Bienvenue dans Bouchaud OS v0.2");
        }
    }
}

unsafe fn create_file(name: &str) -> Option<usize> {
    match alloc_node() {
        Some(index) => {
            FS[index] = Node::empty();
            FS[index].kind = NodeKind::File;
            FS[index].parent = CURRENT_DIR;
            set_node_name(index, name);
            Some(index)
        }
        None => None,
    }
}

unsafe fn alloc_node() -> Option<usize> {
    for i in 1..MAX_NODES {
        if FS[i].kind == NodeKind::Empty {
            return Some(i);
        }
    }
    None
}

unsafe fn find_child(parent: usize, name: &str) -> Option<usize> {
    for i in 0..MAX_NODES {
        if FS[i].kind != NodeKind::Empty && FS[i].parent == parent && node_name_eq(i, name) {
            return Some(i);
        }
    }
    None
}

unsafe fn node_name_eq(index: usize, name: &str) -> bool {
    let bytes = name.as_bytes();
    if FS[index].name_len != bytes.len() {
        return false;
    }
    let mut i = 0;
    while i < bytes.len() {
        if FS[index].name[i] != bytes[i] {
            return false;
        }
        i += 1;
    }
    true
}

unsafe fn set_node_name(index: usize, name: &str) {
    FS[index].name = [0; NAME_MAX];
    FS[index].name_len = 0;
    for (i, byte) in name.bytes().enumerate() {
        if i >= NAME_MAX {
            break;
        }
        FS[index].name[i] = byte;
        FS[index].name_len += 1;
    }
}

unsafe fn write_content(index: usize, content: &str) {
    FS[index].content = [0; CONTENT_MAX];
    FS[index].content_len = 0;
    for (i, byte) in content.bytes().enumerate() {
        if i >= CONTENT_MAX {
            break;
        }
        FS[index].content[i] = byte;
        FS[index].content_len += 1;
    }
}

unsafe fn print_node_name(index: usize) {
    let mut i = 0;
    while i < FS[index].name_len {
        put_byte(FS[index].name[i], COLOR_WHITE);
        i += 1;
    }
}

unsafe fn print_path(mut index: usize) {
    if index == 0 {
        print("/");
        return;
    }

    let mut stack = [0usize; 16];
    let mut count = 0;
    while index != 0 && count < stack.len() {
        stack[count] = index;
        count += 1;
        index = FS[index].parent;
    }

    while count > 0 {
        count -= 1;
        print("/");
        print_node_name(stack[count]);
    }
}

fn keyboard_poll() -> Option<Key> {
    unsafe {
        if inb(0x64) & 1 == 0 {
            return None;
        }

        let sc = inb(0x60);

        match sc {
            0x2A | 0x36 => {
                SHIFT = true;
                None
            }
            0xAA | 0xB6 => {
                SHIFT = false;
                None
            }
            0x1C => Some(Key::Enter),
            0x0E => Some(Key::Backspace),
            code if code & 0x80 != 0 => None,
            code => scancode_to_ascii(code, SHIFT).map(Key::Char),
        }
    }
}

fn scancode_to_ascii(sc: u8, shift: bool) -> Option<u8> {
    let c = match sc {
        0x02 => if shift { b'!' } else { b'1' },
        0x03 => if shift { b'@' } else { b'2' },
        0x04 => if shift { b'#' } else { b'3' },
        0x05 => if shift { b'$' } else { b'4' },
        0x06 => if shift { b'%' } else { b'5' },
        0x07 => if shift { b'^' } else { b'6' },
        0x08 => if shift { b'&' } else { b'7' },
        0x09 => if shift { b'*' } else { b'8' },
        0x0A => if shift { b'(' } else { b'9' },
        0x0B => if shift { b')' } else { b'0' },
        0x0C => if shift { b'_' } else { b'-' },
        0x0D => if shift { b'+' } else { b'=' },
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
        0x1A => if shift { b'{' } else { b'[' },
        0x1B => if shift { b'}' } else { b']' },
        0x1E => letter(b'a', shift),
        0x1F => letter(b's', shift),
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
        0x2B => if shift { b'|' } else { b'\\' },
        0x2C => letter(b'z', shift),
        0x2D => letter(b'x', shift),
        0x2E => letter(b'c', shift),
        0x2F => letter(b'v', shift),
        0x30 => letter(b'b', shift),
        0x31 => letter(b'n', shift),
        0x32 => letter(b'm', shift),
        0x33 => if shift { b'<' } else { b',' },
        0x34 => if shift { b'>' } else { b'.' },
        0x35 => if shift { b'?' } else { b'/' },
        0x39 => b' ',
        _ => return None,
    };
    Some(c)
}

fn letter(c: u8, shift: bool) -> u8 {
    if shift { c - 32 } else { c }
}

fn clear_screen() {
    for row in 0..VGA_HEIGHT {
        for col in 0..VGA_WIDTH {
            write_cell(row, col, b' ', COLOR_WHITE);
        }
    }
    unsafe {
        CURSOR_ROW = 0;
        CURSOR_COL = 0;
    }
}

fn print(text: &str) {
    print_colored(text, COLOR_WHITE);
}

fn print_colored(text: &str, color: u8) {
    for byte in text.bytes() {
        put_byte(byte, color);
    }
}

fn print_usize(mut value: usize) {
    if value == 0 {
        put_byte(b'0', COLOR_WHITE);
        return;
    }

    let mut buffer = [0u8; 20];
    let mut i = 0;
    while value > 0 {
        buffer[i] = b'0' + (value % 10) as u8;
        value /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        put_byte(buffer[i], COLOR_WHITE);
    }
}

fn put_byte(byte: u8, color: u8) {
    unsafe {
        match byte {
            b'\n' => new_line(),
            byte => {
                if CURSOR_COL >= VGA_WIDTH {
                    new_line();
                }
                write_cell(CURSOR_ROW, CURSOR_COL, byte, color);
                CURSOR_COL += 1;
            }
        }
    }
}

unsafe fn screen_backspace() {
    if CURSOR_COL > 0 {
        CURSOR_COL -= 1;
        write_cell(CURSOR_ROW, CURSOR_COL, b' ', COLOR_WHITE);
    }
}

unsafe fn new_line() {
    CURSOR_COL = 0;
    if CURSOR_ROW + 1 >= VGA_HEIGHT {
        scroll_up();
    } else {
        CURSOR_ROW += 1;
    }
}

unsafe fn scroll_up() {
    for row in 1..VGA_HEIGHT {
        for col in 0..VGA_WIDTH {
            let from = (row * VGA_WIDTH + col) * 2;
            let to = ((row - 1) * VGA_WIDTH + col) * 2;
            let char_byte = *((VGA_BUFFER + from) as *const u8);
            let color_byte = *((VGA_BUFFER + from + 1) as *const u8);
            write_volatile((VGA_BUFFER + to) as *mut u8, char_byte);
            write_volatile((VGA_BUFFER + to + 1) as *mut u8, color_byte);
        }
    }

    for col in 0..VGA_WIDTH {
        write_cell(VGA_HEIGHT - 1, col, b' ', COLOR_WHITE);
    }
    CURSOR_ROW = VGA_HEIGHT - 1;
}

fn write_cell(row: usize, col: usize, byte: u8, color: u8) {
    let offset = (row * VGA_WIDTH + col) * 2;
    unsafe {
        write_volatile((VGA_BUFFER + offset) as *mut u8, byte);
        write_volatile((VGA_BUFFER + offset + 1) as *mut u8, color);
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

#[inline(always)]
fn cpu_pause() {
    unsafe { asm!("pause", options(nomem, nostack, preserves_flags)); }
}

#[inline(always)]
fn cpu_hlt() {
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)); }
}
