//! Shell interactif Unix-like de Bouchaud OS.
//!
//! `mod.rs` contient la boucle principale, le decoupage en arguments et le
//! dispatcher de commandes. L'implementation de chaque commande vit dans
//! `commands.rs`.

pub mod commands;
pub mod history;

use crate::drivers::keyboard;
use crate::drivers::vga::{self, COLOR_GREEN, COLOR_CYAN, COLOR_DEFAULT, COLOR_RED};
use crate::fs::ramfs;
use crate::kernel::dmesg;
use crate::users;

/// Boucle principale du shell : affiche le prompt, lit une ligne, l'execute.
pub fn run() -> ! {
    // Demarre dans le repertoire d'accueil de l'utilisateur courant.
    let mut cwd = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut line_buf = [0u8; 256];

    loop {
        vga::set_color(COLOR_GREEN);
        print!("{}@bouchaud-os:", users::session().username());
        vga::set_color(COLOR_CYAN);
        ramfs::print_path(ramfs::fs(), cwd);
        vga::set_color(COLOR_GREEN);
        print!("$ ");
        vga::set_color(COLOR_DEFAULT);

        let len = keyboard::read_line(&mut line_buf);
        let line = unsafe { core::str::from_utf8_unchecked(&line_buf[..len]) };
        let trimmed = trim(line);
        if trimmed.is_empty() { continue; }

        history::record(trimmed);
        dmesg::log("shell: commande executee");
        dispatch(trimmed, &mut cwd);
    }
}

/// Dispatcher principal : route le premier token vers la bonne commande.
fn dispatch(line: &str, cwd: &mut usize) {
    let mut argv = [""; 12];
    let argc = tokenize(line, &mut argv);
    if argc == 0 { return; }

    use commands as c;
    match argv[0] {
        // Aide et systeme
        "help" => c::help(),
        "clear" => vga::clear(),
        "version" => c::version(),
        "uname" => c::uname(),
        "sysinfo" => c::sysinfo(),
        "cpuinfo" => c::cpuinfo(),
        "meminfo" => c::meminfo(),
        "devices" => c::devices(),
        "dmesg" => dmesg::print(),
        "history" => c::history(argc, &argv),
        "uptime" => c::uptime(),
        "ticks" => c::ticks(),
        "interrupts" => c::interrupts(),
        "serial-test" => c::serial_test(),
        "panic-test" => c::panic_test(),
        "roadmap" => c::roadmap(),

        // Sessions / utilisateurs
        "whoami" => println!("{}", users::session().username()),
        "id" => c::id(),
        "users" => c::users(),
        "login" => c::login(argc, &argv, cwd),
        "logout" => c::logout(cwd),
        "su" => c::su(argc, &argv, cwd),

        // Fichiers
        "pwd" => { ramfs::print_path(ramfs::fs(), *cwd); println!(""); }
        "ls" => c::ls(argc, &argv, *cwd),
        "tree" => c::tree(argc, &argv, *cwd),
        "cd" => c::cd(argc, &argv, cwd),
        "mkdir" => c::mkdir(argc, &argv, *cwd),
        "touch" => c::touch(argc, &argv, *cwd),
        "cat" => c::cat(argc, &argv, *cwd),
        "write" => c::write(line, argc, &argv, *cwd),
        "append" => c::append(line, argc, &argv, *cwd),
        "nano" => c::nano(argc, &argv, *cwd),
        "rm" => c::rm(argc, &argv, *cwd),
        "rmdir" => c::rmdir(argc, &argv, *cwd),
        "cp" => c::cp(argc, &argv, *cwd),
        "mv" => c::mv(argc, &argv, *cwd),
        "stat" => c::stat(argc, &argv, *cwd),
        "chmod" => c::chmod(argc, &argv, *cwd),
        "chown" => c::chown(argc, &argv, *cwd),
        "echo" => println!("{}", remainder_after_tokens(line, 1)),
        "lspci" => crate::arch::x86_64::pci::print_devices(),

        // Reseau (placeholders, pile non activee)
        "ifconfig" | "ip" | "route" | "arp" | "ping" | "dhcp" | "dns" | "wget" | "curl" => {
            crate::net::placeholder(argv[0])
        }

        // Disque (placeholders, roadmap BFS)
        "mount" | "df" | "sync" | "mkfs.bfs" => c::disk_placeholder(argv[0]),

        _ => {
            vga::set_color(COLOR_RED);
            println!("{}: commande inconnue", argv[0]);
            vga::set_color(COLOR_DEFAULT);
        }
    }
}

/// Supprime les espaces en debut et fin de chaine.
pub fn trim(s: &str) -> &str {
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

/// Decoupe une ligne en tokens separes par des espaces.
pub fn tokenize<'a>(line: &'a str, out: &mut [&'a str; 12]) -> usize {
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

/// Renvoie le reste de la ligne apres `n` tokens (pour `echo`, `write`, ...).
pub fn remainder_after_tokens(line: &str, n: usize) -> &str {
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
