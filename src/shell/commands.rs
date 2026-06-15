//! Implementation des commandes du shell Bouchaud OS.

use crate::arch::x86_64::{cpu, gdt, idt, interrupts};
use crate::drivers::keyboard;
use crate::drivers::vga::{self, COLOR_CYAN, COLOR_DEFAULT, COLOR_YELLOW};
use crate::fs::ramfs::{self, NodeKind, CONTENT_LEN, MAX_NODES, PERM_R, PERM_W, PERM_X};
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
    println!("Commandes Bouchaud OS {}:", VERSION);
    vga::set_color(COLOR_DEFAULT);
    println!("  systeme : help, clear, version, uname, sysinfo, cpuinfo, meminfo, devices");
    println!("            dmesg, history, uptime, ticks, interrupts, breakpoint, serial-test");
    println!("            panic-test, roadmap");
    println!("  session : whoami, id, users, su [user], logout/exit");
    println!("  comptes : useradd <nom>, userdel <nom>, passwd [user]   (root pour add/del)");
    println!("  fichiers: pwd, ls [-l] [path], tree [path], cd <path>, mkdir <path>");
    println!("            touch <file>, write <file> <texte>, append <file> <texte>, cat <file>");
    println!("            nano <file>, stat <path>, chmod <octal|+x|u+w> <path>, chown <user> <path>");
    println!("            cp <src> <dst>, mv <src> <dst>, rm <file>, rmdir <dir>, echo <texte>");
    println!("  materiel: lspci");
    println!("  reseau  : ping <ip> (loopback actif), ifconfig, ip, route, arp");
    println!("            dhcp, dns, wget, curl   [en attente du driver NIC]");
    println!("  disque  : mount, df, sync, mkfs.bfs                                [roadmap]");
    vga::set_color(COLOR_CYAN);
    println!("  shell   : cmd1 ; cmd2   cmd1 && cmd2   cmd1 || cmd2   cmd > f   cmd >> f");
    println!("            fleches haut/bas = historique, Tab = completion, $? = code retour");
    vga::set_color(COLOR_DEFAULT);
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
    println!("security: sessions + mot de passe + permissions Unix (rwx, uid/gid)");
    println!("pci: {} peripheriques (lspci)", crate::arch::x86_64::pci::count());
    println!("network: loopback lo actif (ping 127.0.0.1); eth0 en attente du driver NIC");
    println!("objectif: OS souverain francais experimental");
}

pub fn cpuinfo() {
    #[cfg(target_arch = "x86_64")]
    cpu::print_cpuinfo();
}

pub fn meminfo() {
    let fs = ramfs::fs();
    let (used, free, total) = crate::kernel::heap::stats();
    println!("memory model: static kernel memory + heap (alloc) + RAMFS");
    println!("heap: used={} o, free={} o, total={} o", used, free, total);
    println!("ramfs inodes: used={} free={} total={}", fs.used_nodes(), fs.free_nodes(), MAX_NODES);
    println!("ramfs max file size: {} bytes", CONTENT_LEN);
    println!("paging/user isolation: roadmap (tas statique pour l'instant)");
}

pub fn alloctest() {
    use alloc::vec::Vec;
    use alloc::string::String;
    let (u0, _, _) = crate::kernel::heap::stats();
    let mut v: Vec<u64> = Vec::new();
    for i in 0..1000u64 { v.push(i * i); }
    let sum: u64 = v.iter().sum();
    let mut s = String::new();
    for i in 0..5 { s.push_str("bouchaud "); let _ = i; }
    let (u1, free, _) = crate::kernel::heap::stats();
    println!("alloctest: Vec<u64> de {} elements, somme des carres = {}", v.len(), sum);
    println!("alloctest: String = \"{}\" (len {})", s.trim(), s.len());
    println!("alloctest: heap avant={} o, pendant={} o, libre={} o", u0, u1, free);
    println!("alloctest: OK (alloc fonctionne)");
}

pub fn devices() {
    let serial_state = if crate::drivers::serial::is_ready() { "COM1 0x3F8 UART 16550, debug actif" } else { "non initialise" };
    println!("devices detected/configured:");
    println!("  cpu0      x86_64 via CPUID");
    println!("  vga0      legacy VGA text buffer 0xb8000");
    println!("  kbd0      PS/2 keyboard polling, AZERTY-FR mapping");
    println!("  serial0   {}", serial_state);
    println!("  ramfs0    in-memory filesystem mounted on /");
    println!("  pci0      bus scanne ({} peripheriques) - voir 'lspci'", crate::arch::x86_64::pci::count());
    match crate::arch::x86_64::pci::find_network() {
        Some(d) => println!("  net0      carte PCI {:04x}:{:04x} detectee, driver non charge", d.vendor, d.device),
        None => println!("  net0      aucune carte reseau PCI detectee"),
    }
    println!("  disk0     planned: virtio-blk/BFS persistent FS");
}

pub fn uptime() {
    if timer::timer_enabled() {
        println!("uptime: {} s ({} ticks @ ~{} Hz)", timer::seconds(), timer::ticks(), timer::TICKS_PER_SECOND);
    } else {
        println!("uptime: timer interrupts not enabled yet");
        println!("  mesure brute (TSC): {} cycles depuis le boot", timer::cycles_since_boot());
    }
}

pub fn ticks() {
    println!("timer ticks: {}", timer::ticks());
    println!("uptime approx: {} s", timer::seconds());
    println!("tsc cycles since boot: {}", timer::cycles_since_boot());
    if !timer::timer_enabled() {
        println!("note: timer interrupts not enabled yet (compteur fige a 0)");
    }
}

pub fn breakpoint() {
    println!("breakpoint: declenchement d'une exception int3...");
    crate::arch::x86_64::idt::trigger_breakpoint();
    println!("breakpoint: reprise apres l'exception (handler OK)");
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
    println!("[x] V0.6 refactor modulaire, serie COM1, dmesg, timer, history");
    println!("[x] V0.6.1 permissions Unix + login mot de passe + scan PCI");
    println!("[x] V0.7 GDT/IDT, exceptions CPU, PIC, IRQ timer + clavier");
    println!("[x] V0.8 pile reseau: Ethernet/ARP/IPv4/ICMP + loopback (ping lo)");
    println!("[ ] driver NIC e1000/virtio-net (RX/TX DMA) -> Internet externe");
    println!("[ ] pagination + heap allocator (passage a alloc)");
    println!("[ ] UDP/DHCP/DNS puis TCP/HTTP/TLS");
    println!("[ ] disque persistant BFS, processus, syscalls, GUI");
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
    users::list();
}

/// Lit un mot de passe au clavier (saisie masquee) dans `buf`, renvoie le slice.
fn read_pass<'a>(prompt: &str, buf: &'a mut [u8]) -> &'a str {
    print!("{}", prompt);
    let len = keyboard::read_secret(buf);
    println!("");
    unsafe { core::str::from_utf8_unchecked(&buf[..len]) }
}

/// `su [user]` : change d'utilisateur dans la session courante (avec mot de passe).
pub fn su(argc: usize, argv: &[&str; 12], cwd: &mut usize) {
    let target = if argc >= 2 { argv[1] } else { "root" };
    let mut buf = [0u8; 64];
    let pass = read_pass("Mot de passe: ", &mut buf);
    match users::authenticate(target, pass) {
        Some(uid) => {
            users::session().set_uid(uid);
            *cwd = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
            println!("session: {}", users::session().username());
        }
        None => {
            vga::set_color(COLOR_YELLOW);
            println!("su: authentification echouee");
            vga::set_color(COLOR_DEFAULT);
        }
    }
}

/// `useradd <nom>` (root) : cree un utilisateur, demande son mot de passe.
pub fn useradd(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: useradd <nom>"); return; }
    if !users::session().is_root() { println!("useradd: reserve a root"); return; }
    let mut b1 = [0u8; 64];
    let mut b2 = [0u8; 64];
    let p1 = read_pass("Nouveau mot de passe: ", &mut b1);
    // Copie locale car le second appel reutilise le meme type de tampon.
    let mut p1buf = [0u8; 64];
    let p1len = p1.len().min(64);
    p1buf[..p1len].copy_from_slice(&p1.as_bytes()[..p1len]);
    let p2 = read_pass("Confirmer: ", &mut b2);
    if &p1buf[..p1len] != p2.as_bytes() {
        println!("useradd: les mots de passe different");
        return;
    }
    let pass = unsafe { core::str::from_utf8_unchecked(&p1buf[..p1len]) };
    match users::add_user(argv[1], pass) {
        Ok(uid) => {
            users::create_home_dirs();
            println!("useradd: {} cree (uid={})", argv[1], uid);
        }
        Err(e) => println!("useradd: {}", e),
    }
}

/// `userdel <nom>` (root) : supprime un utilisateur.
pub fn userdel(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: userdel <nom>"); return; }
    if !users::session().is_root() { println!("userdel: reserve a root"); return; }
    match users::remove_user(argv[1]) {
        Ok(()) => println!("userdel: {} supprime", argv[1]),
        Err(e) => println!("userdel: {}", e),
    }
}

/// `passwd [user]` : change un mot de passe (soi-meme, ou tout compte si root).
pub fn passwd(argc: usize, argv: &[&str; 12]) {
    let target = if argc >= 2 { argv[1] } else { users::session().username() };
    if argc >= 2 && argv[1] != users::session().username() && !users::session().is_root() {
        println!("passwd: seul root peut changer le mot de passe d'un autre compte");
        return;
    }
    let mut buf = [0u8; 64];
    let pass = read_pass("Nouveau mot de passe: ", &mut buf);
    match users::set_password(target, pass) {
        Ok(()) => println!("passwd: mot de passe mis a jour pour {}", target),
        Err(e) => println!("passwd: {}", e),
    }
}

// ---------------------------------------------------------------------------
// Fichiers
// ---------------------------------------------------------------------------

pub fn ls(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
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
    let idx = match fs.resolve_checked(path, cwd) {
        Ok(i) => i,
        Err(e) => { println!("ls: {}", e); return 1; }
    };
    if fs.nodes[idx].kind == NodeKind::File {
        ramfs::print_node_line(fs, idx, long);
    } else {
        // Lister un repertoire demande le droit de lecture sur celui-ci.
        if !fs.can(idx, PERM_R) {
            println!("ls: permission denied");
            return 1;
        }
        for i in 0..MAX_NODES {
            if fs.nodes[i].used && i != idx && fs.nodes[i].parent == idx {
                ramfs::print_node_line(fs, i, long);
            }
        }
    }
    0
}

pub fn tree(argc: usize, argv: &[&str; 12], cwd: usize) {
    let path = if argc >= 2 { argv[1] } else { "." };
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(path, cwd) {
        Ok(i) => i,
        Err(e) => { println!("tree: {}", e); return; }
    };
    ramfs::print_path(fs, idx);
    println!("");
    tree_rec(idx, 0);
}

fn tree_rec(idx: usize, depth: usize) {
    let fs = ramfs::fs();
    if fs.nodes[idx].kind != NodeKind::Dir { return; }
    // On n'explore un repertoire que si on a le droit de le lire.
    if !fs.can(idx, PERM_R) {
        for _ in 0..depth { print!("  "); }
        println!("|- [permission denied]");
        return;
    }
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

pub fn cd(argc: usize, argv: &[&str; 12], cwd: &mut usize) -> i32 {
    if argc < 2 { *cwd = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0); return 0; }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], *cwd) {
        Ok(i) => i,
        Err(e) => { println!("cd: {}", e); return 1; }
    };
    if fs.nodes[idx].kind != NodeKind::Dir {
        println!("cd: pas un dossier");
        return 1;
    }
    // Entrer dans un repertoire demande le droit d'execution dessus.
    if !fs.can(idx, PERM_X) {
        println!("cd: permission denied");
        return 1;
    }
    *cwd = idx;
    0
}

pub fn mkdir(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 { println!("usage: mkdir <path>"); return 1; }
    let fs = ramfs::fs();
    let (parent, name) = match fs.resolve_parent_name_checked(argv[1], cwd) {
        Ok(v) => v,
        Err(e) => { println!("mkdir: {}", e); return 1; }
    };
    if !fs.can(parent, PERM_W) { println!("mkdir: permission denied"); return 1; }
    if let Err(e) = fs.mkdir_at(parent, name) { println!("mkdir: {}", e); return 1; }
    0
}

pub fn touch(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 { println!("usage: touch <file>"); return 1; }
    let fs = ramfs::fs();
    let (parent, name) = match fs.resolve_parent_name_checked(argv[1], cwd) {
        Ok(v) => v,
        Err(e) => { println!("touch: {}", e); return 1; }
    };
    if !fs.can(parent, PERM_W) { println!("touch: permission denied"); return 1; }
    if let Err(e) = fs.touch_at(parent, name) { println!("touch: {}", e); return 1; }
    0
}

pub fn cat(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 { println!("usage: cat <file>"); return 1; }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => { println!("cat: {}", e); return 1; }
    };
    if fs.nodes[idx].kind != NodeKind::File { println!("cat: dossier"); return 1; }
    if !fs.can(idx, PERM_R) { println!("cat: permission denied"); return 1; }
    for i in 0..fs.nodes[idx].content_len {
        print!("{}", fs.nodes[idx].content[i] as char);
    }
    println!("");
    0
}

pub fn write(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: write <file> <texte>"); return; }
    let text = remainder_after_tokens(line, 2);
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => { println!("write: {}", e); return; }
    };
    if fs.nodes[idx].kind != NodeKind::File { println!("write: dossier"); return; }
    if !fs.can(idx, PERM_W) { println!("write: permission denied"); return; }
    fs.write_node(idx, text);
}

pub fn append(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 { println!("usage: append <file> <texte>"); return; }
    let text = remainder_after_tokens(line, 2);
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => { println!("append: {}", e); return; }
    };
    if fs.nodes[idx].kind != NodeKind::File { println!("append: dossier"); return; }
    if !fs.can(idx, PERM_W) { println!("append: permission denied"); return; }
    fs.append_node(idx, text);
}

pub fn nano(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 { println!("usage: nano <file>"); return; }
    println!("nano minimal: ecris une ligne puis Entree");
    print!("> ");
    let mut buf = [0u8; 256];
    let len = keyboard::read_line(&mut buf);
    let text = unsafe { core::str::from_utf8_unchecked(&buf[..len]) };
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(idx) => idx,
        Err("introuvable") => {
            // Le fichier n'existe pas : on tente de le creer dans son parent.
            let (parent, name) = match fs.resolve_parent_name_checked(argv[1], cwd) {
                Ok(v) => v,
                Err(e) => { println!("nano: {}", e); return; }
            };
            if !fs.can(parent, PERM_W) { println!("nano: permission denied"); return; }
            match fs.touch_at(parent, name) {
                Ok(idx) => idx,
                Err(e) => { println!("nano: {}", e); return; }
            }
        }
        Err(e) => { println!("nano: {}", e); return; }
    };
    if fs.nodes[idx].kind != NodeKind::File { println!("nano: pas un fichier"); return; }
    if !fs.can(idx, PERM_W) { println!("nano: permission denied"); return; }
    fs.write_node(idx, text);
}

pub fn rm(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 { println!("usage: rm <file>"); return 1; }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => { println!("rm: {}", e); return 1; }
    };
    if idx == 0 || fs.nodes[idx].kind != NodeKind::File { println!("rm: pas un fichier"); return 1; }
    // Supprimer demande le droit d'ecriture sur le repertoire parent.
    if !fs.can(fs.nodes[idx].parent, PERM_W) { println!("rm: permission denied"); return 1; }
    fs.nodes[idx].used = false;
    0
}

pub fn rmdir(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 { println!("usage: rmdir <dir>"); return 1; }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => { println!("rmdir: {}", e); return 1; }
    };
    if idx == 0 || fs.nodes[idx].kind != NodeKind::Dir { println!("rmdir: pas un dossier"); return 1; }
    if !fs.is_empty_dir(idx) { println!("rmdir: dossier non vide"); return 1; }
    if !fs.can(fs.nodes[idx].parent, PERM_W) { println!("rmdir: permission denied"); return 1; }
    fs.nodes[idx].used = false;
    0
}

pub fn cp(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 { println!("usage: cp <src> <dst>"); return 1; }
    let fs = ramfs::fs();
    let src = match fs.resolve_checked(argv[1], cwd) {
        Ok(idx) if fs.nodes[idx].kind == NodeKind::File => idx,
        Ok(_) => { println!("cp: source invalide"); return 1; }
        Err(e) => { println!("cp: {}", e); return 1; }
    };
    if !fs.can(src, PERM_R) { println!("cp: permission denied (source)"); return 1; }
    let (parent, name) = match fs.resolve_parent_name_checked(argv[2], cwd) {
        Ok(v) => v,
        Err(e) => { println!("cp: {}", e); return 1; }
    };
    if !fs.can(parent, PERM_W) { println!("cp: permission denied (destination)"); return 1; }
    let dst = match fs.touch_at(parent, name) {
        Ok(idx) => idx,
        Err(e) => { println!("cp: {}", e); return 1; }
    };
    let len = fs.nodes[src].content_len;
    for i in 0..len { fs.nodes[dst].content[i] = fs.nodes[src].content[i]; }
    fs.nodes[dst].content_len = len;
    0
}

pub fn mv(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 { println!("usage: mv <src> <dst>"); return 1; }
    let fs = ramfs::fs();
    let src = match fs.resolve_checked(argv[1], cwd) {
        Ok(idx) if idx != 0 => idx,
        Ok(_) => { println!("mv: source invalide"); return 1; }
        Err(e) => { println!("mv: {}", e); return 1; }
    };
    if !fs.can(fs.nodes[src].parent, PERM_W) { println!("mv: permission denied (source)"); return 1; }
    let (parent, name) = match fs.resolve_parent_name_checked(argv[2], cwd) {
        Ok(v) => v,
        Err(e) => { println!("mv: {}", e); return 1; }
    };
    if !fs.can(parent, PERM_W) { println!("mv: permission denied (destination)"); return 1; }
    if fs.find_child(parent, name).is_some() {
        println!("mv: destination existe deja");
        return 1;
    }
    fs.nodes[src].parent = parent;
    if !fs.nodes[src].set_name(name) { println!("mv: nom invalide"); return 1; }
    0
}

pub fn stat(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 { println!("usage: stat <path>"); return 1; }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => { println!("stat: {}", e); return 1; }
    };
    let n = &fs.nodes[idx];
    print!("path: "); ramfs::print_path(fs, idx); println!("");
    print!("type: "); println!("{}", if n.kind == NodeKind::Dir { "directory" } else { "file" });
    print!("mode: "); ramfs::print_mode(n.kind, n.mode); println!("  octal={:o}", n.mode);
    println!("uid: {}", n.uid);
    println!("gid: {}", n.gid);
    println!("size: {}", n.content_len);
    0
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

/// Applique une expression symbolique de chmod (ex. "+x", "u+w", "go-r", "a=rx")
/// au mode courant. Renvoie le nouveau mode, ou None si la syntaxe est invalide.
fn apply_symbolic(mut mode: u16, spec: &str) -> Option<u16> {
    let bytes = spec.as_bytes();
    let mut i = 0;
    // Cibles : u(tilisateur) g(roupe) o(autres) a(tous).
    let mut who_u = false;
    let mut who_g = false;
    let mut who_o = false;
    while i < bytes.len() {
        match bytes[i] {
            b'u' => who_u = true,
            b'g' => who_g = true,
            b'o' => who_o = true,
            b'a' => { who_u = true; who_g = true; who_o = true; }
            _ => break,
        }
        i += 1;
    }
    if !who_u && !who_g && !who_o {
        // Aucune cible => 'a' par defaut (comme sous Unix).
        who_u = true; who_g = true; who_o = true;
    }
    if i >= bytes.len() { return None; }
    let op = bytes[i];
    if op != b'+' && op != b'-' && op != b'=' { return None; }
    i += 1;
    // Permissions demandees.
    let mut perm = 0u16;
    while i < bytes.len() {
        match bytes[i] {
            b'r' => perm |= 0o4,
            b'w' => perm |= 0o2,
            b'x' => perm |= 0o1,
            _ => return None,
        }
        i += 1;
    }
    // Masque sur les trois groupes selectionnes.
    let mut mask = 0u16;
    if who_u { mask |= perm << 6; }
    if who_g { mask |= perm << 3; }
    if who_o { mask |= perm; }
    match op {
        b'+' => mode |= mask,
        b'-' => mode &= !mask,
        b'=' => {
            // Remet a zero les groupes vises puis applique.
            let mut clear = 0u16;
            if who_u { clear |= 0o700; }
            if who_g { clear |= 0o070; }
            if who_o { clear |= 0o007; }
            mode = (mode & !clear) | mask;
        }
        _ => {}
    }
    Some(mode)
}

pub fn chmod(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 { println!("usage: chmod <octal|+x|u+w|go-r|...> <path>"); return 1; }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[2], cwd) {
        Ok(i) => i,
        Err(e) => { println!("chmod: {}", e); return 1; }
    };
    // Seul le proprietaire (ou root) peut changer les droits.
    let s = users::session();
    if !s.is_root() && s.uid() != fs.nodes[idx].uid {
        println!("chmod: operation non permise");
        return 1;
    }
    // Mode octal (ex. 755) ou symbolique (ex. +x, u+w, go-r, a=rx).
    let new_mode = match parse_octal(argv[1]) {
        Some(m) => m,
        None => match apply_symbolic(fs.nodes[idx].mode, argv[1]) {
            Some(m) => m,
            None => { println!("chmod: mode invalide"); return 1; }
        },
    };
    fs.nodes[idx].mode = new_mode;
    0
}

fn parse_u16(s: &str) -> Option<u16> {
    let mut value: u32 = 0;
    if s.is_empty() { return None; }
    for b in s.bytes() {
        if !b.is_ascii_digit() { return None; }
        value = value * 10 + (b - b'0') as u32;
        if value > 65535 { return None; }
    }
    Some(value as u16)
}

pub fn chown(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 { println!("usage: chown <uid|user> <path>"); return 1; }
    // Seul root peut changer le proprietaire (comme sous Linux).
    if !users::session().is_root() {
        println!("chown: operation reservee a root");
        return 1;
    }
    // L'utilisateur peut etre un nom connu ou un uid numerique.
    let new_uid = match users::uid_of_name(argv[1]) {
        Some(u) => u,
        None => match parse_u16(argv[1]) {
            Some(v) => v,
            None => { println!("chown: utilisateur/uid invalide"); return 1; }
        },
    };
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[2], cwd) {
        Ok(i) => i,
        Err(e) => { println!("chown: {}", e); return 1; }
    };
    fs.nodes[idx].uid = new_uid;
    fs.nodes[idx].gid = new_uid;
    0
}

/// Ecrit `data` dans le fichier `path` (cree si besoin), en mode ecriture ou
/// ajout. Utilise par les redirections `>` et `>>` du shell.
pub fn redirect(path: &str, data: &str, append: bool, cwd: usize) -> i32 {
    if path.is_empty() { println!("redirection: fichier cible manquant"); return 1; }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(path, cwd) {
        Ok(i) => i,
        Err("introuvable") => {
            let (parent, name) = match fs.resolve_parent_name_checked(path, cwd) {
                Ok(v) => v,
                Err(e) => { println!("redirection: {}", e); return 1; }
            };
            if !fs.can(parent, PERM_W) { println!("redirection: permission denied"); return 1; }
            match fs.touch_at(parent, name) {
                Ok(i) => i,
                Err(e) => { println!("redirection: {}", e); return 1; }
            }
        }
        Err(e) => { println!("redirection: {}", e); return 1; }
    };
    if fs.nodes[idx].kind != NodeKind::File { println!("redirection: pas un fichier"); return 1; }
    if !fs.can(idx, PERM_W) { println!("redirection: permission denied"); return 1; }
    if append {
        fs.append_node(idx, data);
    } else {
        fs.write_node(idx, data);
    }
    0
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
