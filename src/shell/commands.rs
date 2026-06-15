//! Implementation des commandes du shell Bouchaud OS.

use crate::arch::x86_64::{cpu, gdt, idt, interrupts};
use crate::drivers::keyboard;
use crate::drivers::vga::{self, COLOR_CYAN, COLOR_DEFAULT, COLOR_YELLOW};
use crate::fs::ramfs::{self, NodeKind, CONTENT_LEN, MAX_NODES};
use crate::kernel::timer;
use crate::net;
use crate::shell::history;
use crate::shell::remainder_after_tokens;
use crate::users;
use crate::{serial_println, OS_NAME, VERSION};

// ---------------------------------------------------------------------------
// Aide et informations
// ---------------------------------------------------------------------------

pub fn help() {
    vga::set_color(COLOR_CYAN);
    println!("Commandes Bouchaud OS V0.6:");
    vga::set_color(COLOR_DEFAULT);
    println!("  systeme : help, clear, version, uname, sysinfo, cpuinfo, meminfo, devices");
    println!("            dmesg, history, uptime, ticks, interrupts, serial-test, panic-test, roadmap");
    println!("  session : whoami, id, users, login <root|arthur|guest>, logout, su");
    println!("  fichiers: pwd, ls [-l] [path], tree [path], cd <path>, mkdir <path>");
    println!("            touch <file>, write <file> <texte>, append <file> <texte>, cat <file>");
    println!("            nano <file>, stat <path>, chmod <mode> <path>, cp <src> <dst>");
    println!("            mv <src> <dst>, rm <file>, rmdir <dir>, echo <texte>");
    println!("  reseau  : ifconfig, ip, route, arp, ping, dhcp, dns, wget, curl   [roadmap]");
    println!("  disque  : mount, df, sync, mkfs.bfs                                [roadmap]");
}

pub fn version() {
    println!("{} {} - kernel foundation", OS_NAME, VERSION);
    println!("Objectif: OS souverain francais experimental");
}

pub fn uname() {
    println!("Bouchaud OS {} x86_64 cli unix-like rust-no_std", VERSION);
}

pub fn sysinfo() {
    println!("os: {}", OS_NAME);
    println!("version: {} - kernel foundation", VERSION);
    println!("arch: x86_64");
    println!("keyboard: AZERTY-FR");
    println!("display: VGA text mode");
    println!("serial: COM1 debug {}", if crate::drivers::serial::is_ready() { "enabled" } else { "disabled" });
    println!("filesystem: RAMFS mounted on /");
    println!("gdt: {}", gdt::state());
    println!("idt: {}", idt::state());
    println!("interrupts: {}", interrupts::state());
    println!("security: sessions + permissions simples, no user/kernel split yet");
    println!("network: OSI stack planned, driver not enabled yet");
    println!("objectif: OS souverain francais experimental");
}

pub fn cpuinfo() {
    #[cfg(target_arch = "x86_64")]
    cpu::print_cpuinfo();
}

pub fn meminfo() {
    let fs = ramfs::fs();
    println!("memory model: static kernel memory + RAMFS fixed arrays");
    println!("ramfs inodes: used={} free={} total={}", fs.used_nodes(), fs.free_nodes(), MAX_NODES);
    println!("ramfs max file size: {} bytes", CONTENT_LEN);
    println!("heap allocator: not enabled yet");
    println!("paging/user isolation: roadmap V0.7+");
}

pub fn devices() {
    let serial_state = if crate::drivers::serial::is_ready() { "COM1 0x3F8 UART 16550, debug actif" } else { "non initialise" };
    println!("devices detected/configured:");
    println!("  cpu0      x86_64 via CPUID");
    println!("  vga0      legacy VGA text buffer 0xb8000");
    println!("  kbd0      PS/2 keyboard polling, AZERTY-FR mapping");
    println!("  serial0   {}", serial_state);
    println!("  ramfs0    in-memory filesystem mounted on /");
    println!("  pci0      planned");
    println!("  net0      planned: e1000/virtio-net");
    println!("  disk0     planned: virtio-blk/BFS persistent FS");
}

pub fn uptime() {
    if timer::timer_enabled() {
        println!("uptime: {} ticks", timer::ticks());
    } else {
        println!("uptime: timer interrupts not enabled yet");
        println!("  mesure brute (TSC): {} cycles depuis le boot", timer::cycles_since_boot());
    }
}

pub fn ticks() {
    println!("timer ticks: {}", timer::ticks());
    println!("tsc cycles since boot: {}", timer::cycles_since_boot());
    if !timer::timer_enabled() {
        println!("note: timer interrupts not enabled yet (compteur fige a 0)");
    }
}

pub fn interrupts() {
    println!("gdt: {}", gdt::state());
    println!("idt: {}", idt::state());
    println!("interrupts: {}", interrupts::state());
    println!("hardware IRQ: {}", if interrupts::enabled() { "enabled" } else { "disabled (polling clavier)" });
}

pub fn serial_test() {
    if !crate::drivers::serial::is_ready() {
        println!("serial-test: COM1 non initialise");
        return;
    }
    serial_println!("serial-test: message de test depuis Bouchaud OS V0.6 sur COM1");
    println!("serial-test: ecrit sur COM1 (visible dans le terminal QEMU via -serial stdio)");
}

pub fn panic_test() {
    if !users::session().is_root() {
        println!("panic-test: reserve a root (utilise 'su')");
        return;
    }
    vga::set_color(COLOR_YELLOW);
    println!("panic-test: declenchement volontaire d'une panique noyau...");
    vga::set_color(COLOR_DEFAULT);
    panic!("panic-test demande par l'utilisateur root");
}

pub fn roadmap() {
    vga::set_color(COLOR_CYAN);
    println!("Roadmap Bouchaud OS - OS souverain francais experimental");
    vga::set_color(COLOR_DEFAULT);
    println!("V0.6 (actuel): refactor modulaire, serie COM1, dmesg reel, stubs GDT/IDT, timer TSC");
    println!("V0.7: GDT/IDT reelles, exceptions CPU, PIC, IRQ timer + clavier");
    println!("V0.8: pagination + heap allocator (passage a alloc)");
    println!("V0.9: scan PCI + bus devices");
    println!("V1.0: pile reseau (driver e1000/virtio-net -> Ethernet -> IPv4 -> TCP)");
    println!("Plus tard: disque persistant BFS, processus, syscalls, securite, GUI");
    println!("");
    net::print_roadmap();
}

pub fn history(argc: usize, argv: &[&str; 12]) {
    if argc >= 2 && argv[1] == "clear" {
        history::clear();
        println!("history: efface");
        return;
    }
    history::print();
}

// ---------------------------------------------------------------------------
// Sessions / utilisateurs
// ---------------------------------------------------------------------------

pub fn id() {
    let s = users::session();
    println!("uid={}({}) gid={}({})", s.uid(), s.username(), s.gid(), s.username());
}

pub fn users() {
    println!("root:x:0:0:/root");
    println!("arthur:x:1000:1000:/home/arthur");
    println!("guest:x:65534:65534:/tmp");
}

pub fn login(argc: usize, argv: &[&str; 12]) {
    if argc < 2 {
        println!("usage: login <root|arthur|guest>");
        return;
    }
    match users::user_from_name(argv[1]) {
        Some(user) => {
            users::session().login(user);
            println!("session ouverte: {}", users::session().username());
        }
        None => println!("login: utilisateur inconnu"),
    }
}

// ---------------------------------------------------------------------------
// Fichiers
// ---------------------------------------------------------------------------

pub fn ls(argc: usize, argv: &[&str; 12], cwd: usize) {
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

    let fs = ramfs::fs();
    match fs.resolve(path, cwd) {
        Some(idx) => {
            if fs.nodes[idx].kind == NodeKind::File {
                ramfs::print_node_line(fs, idx, long);
            } else {
                for i in 0..MAX_NODES {
                    if fs.nodes[i].used && i != idx && fs.nodes[i].parent == idx {
                        ramfs::print_node_line(fs, i, long);
                    }
                }
            }
        }
        None => println!("ls: chemin introuvable"),
    }
}

pub fn tree(argc: usize, argv: &[&str; 12], cwd: usize) {
    let path = if argc >= 2 { argv[1] } else { "." };
    let fs = ramfs::fs();
    match fs.resolve(path, cwd) {
        Some(idx) => {
            ramfs::print_path(fs, idx);
            println!("");
            tree_rec(idx, 0);
        }
        None => println!("tree: chemin introuvable"),
    }
}

fn tree_rec(idx: usize, depth: usize) {
    let fs = ramfs::fs();
    if fs.nodes[idx].kind != NodeKind::Dir { return; }
    for i in 0..MAX_NODES {
        if fs.nodes[i].used && i != idx && fs.nodes[i].parent == idx {
            for _ in 0..depth { print!("  "); }
            if fs.nodes[i].kind == NodeKind::Dir {
                vga::set_color(COLOR_CYAN);
                println!("|- {}/", fs.nodes[i].name_str());
                vga::set_color(COLOR_DEFAULT);
                tree_rec(i, depth + 1);
            } else {
                println!("|- {}", fs.nodes[i].name_str());
            }
        }
    }
}

pub fn cd(argc: usize, argv: &[&str; 12], cwd: &mut usize) {
    if argc < 2 { *cwd = 0; return; }
    let fs = ramfs::fs();
    match fs.resolve(argv[1], *cwd) {
        Some(idx) if fs.nodes[idx].kind == NodeKind::Dir => *cwd = idx,
        Some(_) => println!("cd: pas un dossier"),
        None => println!("cd: chemin introuvable"),
    }
}

pub fn mkdir(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: mkdir <path>"); return; }
    let fs = ramfs::fs();
    match fs.resolve_parent_name(argv[1], cwd) {
        Some((parent, name)) => match fs.mkdir_at(parent, name) {
            Ok(_) => {}
            Err(e) => println!("mkdir: {}", e),
        },
        None => println!("mkdir: chemin invalide"),
    }
}

pub fn touch(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: touch <file>"); return; }
    let fs = ramfs::fs();
    match fs.resolve_parent_name(argv[1], cwd) {
        Some((parent, name)) => match fs.touch_at(parent, name) {
            Ok(_) => {}
            Err(e) => println!("touch: {}", e),
        },
        None => println!("touch: chemin invalide"),
    }
}

pub fn cat(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: cat <file>"); return; }
    let fs = ramfs::fs();
    match fs.resolve(argv[1], cwd) {
        Some(idx) if fs.nodes[idx].kind == NodeKind::File => {
            for i in 0..fs.nodes[idx].content_len {
                print!("{}", fs.nodes[idx].content[i] as char);
            }
            println!("");
        }
        Some(_) => println!("cat: dossier"),
        None => println!("cat: fichier introuvable"),
    }
}

pub fn write(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: write <file> <texte>"); return; }
    let text = remainder_after_tokens(line, 2);
    let fs = ramfs::fs();
    match fs.resolve(argv[1], cwd) {
        Some(idx) if fs.nodes[idx].kind == NodeKind::File => fs.write_node(idx, text),
        Some(_) => println!("write: dossier"),
        None => println!("write: fichier introuvable"),
    }
}

pub fn append(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: append <file> <texte>"); return; }
    let text = remainder_after_tokens(line, 2);
    let fs = ramfs::fs();
    match fs.resolve(argv[1], cwd) {
        Some(idx) if fs.nodes[idx].kind == NodeKind::File => fs.append_node(idx, text),
        Some(_) => println!("append: dossier"),
        None => println!("append: fichier introuvable"),
    }
}

pub fn nano(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: nano <file>"); return; }
    println!("nano minimal: ecris une ligne puis Entree");
    print!("> ");
    let mut buf = [0u8; 256];
    let len = keyboard::read_line(&mut buf);
    let text = unsafe { core::str::from_utf8_unchecked(&buf[..len]) };
    let fs = ramfs::fs();
    let idx = match fs.resolve(argv[1], cwd) {
        Some(idx) => idx,
        None => match fs.resolve_parent_name(argv[1], cwd) {
            Some((parent, name)) => match fs.touch_at(parent, name) {
                Ok(idx) => idx,
                Err(e) => { println!("nano: {}", e); return; }
            },
            None => { println!("nano: chemin invalide"); return; }
        },
    };
    if fs.nodes[idx].kind == NodeKind::File { fs.write_node(idx, text); }
}

pub fn rm(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: rm <file>"); return; }
    let fs = ramfs::fs();
    match fs.resolve(argv[1], cwd) {
        Some(idx) if idx != 0 && fs.nodes[idx].kind == NodeKind::File => fs.nodes[idx].used = false,
        Some(_) => println!("rm: pas un fichier"),
        None => println!("rm: introuvable"),
    }
}

pub fn rmdir(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: rmdir <dir>"); return; }
    let fs = ramfs::fs();
    match fs.resolve(argv[1], cwd) {
        Some(idx) if idx != 0 && fs.nodes[idx].kind == NodeKind::Dir && fs.is_empty_dir(idx) => fs.nodes[idx].used = false,
        Some(idx) if fs.nodes[idx].kind == NodeKind::Dir => println!("rmdir: dossier non vide"),
        Some(_) => println!("rmdir: pas un dossier"),
        None => println!("rmdir: introuvable"),
    }
}

pub fn cp(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: cp <src> <dst>"); return; }
    let fs = ramfs::fs();
    let src = match fs.resolve(argv[1], cwd) {
        Some(idx) if fs.nodes[idx].kind == NodeKind::File => idx,
        _ => { println!("cp: source invalide"); return; }
    };
    let (parent, name) = match fs.resolve_parent_name(argv[2], cwd) {
        Some(v) => v,
        None => { println!("cp: destination invalide"); return; }
    };
    let dst = match fs.touch_at(parent, name) {
        Ok(idx) => idx,
        Err(e) => { println!("cp: {}", e); return; }
    };
    let len = fs.nodes[src].content_len;
    for i in 0..len { fs.nodes[dst].content[i] = fs.nodes[src].content[i]; }
    fs.nodes[dst].content_len = len;
}

pub fn mv(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: mv <src> <dst>"); return; }
    let fs = ramfs::fs();
    let src = match fs.resolve(argv[1], cwd) {
        Some(idx) if idx != 0 => idx,
        _ => { println!("mv: source invalide"); return; }
    };
    let (parent, name) = match fs.resolve_parent_name(argv[2], cwd) {
        Some(v) => v,
        None => { println!("mv: destination invalide"); return; }
    };
    if fs.find_child(parent, name).is_some() {
        println!("mv: destination existe deja");
        return;
    }
    fs.nodes[src].parent = parent;
    if !fs.nodes[src].set_name(name) { println!("mv: nom invalide"); }
}

pub fn stat(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: stat <path>"); return; }
    let fs = ramfs::fs();
    match fs.resolve(argv[1], cwd) {
        Some(idx) => {
            let n = &fs.nodes[idx];
            print!("path: "); ramfs::print_path(fs, idx); println!("");
            print!("type: "); println!("{}", if n.kind == NodeKind::Dir { "directory" } else { "file" });
            print!("mode: "); ramfs::print_mode(n.kind, n.mode); println!("  octal={:o}", n.mode);
            println!("uid: {}", n.uid);
            println!("gid: {}", n.gid);
            println!("size: {}", n.content_len);
        }
        None => println!("stat: introuvable"),
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

pub fn chmod(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: chmod <mode-octal> <path>"); return; }
    let mode = match parse_octal(argv[1]) { Some(m) => m, None => { println!("chmod: mode invalide"); return; } };
    let fs = ramfs::fs();
    match fs.resolve(argv[2], cwd) {
        Some(idx) => fs.nodes[idx].mode = mode,
        None => println!("chmod: introuvable"),
    }
}

// ---------------------------------------------------------------------------
// Disque (placeholders, roadmap BFS)
// ---------------------------------------------------------------------------

pub fn disk_placeholder(cmd: &str) {
    vga::set_color(COLOR_YELLOW);
    println!("{}: stockage persistant non active dans V0.6", cmd);
    vga::set_color(COLOR_DEFAULT);
    println!("  actuel: RAMFS volatil monte sur /");
    println!("  roadmap: block device -> virtio-blk -> BFS (Bouchaud File System) persistant");
}
