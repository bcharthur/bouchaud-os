//! Implementation des commandes du shell Bouchaud OS.

use crate::arch::x86_64::{cpu, gdt, idt, interrupts};
use crate::drivers::keyboard;
use crate::drivers::vga::{self, COLOR_CYAN, COLOR_DEFAULT, COLOR_YELLOW};
use crate::fs::ramfs::{self, NodeKind, MAX_FILE_SIZE, MAX_NODES, PERM_R, PERM_W, PERM_X};
use crate::kernel::timer;
use crate::net;
use crate::shell::history;
use crate::shell::remainder_after_tokens;
use crate::users;
use crate::{serial_println, OS_NAME, VERSION};
use alloc::string::String;

// ---------------------------------------------------------------------------
// Aide et informations
// ---------------------------------------------------------------------------

pub fn help() {
    vga::set_color(COLOR_CYAN);
    println!("Commandes Bouchaud OS {}:", VERSION);
    vga::set_color(COLOR_DEFAULT);
    println!("  systeme : help, clear, version, uname, sysinfo, cpuinfo, meminfo, resstat, memtop, gpuinfo, devices");
    println!("            dmesg, history, uptime, ticks, interrupts, breakpoint, serial-test");
    println!("            panic-test, roadmap");
    println!("  noyau   : ps, kill <pid>, free, syscalls, apps, launch <app>, df");
    println!("  ring 3  : exec <elf64> [args], elfinfo <f>, usermode (autotest), tasks");
    println!("            poll-selftest (Ladybird M5 : pipe2 + clone + poll, 100% bare-metal)");
    println!("            vmstat (memoire virtuelle), strace on|off");
    println!("  session : whoami, id, users, su [user], logout/exit");
    println!("  comptes : useradd <nom>, userdel <nom>, passwd [user]   (root pour add/del)");
    println!("  fichiers: pwd, ls [-l] [path], tree [path], cd <path>, mkdir <path>");
    println!("            touch <file>, write <file> <texte>, append <file> <texte>, cat <file>");
    println!("            nano <file>, edit <file> (plein ecran), stat <path>, chmod <...> <path>, chown <user> <path>");
    println!("            cp <src> <dst>, mv <src> <dst>, rm <file>, rmdir <dir>, echo <texte>");
    println!("  texte   : grep <motif> [f], wc [f], head [-n N] [f], tail [-n N] [f], find [path]");
    println!("  env     : export NOM=val, env, unset NOM, $NOM, run <script.bsh>");
    println!("  divers  : date, js-selftest, wasm <f.wasm>, wasm-selftest, rustc <f.rs>");
    println!("  python  : python (REPL) | python <f.py> [args] | python -c \"code\"");
    println!("            pip install <paquet> (wheels pures PyPI), pip list");
    println!("  graphique: desktop (bureau VGA + souris, Echap pour quitter)");
    println!("  materiel: lspci");
    println!("  reseau  : ifup, ethinfo, arping <ip>, ping <ip>, ifconfig, ip, route, arp");
    println!("            dns <nom>, wget/http <url>, https <url> (TLS 1.3 reel), dhcp");
    println!("            tls [hote] (diagnostic + handshake), tls-selftest (vecteurs crypto)");
    println!("  disque  : mount, df, sync, mkfs.bfs                                [roadmap]");
    vga::set_color(COLOR_CYAN);
    println!("  shell   : cmd1 ; cmd2   &&   ||   cmd > f   cmd >> f   cmd1 | cmd2");
    println!("            fleches haut/bas = historique, Tab = completion, $? = code retour");
    println!("            clavier: | = AltGr+6 | < = AltGr+, | > = AltGr+; (ou touche ISO)");
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
    println!(
        "serial: COM1 debug {}",
        if crate::drivers::serial::is_ready() {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("filesystem: RAMFS mounted on /");
    println!("gdt: {}", gdt::state());
    println!("idt: {}", idt::state());
    println!("interrupts: {}", interrupts::state());
    println!("user-mode: {}", crate::arch::x86_64::usermode::state());
    println!("abi: Linux x86-64 (exec <elf64>, voir syscalls)");
    println!("security: sessions + mot de passe + permissions Unix (rwx, uid/gid)");
    println!(
        "pci: {} peripheriques (lspci)",
        crate::arch::x86_64::pci::count()
    );
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
    let (fu, ff, ft) = crate::kernel::vmm::frame_stats();
    println!("memory model: tas noyau + frames physiques 4 KiB + RAMFS");
    println!("heap: used={} o, free={} o, total={} o", used, free, total);
    println!(
        "frames user: used={} free={} total={} ({} MiB)",
        fu,
        ff,
        ft,
        ft * 4096 / (1024 * 1024)
    );
    println!(
        "ramfs inodes: used={} free={} total={}",
        fs.used_nodes(),
        fs.free_nodes(),
        MAX_NODES
    );
    println!("ramfs max file size: {} bytes", MAX_FILE_SIZE);
    println!(
        "pagination: une PML4 par processus, creneau user {:#x} (voir vmstat)",
        crate::kernel::vmm::user_slot_base()
    );
    let (lazy_files, lazy_bytes, disk_reads, disk_bytes) = crate::fs::backing::stats();
    let (zero_faults, file_faults) = crate::kernel::task::demand_fault_stats();
    println!(
        "memory-fabric: backing={} fichiers/{} Kio, disk-reads={} ({} Kio), faults zero={} file={}",
        lazy_files,
        lazy_bytes / 1024,
        disk_reads,
        disk_bytes / 1024,
        zero_faults,
        file_faults
    );
    if let Some(task) = crate::kernel::task::try_current() {
        let process = task.process.borrow();
        println!(
            "vma: {} regions, {} Mio virtuels",
            process.promesses.len(),
            crate::kernel::vma::octets_virtuels(&process.promesses)
                / (1024 * 1024)
        );
    }
    let pmm = crate::kernel::vmm::frame_stats_detailed();
    let dma = crate::kernel::memory::dma_stats();
    println!(
        "pmm-accounting: high={} alloc={} free_ops={} failures={}",
        pmm.high_watermark, pmm.allocations, pmm.frees, pmm.failures
    );
    println!(
        "dma-accounting: used={} KiB free={} KiB alloc={} failures={}",
        dma.used / 1024, dma.free / 1024, dma.allocations, dma.failures
    );
    println!(
        "vma-selftest: {}",
        if crate::kernel::vma::self_test() {
            "OK"
        } else {
            "ECHEC"
        }
    );
}

pub fn resstat() {
    crate::kernel::resource::print_system();
}

pub fn memtop() {
    crate::kernel::resource::print_processes();
}

pub fn gpuinfo() {
    crate::drivers::gpu::print_info();
}

pub fn resource_selftest() {
    let ok = crate::kernel::resource::self_test();
    println!(
        "[resource-selftest] {}",
        if ok { "OK" } else { "ECHEC" }
    );
}

pub fn alloctest() {
    use alloc::string::String;
    use alloc::vec::Vec;
    let (u0, _, _) = crate::kernel::heap::stats();
    let mut v: Vec<u64> = Vec::new();
    for i in 0..1000u64 {
        v.push(i * i);
    }
    let sum: u64 = v.iter().sum();
    let mut s = String::new();
    for i in 0..5 {
        s.push_str("bouchaud ");
        let _ = i;
    }
    let (u1, free, _) = crate::kernel::heap::stats();
    println!(
        "alloctest: Vec<u64> de {} elements, somme des carres = {}",
        v.len(),
        sum
    );
    println!("alloctest: String = \"{}\" (len {})", s.trim(), s.len());
    println!(
        "alloctest: heap avant={} o, pendant={} o, libre={} o",
        u0, u1, free
    );
    println!("alloctest: OK (alloc fonctionne)");
}

pub fn devices() {
    let serial_state = if crate::drivers::serial::is_ready() {
        "COM1 0x3F8 UART 16550, debug actif"
    } else {
        "non initialise"
    };
    println!("devices detected/configured:");
    println!("  cpu0      x86_64 via CPUID");
    println!("  vga0      legacy VGA text buffer 0xb8000");
    println!("  kbd0      PS/2 keyboard polling, AZERTY-FR mapping");
    println!("  serial0   {}", serial_state);
    println!("  ramfs0    in-memory filesystem mounted on /");
    println!(
        "  pci0      bus scanne ({} peripheriques) - voir 'lspci'",
        crate::arch::x86_64::pci::count()
    );
    match crate::arch::x86_64::pci::find_network() {
        Some(d) => println!(
            "  net0      carte PCI {:04x}:{:04x} detectee, driver non charge",
            d.vendor, d.device
        ),
        None => println!("  net0      aucune carte reseau PCI detectee"),
    }
    println!("  disk0     planned: virtio-blk/BFS persistent FS");
}

pub fn uptime() {
    if timer::timer_enabled() {
        println!(
            "uptime: {} s ({} ticks @ ~{} Hz)",
            timer::seconds(),
            timer::ticks(),
            timer::TICKS_PER_SECOND
        );
    } else {
        println!("uptime: timer interrupts not enabled yet");
        println!(
            "  mesure brute (TSC): {} cycles depuis le boot",
            timer::cycles_since_boot()
        );
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
    println!(
        "hardware IRQ: {}",
        if interrupts::enabled() {
            "enabled"
        } else {
            "disabled (polling clavier)"
        }
    );
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
    println!(
        "uid={}({}) gid={}({})",
        s.uid(),
        s.username(),
        s.gid(),
        s.username()
    );
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
    if argc < 2 {
        println!("usage: useradd <nom>");
        return;
    }
    if !users::session().is_root() {
        println!("useradd: reserve a root");
        return;
    }
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
    if argc < 2 {
        println!("usage: userdel <nom>");
        return;
    }
    if !users::session().is_root() {
        println!("userdel: reserve a root");
        return;
    }
    match users::remove_user(argv[1]) {
        Ok(()) => println!("userdel: {} supprime", argv[1]),
        Err(e) => println!("userdel: {}", e),
    }
}

/// `passwd [user]` : change un mot de passe (soi-meme, ou tout compte si root).
pub fn passwd(argc: usize, argv: &[&str; 12]) {
    let target = if argc >= 2 {
        argv[1]
    } else {
        users::session().username()
    };
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
            if argc >= 3 {
                path = argv[2];
            }
        } else {
            path = argv[1];
        }
    }

    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(path, cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("ls: {}", e);
            return 1;
        }
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
        Err(e) => {
            println!("tree: {}", e);
            return;
        }
    };
    ramfs::print_path(fs, idx);
    println!("");
    tree_rec(idx, 0);
}

fn tree_rec(idx: usize, depth: usize) {
    let fs = ramfs::fs();
    if fs.nodes[idx].kind != NodeKind::Dir {
        return;
    }
    // On n'explore un repertoire que si on a le droit de le lire.
    if !fs.can(idx, PERM_R) {
        for _ in 0..depth {
            print!("  ");
        }
        println!("|- [permission denied]");
        return;
    }
    for i in 0..MAX_NODES {
        if fs.nodes[i].used && i != idx && fs.nodes[i].parent == idx {
            for _ in 0..depth {
                print!("  ");
            }
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
    if argc < 2 {
        *cwd = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
        return 0;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], *cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("cd: {}", e);
            return 1;
        }
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
    if argc < 2 {
        println!("usage: mkdir <path>");
        return 1;
    }
    let fs = ramfs::fs();
    let (parent, name) = match fs.resolve_parent_name_checked(argv[1], cwd) {
        Ok(v) => v,
        Err(e) => {
            println!("mkdir: {}", e);
            return 1;
        }
    };
    if !fs.can(parent, PERM_W) {
        println!("mkdir: permission denied");
        return 1;
    }
    if let Err(e) = fs.mkdir_at(parent, name) {
        println!("mkdir: {}", e);
        return 1;
    }
    0
}

pub fn touch(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        println!("usage: touch <file>");
        return 1;
    }
    let fs = ramfs::fs();
    let (parent, name) = match fs.resolve_parent_name_checked(argv[1], cwd) {
        Ok(v) => v,
        Err(e) => {
            println!("touch: {}", e);
            return 1;
        }
    };
    if !fs.can(parent, PERM_W) {
        println!("touch: permission denied");
        return 1;
    }
    if let Err(e) = fs.touch_at(parent, name) {
        println!("touch: {}", e);
        return 1;
    }
    0
}

pub fn cat(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        // Sans argument : recopie l'entree standard (utile dans un pipe).
        if let Some(s) = crate::shell::take_stdin() {
            print!("{}", s);
            if !s.ends_with('\n') {
                println!("");
            }
            return 0;
        }
        println!("usage: cat <file>");
        return 1;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("cat: {}", e);
            return 1;
        }
    };
    if fs.nodes[idx].kind != NodeKind::File {
        println!("cat: dossier");
        return 1;
    }
    if !fs.can(idx, PERM_R) {
        println!("cat: permission denied");
        return 1;
    }
    // `cat` diffuse par tranches : il n'a aucune raison de tenir en memoire un
    // fichier de 190 Mio, et il doit fonctionner qu'il soit resident ou adosse
    // au disque. `fs.nodes[idx].content` etait vide dans le second cas.
    let taille = crate::fs::backing::logical_len(idx);
    let mut position = 0usize;
    let mut tranche = alloc::vec![0u8; 64 * 1024];
    while position < taille {
        let voulu = core::cmp::min(tranche.len(), taille - position);
        let lus = crate::fs::backing::read_at(idx, position, &mut tranche[..voulu]);
        if lus == 0 {
            println!("cat: lecture interrompue a l'octet {}", position);
            return 1;
        }
        print!("{}", String::from_utf8_lossy(&tranche[..lus]));
        position += lus;
    }
    println!("");
    0
}

pub fn write(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 {
        println!("usage: write <file> <texte>");
        return;
    }
    let text = remainder_after_tokens(line, 2);
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("write: {}", e);
            return;
        }
    };
    if fs.nodes[idx].kind != NodeKind::File {
        println!("write: dossier");
        return;
    }
    if !fs.can(idx, PERM_W) {
        println!("write: permission denied");
        return;
    }
    fs.write_node(idx, text);
}

pub fn append(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 3 {
        println!("usage: append <file> <texte>");
        return;
    }
    let text = remainder_after_tokens(line, 2);
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("append: {}", e);
            return;
        }
    };
    if fs.nodes[idx].kind != NodeKind::File {
        println!("append: dossier");
        return;
    }
    if !fs.can(idx, PERM_W) {
        println!("append: permission denied");
        return;
    }
    fs.append_node(idx, text);
}

pub fn nano(argc: usize, argv: &[&str; 12], cwd: usize) {
    if argc < 2 {
        println!("usage: nano <file>");
        return;
    }
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
                Err(e) => {
                    println!("nano: {}", e);
                    return;
                }
            };
            if !fs.can(parent, PERM_W) {
                println!("nano: permission denied");
                return;
            }
            match fs.touch_at(parent, name) {
                Ok(idx) => idx,
                Err(e) => {
                    println!("nano: {}", e);
                    return;
                }
            }
        }
        Err(e) => {
            println!("nano: {}", e);
            return;
        }
    };
    if fs.nodes[idx].kind != NodeKind::File {
        println!("nano: pas un fichier");
        return;
    }
    if !fs.can(idx, PERM_W) {
        println!("nano: permission denied");
        return;
    }
    fs.write_node(idx, text);
}

pub fn rm(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        println!("usage: rm <file>");
        return 1;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("rm: {}", e);
            return 1;
        }
    };
    if idx == 0 || fs.nodes[idx].kind != NodeKind::File {
        println!("rm: pas un fichier");
        return 1;
    }
    // Supprimer demande le droit d'ecriture sur le repertoire parent.
    if !fs.can(fs.nodes[idx].parent, PERM_W) {
        println!("rm: permission denied");
        return 1;
    }
    fs.nodes[idx].used = false;
    0
}

pub fn rmdir(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        println!("usage: rmdir <dir>");
        return 1;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("rmdir: {}", e);
            return 1;
        }
    };
    if idx == 0 || fs.nodes[idx].kind != NodeKind::Dir {
        println!("rmdir: pas un dossier");
        return 1;
    }
    if !fs.is_empty_dir(idx) {
        println!("rmdir: dossier non vide");
        return 1;
    }
    if !fs.can(fs.nodes[idx].parent, PERM_W) {
        println!("rmdir: permission denied");
        return 1;
    }
    fs.nodes[idx].used = false;
    0
}

pub fn cp(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 {
        println!("usage: cp <src> <dst>");
        return 1;
    }
    let fs = ramfs::fs();
    let src = match fs.resolve_checked(argv[1], cwd) {
        Ok(idx) if fs.nodes[idx].kind == NodeKind::File => idx,
        Ok(_) => {
            println!("cp: source invalide");
            return 1;
        }
        Err(e) => {
            println!("cp: {}", e);
            return 1;
        }
    };
    if !fs.can(src, PERM_R) {
        println!("cp: permission denied (source)");
        return 1;
    }
    let (parent, name) = match fs.resolve_parent_name_checked(argv[2], cwd) {
        Ok(v) => v,
        Err(e) => {
            println!("cp: {}", e);
            return 1;
        }
    };
    if !fs.can(parent, PERM_W) {
        println!("cp: permission denied (destination)");
        return 1;
    }
    let dst = match fs.touch_at(parent, name) {
        Ok(idx) => idx,
        Err(e) => {
            println!("cp: {}", e);
            return 1;
        }
    };
    let data = fs.nodes[src].content.clone();
    fs.nodes[dst].content = data;
    0
}

pub fn mv(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 {
        println!("usage: mv <src> <dst>");
        return 1;
    }
    let fs = ramfs::fs();
    let src = match fs.resolve_checked(argv[1], cwd) {
        Ok(idx) if idx != 0 => idx,
        Ok(_) => {
            println!("mv: source invalide");
            return 1;
        }
        Err(e) => {
            println!("mv: {}", e);
            return 1;
        }
    };
    if !fs.can(fs.nodes[src].parent, PERM_W) {
        println!("mv: permission denied (source)");
        return 1;
    }
    let (parent, name) = match fs.resolve_parent_name_checked(argv[2], cwd) {
        Ok(v) => v,
        Err(e) => {
            println!("mv: {}", e);
            return 1;
        }
    };
    if !fs.can(parent, PERM_W) {
        println!("mv: permission denied (destination)");
        return 1;
    }
    if fs.find_child(parent, name).is_some() {
        println!("mv: destination existe deja");
        return 1;
    }
    fs.nodes[src].parent = parent;
    if !fs.nodes[src].set_name(name) {
        println!("mv: nom invalide");
        return 1;
    }
    0
}

pub fn stat(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        println!("usage: stat <path>");
        return 1;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("stat: {}", e);
            return 1;
        }
    };
    let n = &fs.nodes[idx];
    print!("path: ");
    ramfs::print_path(fs, idx);
    println!("");
    print!("type: ");
    println!(
        "{}",
        if n.kind == NodeKind::Dir {
            "directory"
        } else {
            "file"
        }
    );
    print!("mode: ");
    ramfs::print_mode(n.kind, n.mode);
    println!("  octal={:o}", n.mode);
    println!("uid: {}", n.uid);
    println!("gid: {}", n.gid);
    println!("size: {}", n.content.len());
    0
}

fn parse_octal(s: &str) -> Option<u16> {
    let mut value: u16 = 0;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if b < b'0' || b > b'7' {
            return None;
        }
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
            b'a' => {
                who_u = true;
                who_g = true;
                who_o = true;
            }
            _ => break,
        }
        i += 1;
    }
    if !who_u && !who_g && !who_o {
        // Aucune cible => 'a' par defaut (comme sous Unix).
        who_u = true;
        who_g = true;
        who_o = true;
    }
    if i >= bytes.len() {
        return None;
    }
    let op = bytes[i];
    if op != b'+' && op != b'-' && op != b'=' {
        return None;
    }
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
    if who_u {
        mask |= perm << 6;
    }
    if who_g {
        mask |= perm << 3;
    }
    if who_o {
        mask |= perm;
    }
    match op {
        b'+' => mode |= mask,
        b'-' => mode &= !mask,
        b'=' => {
            // Remet a zero les groupes vises puis applique.
            let mut clear = 0u16;
            if who_u {
                clear |= 0o700;
            }
            if who_g {
                clear |= 0o070;
            }
            if who_o {
                clear |= 0o007;
            }
            mode = (mode & !clear) | mask;
        }
        _ => {}
    }
    Some(mode)
}

pub fn chmod(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 {
        println!("usage: chmod <octal|+x|u+w|go-r|...> <path>");
        return 1;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[2], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("chmod: {}", e);
            return 1;
        }
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
            None => {
                println!("chmod: mode invalide");
                return 1;
            }
        },
    };
    fs.nodes[idx].mode = new_mode;
    0
}

fn parse_u16(s: &str) -> Option<u16> {
    let mut value: u32 = 0;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + (b - b'0') as u32;
        if value > 65535 {
            return None;
        }
    }
    Some(value as u16)
}

pub fn chown(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 3 {
        println!("usage: chown <uid|user> <path>");
        return 1;
    }
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
            None => {
                println!("chown: utilisateur/uid invalide");
                return 1;
            }
        },
    };
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[2], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("chown: {}", e);
            return 1;
        }
    };
    fs.nodes[idx].uid = new_uid;
    fs.nodes[idx].gid = new_uid;
    0
}

/// Ecrit `data` dans le fichier `path` (cree si besoin), en mode ecriture ou
/// ajout. Utilise par les redirections `>` et `>>` du shell.
pub fn redirect(path: &str, data: &str, append: bool, cwd: usize) -> i32 {
    if path.is_empty() {
        println!("redirection: fichier cible manquant");
        return 1;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(path, cwd) {
        Ok(i) => i,
        Err("introuvable") => {
            let (parent, name) = match fs.resolve_parent_name_checked(path, cwd) {
                Ok(v) => v,
                Err(e) => {
                    println!("redirection: {}", e);
                    return 1;
                }
            };
            if !fs.can(parent, PERM_W) {
                println!("redirection: permission denied");
                return 1;
            }
            match fs.touch_at(parent, name) {
                Ok(i) => i,
                Err(e) => {
                    println!("redirection: {}", e);
                    return 1;
                }
            }
        }
        Err(e) => {
            println!("redirection: {}", e);
            return 1;
        }
    };
    if fs.nodes[idx].kind != NodeKind::File {
        println!("redirection: pas un fichier");
        return 1;
    }
    if !fs.can(idx, PERM_W) {
        println!("redirection: permission denied");
        return 1;
    }
    if append {
        fs.append_node(idx, data);
    } else {
        fs.write_node(idx, data);
    }
    0
}

// ---------------------------------------------------------------------------
// Horloge et coreutils (grep / wc / head / tail / find)
// ---------------------------------------------------------------------------

/// `expr-selftest` : verifie l'evaluateur d'expressions de la calculatrice.
pub fn expr_selftest() {
    match crate::lang::expr::selftest() {
        Ok(()) => println!("expr-selftest: OK"),
        Err(e) => println!("expr-selftest: ECHEC ({})", e),
    }
}

pub fn wasm_selftest() {
    match crate::wasm::selftest() {
        Ok(()) => println!("wasm-selftest: OK (add(2,3)=5)"),
        Err(e) => println!("wasm-selftest: ECHEC ({})", e),
    }
}

/// Execute un module WebAssembly (`.wasm`) depuis le systeme de fichiers via le
/// runtime `wasmi` embarque. Appelle `_start` / `main` / `run`.
pub fn wasm(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        println!("usage: wasm <fichier.wasm>");
        return 1;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("wasm: {}", e);
            return 1;
        }
    };
    if fs.nodes[idx].kind != NodeKind::File {
        println!("wasm: pas un fichier");
        return 1;
    }
    if !fs.can(idx, PERM_R) {
        println!("wasm: permission denied");
        return 1;
    }
    let len = fs.nodes[idx].content.len();
    let mut bytes = alloc::vec::Vec::with_capacity(len);
    for i in 0..len {
        bytes.push(fs.nodes[idx].content[i]);
    }

    let res = crate::wasm::run_bytes(&bytes);
    if !res.output.is_empty() {
        print!("{}", res.output);
        if !res.output.ends_with('\n') {
            println!("");
        }
    }
    if let Some(code) = res.exit_code {
        println!("wasm: proc_exit({})", code);
    }
    if let Some(v) = res.result {
        println!("wasm: resultat = {}", v);
    }
    match res.error {
        Some(e) => {
            println!("wasm: {}", e);
            1
        }
        None => res.exit_code.unwrap_or(0),
    }
}

pub fn date() {
    let dt = crate::arch::x86_64::rtc::now();
    println!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC+2",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
    );
}

/// Recupere le texte a traiter : depuis un fichier si fourni, sinon depuis le
/// pipe (stdin). Renvoie None et affiche une erreur si le fichier est invalide.
/// Ce qu'un outil texte du shell accepte de charger d'un coup.
///
/// Les fichiers de l'archive de boot qui depassent `INLINE_BOOT_FILE_SIZE` ne
/// sont pas residents : `Node::content` est vide et leurs octets restent sur
/// hdb. Un runtime Ladybird pese 190 Mio ; le charger entier dans le tas noyau
/// pour un `grep` serait deraisonnable. Au-dela de cette borne, on le DIT.
const MAX_TEXTE_SHELL: usize = 8 * 1024 * 1024;

/// Lit le contenu d'un nœud, qu'il soit resident ou adosse au disque.
///
/// `Node::content` ne suffit pas : `tar::index_data_disk` le vide pour les gros
/// fichiers et n'enregistre que leur etendue sur hdb. Les outils texte du shell
/// lisaient ce champ directement et voyaient donc un fichier VIDE la ou `ls -l`
/// annonce 190 Mio — un mensonge silencieux, exactement celui qu'on ne veut pas.
fn lit_noeud(idx: usize, who: &str) -> Option<String> {
    let taille = crate::fs::backing::logical_len(idx);
    if taille > MAX_TEXTE_SHELL {
        println!(
            "{}: fichier de {} octets, au-dela des {} octets qu'un outil texte charge",
            who, taille, MAX_TEXTE_SHELL
        );
        return None;
    }
    let mut octets = alloc::vec![0u8; taille];
    let lus = crate::fs::backing::read_at(idx, 0, &mut octets);
    if lus < taille {
        println!("{}: lecture incomplete ({} sur {} octets)", who, lus, taille);
        octets.truncate(lus);
    }
    Some(String::from_utf8_lossy(&octets).into_owned())
}

fn input_text(path: Option<&str>, cwd: usize, who: &str) -> Option<String> {
    match path {
        Some(p) => {
            let fs = ramfs::fs();
            let idx = match fs.resolve_checked(p, cwd) {
                Ok(i) => i,
                Err(e) => {
                    println!("{}: {}", who, e);
                    return None;
                }
            };
            if fs.nodes[idx].kind != NodeKind::File {
                println!("{}: pas un fichier", who);
                return None;
            }
            if !fs.can(idx, PERM_R) {
                println!("{}: permission denied", who);
                return None;
            }
            lit_noeud(idx, who)
        }
        None => Some(crate::shell::take_stdin().unwrap_or_default()),
    }
}

pub fn grep(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        println!("usage: grep <motif> [fichier]");
        return 1;
    }
    let pat = argv[1];
    let file = if argc >= 3 { Some(argv[2]) } else { None };
    let content = match input_text(file, cwd, "grep") {
        Some(c) => c,
        None => return 2,
    };
    let mut found = false;
    for line in content.lines() {
        if line.contains(pat) {
            println!("{}", line);
            found = true;
        }
    }
    if found {
        0
    } else {
        1
    }
}

pub fn wc(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    let file = if argc >= 2 { Some(argv[1]) } else { None };
    let content = match input_text(file, cwd, "wc") {
        Some(c) => c,
        None => return 1,
    };
    let lines = content.lines().count();
    let words = content.split_whitespace().count();
    let bytes = content.len();
    println!("{:>6} {:>6} {:>6}", lines, words, bytes);
    0
}

/// Analyse une eventuelle option `-n N` et renvoie (nombre, index du fichier).
fn parse_n(argc: usize, argv: &[&str; 12]) -> (usize, Option<usize>) {
    if argc >= 3 && argv[1] == "-n" {
        let n = argv[2].parse::<usize>().unwrap_or(10);
        let file = if argc >= 4 { Some(3) } else { None };
        (n, file)
    } else {
        let file = if argc >= 2 { Some(1) } else { None };
        (10, file)
    }
}

pub fn head(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    let (n, fidx) = parse_n(argc, argv);
    let content = match input_text(fidx.map(|i| argv[i]), cwd, "head") {
        Some(c) => c,
        None => return 1,
    };
    for (i, line) in content.lines().enumerate() {
        if i >= n {
            break;
        }
        println!("{}", line);
    }
    0
}

pub fn tail(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    let (n, fidx) = parse_n(argc, argv);
    let content = match input_text(fidx.map(|i| argv[i]), cwd, "tail") {
        Some(c) => c,
        None => return 1,
    };
    let total = content.lines().count();
    let skip = if total > n { total - n } else { 0 };
    for line in content.lines().skip(skip) {
        println!("{}", line);
    }
    0
}

pub fn find(argc: usize, argv: &[&str; 12], cwd: usize) {
    let path = if argc >= 2 { argv[1] } else { "." };
    let filter = if argc >= 3 { Some(argv[2]) } else { None };
    let fs = ramfs::fs();
    let start = match fs.resolve_checked(path, cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("find: {}", e);
            return;
        }
    };
    find_rec(start, filter);
}

fn find_rec(idx: usize, filter: Option<&str>) {
    let fs = ramfs::fs();
    for i in 0..MAX_NODES {
        if fs.nodes[i].used && i != idx && fs.nodes[i].parent == idx {
            let name = fs.nodes[i].name_str();
            if filter.map_or(true, |f| name.contains(f)) {
                ramfs::print_path(fs, i);
                println!("");
            }
            if fs.nodes[i].kind == NodeKind::Dir {
                find_rec(i, filter);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Rustc / Cargo — interpréteur Rust minimal
// ---------------------------------------------------------------------------

/// Lance le mini-interpréteur Rust sur un fichier .rs ou un snippet inline.
/// Usage : `rustc <fichier.rs>` ou `cargo run` (exécute main.rs du repo courant).
pub fn rustc_run(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    let src = if argv[0] == "cargo" {
        // `cargo run` — cherche main.rs dans src/ ou racine du répertoire courant
        let fs = ramfs::fs();
        let src_dir = fs.find_child(cwd, "src");
        let main_idx = src_dir
            .and_then(|d| fs.find_child(d, "main.rs"))
            .or_else(|| fs.find_child(cwd, "main.rs"));
        match main_idx {
            Some(idx) if fs.nodes[idx].kind == NodeKind::File => {
                let n = &fs.nodes[idx];
                let mut s = alloc::string::String::new();
                s.push_str(&n.content_str());
                s
            }
            _ => {
                println!("cargo: main.rs introuvable (cherche src/main.rs ou ./main.rs)");
                return 1;
            }
        }
    } else if argc < 2 {
        println!("usage: rustc <fichier.rs>");
        return 1;
    } else {
        let fs = ramfs::fs();
        let idx = match fs.resolve_checked(argv[1], cwd) {
            Ok(i) => i,
            Err(e) => {
                println!("rustc: {}: {}", argv[1], e);
                return 1;
            }
        };
        if fs.nodes[idx].kind != NodeKind::File {
            println!("rustc: {} n'est pas un fichier", argv[1]);
            return 1;
        }
        if !fs.can(idx, PERM_R) {
            println!("rustc: permission denied");
            return 1;
        }
        let n = &fs.nodes[idx];
        let mut s = alloc::string::String::new();
        s.push_str(&n.content_str());
        s
    };

    let (output, err) = crate::lang::mini_rust::run(&src);
    if !output.is_empty() {
        print!("{}", output);
        if !output.ends_with('\n') {
            println!("");
        }
    }
    match err {
        Some(e) => {
            vga::set_color(vga::COLOR_RED);
            println!("erreur: {}", e);
            vga::set_color(COLOR_DEFAULT);
            1
        }
        None => 0,
    }
}

/// Lance une batterie de tests unitaires sur le mini-interpréteur Rust.
pub fn rust_selftest() {
    let mut ok = 0u32;
    let mut fail = 0u32;

    let cases: &[(&str, &str)] = &[
        // arithmétique basique
        ("fn main() { println!(\"{}\", 2 + 3); }", "5\n"),
        // variables let
        (
            "fn main() { let x = 10; let y = x * 2; println!(\"{}\", y); }",
            "20\n",
        ),
        // if/else
        (
            "fn main() { let x = 5; if x > 3 { println!(\"ok\"); } else { println!(\"ko\"); } }",
            "ok\n",
        ),
        // boucle for + range
        (
            "fn main() { let mut s = 0; for i in 0..5 { s = s + i; } println!(\"{}\", s); }",
            "10\n",
        ),
        // fonction auxiliaire
        (
            "fn double(x: i64) -> i64 { x * 2 } fn main() { println!(\"{}\", double(7)); }",
            "14\n",
        ),
        // chaîne de caractères
        (
            "fn main() { let s = \"bonjour\"; println!(\"{}\", s.len()); }",
            "7\n",
        ),
        // while
        (
            "fn main() { let mut n = 1; while n < 10 { n = n * 2; } println!(\"{}\", n); }",
            "16\n",
        ),
        // booléens
        (
            "fn main() { let a = true; let b = false; println!(\"{}\", a && !b); }",
            "true\n",
        ),
    ];

    for (src, expected) in cases {
        let (out, err) = crate::lang::mini_rust::run(src);
        let pass = err.is_none() && out == *expected;
        if pass {
            ok += 1;
        } else {
            fail += 1;
            vga::set_color(vga::COLOR_RED);
            println!("FAIL: {:?}", src);
            println!("  attendu:  {:?}", expected);
            println!("  obtenu:   {:?}", out);
            if let Some(e) = err {
                println!("  erreur:   {}", e);
            }
            vga::set_color(COLOR_DEFAULT);
        }
    }

    if fail == 0 {
        vga::set_color(vga::COLOR_GREEN);
        println!("rust-selftest: {}/{} OK", ok, ok);
    } else {
        vga::set_color(vga::COLOR_RED);
        println!("rust-selftest: {}/{} OK, {} echecs", ok, ok + fail, fail);
    }
    vga::set_color(COLOR_DEFAULT);
}

// ---------------------------------------------------------------------------
// Python — interpréteur RustPython embarqué (WASM/WASI) + pip
// ---------------------------------------------------------------------------

/// `python` : REPL interactif sans argument, `python -c "code"`, ou
/// `python <fichier.py> [args...]`. Voir `lang::python`.
pub fn python_run(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    let cwd_path = ramfs::path_string(ramfs::fs(), cwd);

    if argc < 2 {
        // REPL : lecture clavier bloquante, disponible dans le shell texte.
        if crate::drivers::gfx::is_active() {
            println!("python: REPL indisponible dans le bureau graphique (utilise le shell texte)");
            println!("        (python <fichier.py> et python -c \"code\" restent utilisables)");
            return 1;
        }
        return crate::lang::python::run_repl(cwd, &cwd_path);
    }

    if argv[1] == "-c" {
        // Le shell ne gere pas les guillemets : tout ce qui suit `-c` est le code.
        let code = remainder_after_tokens(line, 2);
        if code.is_empty() {
            println!("usage: python -c \"code\"");
            return 1;
        }
        return crate::lang::python::run_code(code, cwd, &cwd_path);
    }

    // Fichier : resolu en chemin absolu, le module WASM l'ouvre via WASI.
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("python: {}: {}", argv[1], e);
            return 1;
        }
    };
    if fs.nodes[idx].kind != NodeKind::File {
        println!("python: {} n'est pas un fichier", argv[1]);
        return 1;
    }
    if !fs.can(idx, PERM_R) {
        println!("python: permission denied");
        return 1;
    }
    let abs = ramfs::path_string(fs, idx);
    let mut extra: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for i in 2..argc {
        extra.push(argv[i]);
    }
    crate::lang::python::run_file(&abs, &extra, cwd, &cwd_path)
}

/// `pybrowser [url|--check]` : navigateur Web simpliste ecrit en Python.
///
/// Sert de test d'integration des couches : interpreteur Python -> pont WASI
/// -> RAMFS -> pile TCP/TLS du noyau -> console. Le script vit dans
/// `/usr/lib/python/browser.py`, installe au demarrage par `lang::pyweb`.
pub fn pybrowser_cmd(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    let cwd_path = ramfs::path_string(ramfs::fs(), cwd);

    // Liste d'arguments Python, construite depuis argv.
    let mut args = String::from("[");
    let mut count = 0;
    for i in 1..argc {
        let token = argv[i];
        if token.is_empty() {
            continue;
        }
        if count > 0 {
            args.push_str(", ");
        }
        args.push('"');
        for c in token.chars() {
            // Une URL encodee ne contient ni guillemet ni antislash, mais on
            // ne fait pas confiance a la saisie.
            if c == '"' || c == '\\' {
                args.push('\\');
            }
            args.push(c);
        }
        args.push('"');
        count += 1;
    }
    args.push(']');

    let code = alloc::format!(
        "import sys\n\
         sys.path.insert(0, '/usr/lib/python')\n\
         import browser\n\
         sys.exit(browser.main({}))\n",
        args
    );
    crate::lang::python::run_code(&code, cwd, &cwd_path)
}

/// `pip install <paquet>` / `pip list` : voir `lang::pip`.
pub fn pip_cmd(argc: usize, argv: &[&str; 12]) -> i32 {
    crate::lang::pip::cmd(argc, argv)
}

/// Selftest Python : execute un petit programme via toute la chaine
/// (wasmi + WASI + RustPython) et verifie la sortie.
pub fn python_selftest() {
    match crate::lang::python::selftest() {
        Ok(()) => println!("python-selftest: OK (sum(x*x)=30)"),
        Err(e) => {
            vga::set_color(vga::COLOR_RED);
            println!("python-selftest: ECHEC ({})", e);
            vga::set_color(COLOR_DEFAULT);
        }
    }
}

// ---------------------------------------------------------------------------
// Disque (placeholders, roadmap BFS)
// ---------------------------------------------------------------------------

pub fn disk_placeholder(cmd: &str) {
    vga::set_color(COLOR_YELLOW);
    println!("{}: pas encore implemente", cmd);
    vga::set_color(COLOR_DEFAULT);
    println!("  actuel: RAMFS volatil monte sur /, zone persistante sur {}", crate::fs::persistance::RACINE);
    println!("  roadmap: block device -> virtio-blk -> BFS (Bouchaud File System) persistant");
}

/// `sync` : ecrit la zone persistante sur le disque de donnees.
///
/// La commande annoncait jusqu'ici que « le stockage persistant n'est pas
/// active dans V0.6 », alors que `/persist` existe et que les programmes y
/// ecrivent deja par `fsync`. C'etait le seul moyen depuis le shell de prouver
/// qu'un fichier survit a un redemarrage, et il mentait. Elle appelle donc
/// maintenant la meme primitive que l'appel systeme : `persistance::synchronise`.
pub fn sync_persistant() -> i32 {
    let ecrits = crate::fs::persistance::synchronise();
    if ecrits < 0 {
        vga::set_color(COLOR_YELLOW);
        println!("sync: zone persistante indisponible (disque de donnees trop petit ou absent)");
        vga::set_color(COLOR_DEFAULT);
        return 1;
    }
    println!("sync: {} fichier(s) ecrit(s) sous {}", ecrits, crate::fs::persistance::RACINE);
    0
}

// ---------------------------------------------------------------------------
// Mode utilisateur : binaires Linux natifs en ring 3
// ---------------------------------------------------------------------------

/// `exec <binaire> [args...]` : charge un ELF64 et l'execute en ring 3.
pub fn exec_cmd(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        println!("usage: exec <binaire-elf64> [arguments...]");
        println!(
            "  le binaire doit etre statique-PIE, ou lie a {:#x}",
            crate::kernel::vmm::user_load_base()
        );
        println!("  voir tools/userland/README.md pour la chaine musl");
        return 1;
    }
    let mut args = alloc::vec::Vec::new();
    for i in 1..argc {
        if !argv[i].is_empty() {
            args.push(String::from(argv[i]));
        }
    }
    let env = crate::kernel::exec::shell_environment();
    match crate::kernel::exec::exec(argv[1], &args, &env, cwd) {
        Ok(code) => {
            if code != 0 {
                println!("exec: {} termine avec le code {}", argv[1], code);
            }
            code
        }
        Err(message) => {
            vga::set_color(vga::COLOR_RED);
            println!("exec: {}", message);
            vga::set_color(COLOR_DEFAULT);
            1
        }
    }
}

/// Lance un programme nomme directement, sans le mot-cle `exec`.
///
/// C'est le repli du shell quand un mot n'est aucune de ses commandes
/// internes, et c'est le comportement de tout shell POSIX : un token qui
/// contient une barre oblique designe un chemin, sinon on le cherche dans
/// `PATH`. Le shell rendait jusqu'ici « commande inconnue » et 127, si bien
/// qu'un autorun ecrit normalement ne demarrait rien :
///
///     + /usr/libexec/ladybird/BouchaudBrowserHost
///     /usr/libexec/ladybird/BouchaudBrowserHost: commande inconnue
///
/// Le diagnostic reste distinct dans les deux cas qui comptent : un nom
/// introuvable dit qu'il est introuvable, un fichier present mais non
/// executable dit qu'il ne l'est pas. Confondre les deux, c'est envoyer
/// chercher un binaire manquant qui est en fait la, sans son bit `+x`.
pub fn execute_programme(line: &str, argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    let nom = argv[0];
    if nom.is_empty() {
        return 0;
    }

    let chemin = match resout_programme(nom, cwd) {
        Some(chemin) => chemin,
        None => {
            vga::set_color(vga::COLOR_RED);
            println!("{}: commande inconnue", nom);
            vga::set_color(COLOR_DEFAULT);
            return 127;
        }
    };

    {
        let fs = ramfs::fs();
        if let Some(idx) = fs.resolve(&chemin, cwd) {
            if !fs.can(idx, PERM_X) {
                vga::set_color(vga::COLOR_RED);
                println!("{}: permission refusee (bit d'execution absent)", chemin);
                vga::set_color(COLOR_DEFAULT);
                return 126;
            }
        }
    }

    let mut args = alloc::vec::Vec::new();
    args.push(String::from(chemin.as_str()));
    for i in 1..argc {
        if !argv[i].is_empty() {
            args.push(String::from(argv[i]));
        }
    }
    let _ = line;

    let env = crate::kernel::exec::shell_environment();
    match crate::kernel::exec::exec(&chemin, &args, &env, cwd) {
        Ok(code) => code,
        Err(message) => {
            vga::set_color(vga::COLOR_RED);
            println!("{}: {}", nom, message);
            vga::set_color(COLOR_DEFAULT);
            126
        }
    }
}

/// Le chemin d'un programme nomme, ou `None` s'il n'existe nulle part.
///
/// Un token qui contient `/` est un chemin, relatif ou absolu. Sinon on
/// parcourt `PATH` s'il est defini, et a defaut les repertoires ou Bouchaud
/// installe ses programmes.
fn resout_programme(nom: &str, cwd: usize) -> Option<String> {
    let fs = ramfs::fs();
    let est_fichier = |chemin: &str| -> bool {
        matches!(fs.resolve(chemin, cwd), Some(idx) if fs.nodes[idx].kind == NodeKind::File)
    };

    if nom.contains('/') {
        return if est_fichier(nom) { Some(String::from(nom)) } else { None };
    }

    let chemin_env = crate::shell::path_de_recherche();
    for repertoire in chemin_env.split(':') {
        if repertoire.is_empty() {
            continue;
        }
        let candidat = alloc::format!("{}/{}", repertoire.trim_end_matches('/'), nom);
        if est_fichier(&candidat) {
            return Some(candidat);
        }
    }
    None
}

/// `elfinfo <fichier>` : analyse un binaire sans l'executer.
pub fn elfinfo(argc: usize, argv: &[&str; 12], cwd: usize) -> i32 {
    if argc < 2 {
        println!("usage: elfinfo <fichier>");
        return 1;
    }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], cwd) {
        Ok(i) => i,
        Err(e) => {
            println!("elfinfo: {}", e);
            return 1;
        }
    };
    if fs.nodes[idx].kind != NodeKind::File {
        println!("elfinfo: {} n'est pas un fichier", argv[1]);
        return 1;
    }
    let data = fs.nodes[idx].content.clone();
    crate::kernel::elf::describe(&data);
    0
}

/// `strace on|off` : trace les appels systeme sur la sortie serie.
pub fn strace(argc: usize, argv: &[&str; 12]) {
    if argc < 2 {
        println!("usage: strace on|echecs|off");
        println!("  on      tous les appels : inutilisable sur un vrai programme");
        println!("  echecs  seuls ceux qui rendent une erreur, hors EAGAIN/EINTR");
        println!(
            "etat actuel : {}",
            if crate::kernel::abi::trace_enabled() {
                "actif"
            } else {
                "inactif"
            }
        );
        return;
    }
    match argv[1] {
        "on" => {
            crate::kernel::abi::set_trace_mode(crate::kernel::abi::Trace::Tous);
            println!("strace: tous les appels systeme (sortie serie COM1)");
        }
        "echecs" | "errors" => {
            crate::kernel::abi::set_trace_mode(crate::kernel::abi::Trace::Echecs);
            println!("strace: seuls les appels en echec (hors EAGAIN/EINTR)");
        }
        "off" => {
            crate::kernel::abi::set_trace_mode(crate::kernel::abi::Trace::Aucune);
            println!("strace: trace desactivee");
        }
        _ => println!("usage: strace on|echecs|off"),
    }
}
