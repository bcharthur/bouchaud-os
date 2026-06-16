//! Shell interactif Unix-like de Bouchaud OS.
//!
//! `mod.rs` contient la boucle principale, le decoupage en arguments et le
//! dispatcher de commandes. L'implementation de chaque commande vit dans
//! `commands.rs`.

pub mod commands;
pub mod editor;
pub mod history;

use crate::drivers::keyboard::{self, Key};
use crate::drivers::vga::{self, COLOR_GREEN, COLOR_CYAN, COLOR_DEFAULT, COLOR_RED};
use crate::fs::ramfs;
use crate::kernel::dmesg;
use crate::users;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Code de retour de la derniere commande (consultable via `$?`).
static mut LAST_STATUS: i32 = 0;
fn last_status() -> i32 { unsafe { LAST_STATUS } }
fn set_status(c: i32) { unsafe { LAST_STATUS = c; } }

/// Entree standard transmise a une commande via un pipe (`cmd1 | cmd2`).
static mut STDIN: Option<String> = None;
fn set_stdin(data: Option<String>) { unsafe { STDIN = data; } }
/// Recupere (et consomme) l'entree standard fournie par un pipe.
pub fn take_stdin() -> Option<String> { unsafe { STDIN.take() } }

// --- Variables d'environnement --------------------------------------------

static mut ENV: Option<Vec<(String, String)>> = None;

fn env_mut() -> &'static mut Vec<(String, String)> {
    unsafe {
        if ENV.is_none() { ENV = Some(Vec::new()); }
        ENV.as_mut().unwrap()
    }
}

fn env_set(name: &str, val: &str) {
    let env = env_mut();
    for (k, v) in env.iter_mut() {
        if k == name { *v = val.to_string(); return; }
    }
    env.push((name.to_string(), val.to_string()));
}

fn env_get(name: &str) -> Option<String> {
    for (k, v) in env_mut().iter() {
        if k == name { return Some(v.clone()); }
    }
    None
}

fn env_unset(name: &str) {
    env_mut().retain(|(k, _)| k != name);
}

fn env_list() {
    for (k, v) in env_mut().iter() {
        println!("{}={}", k, v);
    }
}

/// Traite `export NOM=valeur` (a partir de la ligne complete).
fn env_export(line: &str) {
    let rest = remainder_after_tokens(line, 1);
    match rest.find('=') {
        Some(p) => {
            let name = trim(&rest[..p]);
            let val = trim(&rest[p + 1..]);
            if name.is_empty() { println!("usage: export NOM=valeur"); return; }
            env_set(name, val);
        }
        None => {
            // `export NOM` sans valeur : variable vide.
            if rest.is_empty() { env_list(); } else { env_set(trim(rest), ""); }
        }
    }
}

/// Liste des commandes connues, pour la tab-completion.
pub const COMMANDS: &[&str] = &[
    "help", "clear", "version", "uname", "sysinfo", "cpuinfo", "meminfo", "alloctest",
    "devices", "dmesg", "history", "uptime", "ticks", "interrupts", "breakpoint",
    "serial-test", "panic-test", "roadmap", "whoami", "id", "users", "useradd",
    "userdel", "passwd", "su", "pwd", "ls", "tree", "cd", "mkdir", "touch", "cat",
    "write", "append", "nano", "edit", "rm", "rmdir", "cp", "mv", "stat", "chmod", "chown",
    "echo", "date", "grep", "wc", "head", "tail", "find", "lspci", "ping", "ifconfig",
    "ip", "route", "arp", "dhcp", "dns", "wget", "curl", "mount", "df", "sync",
    "mkfs.bfs", "true", "false", "logout", "exit", "export", "env", "unset", "run",
    "source", "desktop", "gui", "ps", "kill", "free", "syscalls", "apps", "launch",
    "ifup", "arping", "ethinfo", "nslookup", "http",
];

/// Operateur reliant un segment de commande au precedent.
#[derive(Clone, Copy, PartialEq)]
enum Sep { Always, And, Or }

/// Point d'entree du shell : ecran de connexion, puis session, en boucle.
pub fn run() -> ! {
    loop {
        let uid = login_screen();
        users::session().set_uid(uid);
        dmesg::log("shell: session ouverte");
        session_loop();
        dmesg::log("shell: session fermee");
    }
}

/// Ecran de connexion : demande utilisateur + mot de passe jusqu'a reussite.
fn login_screen() -> u16 {
    let mut name_buf = [0u8; 64];
    let mut pass_buf = [0u8; 64];
    loop {
        vga::set_color(COLOR_CYAN);
        println!("");
        println!("=== Bouchaud OS - connexion ===");
        vga::set_color(COLOR_DEFAULT);
        print!("login: ");
        let nlen = keyboard::read_line(&mut name_buf);
        let name = trim(unsafe { core::str::from_utf8_unchecked(&name_buf[..nlen]) });
        if name.is_empty() { continue; }

        print!("Mot de passe: ");
        let plen = keyboard::read_secret(&mut pass_buf);
        println!("");
        let pass = unsafe { core::str::from_utf8_unchecked(&pass_buf[..plen]) };

        match users::authenticate(name, pass) {
            Some(uid) => {
                vga::set_color(COLOR_GREEN);
                println!("Bienvenue, {} !", name);
                vga::set_color(COLOR_DEFAULT);
                return uid;
            }
            None => {
                vga::set_color(COLOR_RED);
                println!("login: identifiants invalides");
                vga::set_color(COLOR_DEFAULT);
            }
        }
    }
}

/// Affiche l'invite de commande.
fn print_prompt(cwd: usize) {
    vga::set_color(COLOR_GREEN);
    print!("{}@bouchaud-os:", users::session().username());
    vga::set_color(COLOR_CYAN);
    ramfs::print_path(ramfs::fs(), cwd);
    vga::set_color(COLOR_GREEN);
    print!("$ ");
    vga::set_color(COLOR_DEFAULT);
}

/// Boucle de session : prompt + execution, jusqu'a `logout`/`exit`.
fn session_loop() {
    let mut cwd = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut line_buf = [0u8; 256];

    loop {
        print_prompt(cwd);
        let len = read_command(&mut line_buf, cwd);
        let line = unsafe { core::str::from_utf8_unchecked(&line_buf[..len]) };
        let trimmed = trim(line);
        if trimmed.is_empty() { continue; }

        history::record(trimmed);
        dmesg::log("shell: commande executee");

        if trimmed == "logout" || trimmed == "exit" {
            println!("Deconnexion.");
            return;
        }
        run_line(trimmed, &mut cwd);
    }
}

// ---------------------------------------------------------------------------
// Editeur de ligne : historique (fleches) + tab-completion
// ---------------------------------------------------------------------------

/// Lit une commande avec navigation dans l'historique et completion.
fn read_command(buf: &mut [u8], cwd: usize) -> usize {
    let mut len = 0usize;
    let mut hist: i32 = -1; // -1 = ligne en cours (pas de navigation)

    loop {
        match keyboard::read_key() {
            Key::Enter => { println!(""); return len; }
            Key::Backspace => { if len > 0 { len -= 1; print!("\x08"); } }
            Key::Char(c) => {
                if len < buf.len() { buf[len] = c; len += 1; print!("{}", c as char); }
            }
            Key::Up => {
                let n = history::len() as i32;
                if n > 0 && hist + 1 < n {
                    hist += 1;
                    if let Some(e) = history::nth_recent(hist as usize) { line_set(buf, &mut len, e); }
                }
            }
            Key::Down => {
                if hist > 0 {
                    hist -= 1;
                    if let Some(e) = history::nth_recent(hist as usize) { line_set(buf, &mut len, e); }
                } else if hist == 0 {
                    hist = -1;
                    line_set(buf, &mut len, "");
                }
            }
            Key::Tab => complete(buf, &mut len, cwd),
            _ => {}
        }
    }
}

/// Remplace le contenu affiche de la ligne par `text`.
fn line_set(buf: &mut [u8], len: &mut usize, text: &str) {
    for _ in 0..*len { print!("\x08"); }
    *len = 0;
    for &b in text.as_bytes() {
        if *len < buf.len() { buf[*len] = b; *len += 1; print!("{}", b as char); }
    }
}

fn append_str(buf: &mut [u8], len: &mut usize, s: &str) {
    for &b in s.as_bytes() {
        if *len < buf.len() { buf[*len] = b; *len += 1; print!("{}", b as char); }
    }
}

/// Tab-completion : commande (premier mot) ou chemin (mots suivants).
fn complete(buf: &mut [u8], len: &mut usize, cwd: usize) {
    let text = unsafe { core::str::from_utf8_unchecked(&buf[..*len]) };
    let mut tstart = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b' ' { tstart = i + 1; }
    }
    let prefix = &text[tstart..];

    let mut cands: Vec<String> = Vec::new();
    if tstart == 0 {
        for c in COMMANDS {
            if c.starts_with(prefix) { cands.push(String::from(*c)); }
        }
    } else {
        path_candidates(prefix, cwd, &mut cands);
    }
    if cands.is_empty() { return; }

    let lcp = longest_common_prefix(&cands);
    if lcp.len() > prefix.len() {
        let suffix = lcp[prefix.len()..].to_string();
        append_str(buf, len, &suffix);
    } else if cands.len() > 1 {
        println!("");
        for c in &cands { print!("{}  ", c); }
        println!("");
        print_prompt(cwd);
        let cur = unsafe { core::str::from_utf8_unchecked(&buf[..*len]) }.to_string();
        print!("{}", cur);
    }
}

fn path_candidates(prefix: &str, cwd: usize, out: &mut Vec<String>) {
    // Decoupe en partie repertoire + base a completer.
    let mut slash: Option<usize> = None;
    for (i, b) in prefix.bytes().enumerate() {
        if b == b'/' { slash = Some(i); }
    }
    let (dir, base, dir_prefix) = match slash {
        None => (Some(cwd), prefix, ""),
        Some(0) => (Some(0), &prefix[1..], &prefix[..1]),
        Some(p) => (ramfs::fs().resolve(&prefix[..p], cwd), &prefix[p + 1..], &prefix[..p + 1]),
    };
    let dir = match dir { Some(d) => d, None => return };
    let fs = ramfs::fs();
    for i in 0..ramfs::MAX_NODES {
        if fs.nodes[i].used && i != dir && fs.nodes[i].parent == dir {
            let name = fs.nodes[i].name_str();
            if name.starts_with(base) {
                let mut s = String::from(dir_prefix);
                s.push_str(name);
                out.push(s);
            }
        }
    }
}

fn longest_common_prefix(items: &[String]) -> String {
    if items.is_empty() { return String::new(); }
    let mut prefix = items[0].clone();
    for it in &items[1..] {
        let a = prefix.as_bytes();
        let b = it.as_bytes();
        let mut k = 0;
        while k < a.len() && k < b.len() && a[k] == b[k] { k += 1; }
        prefix.truncate(k);
    }
    prefix
}

// ---------------------------------------------------------------------------
// Execution : chainage ; && ||, redirections > >>, $?
// ---------------------------------------------------------------------------

/// Execute une ligne en capturant sa sortie texte (pour le terminal graphique).
pub fn run_capture(line: &str, cwd: &mut usize) -> String {
    vga::capture_start();
    run_line(trim(line), cwd);
    vga::capture_take().unwrap_or_default()
}

/// Decoupe et execute une ligne (chainage + redirections).
fn run_line(line: &str, cwd: &mut usize) {
    let bytes = line.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut sep = Sep::Always;

    while i < bytes.len() {
        if bytes[i] == b';' {
            run_chained(&line[start..i], sep, cwd);
            sep = Sep::Always; i += 1; start = i;
        } else if bytes[i] == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'&' {
            run_chained(&line[start..i], sep, cwd);
            sep = Sep::And; i += 2; start = i;
        } else if bytes[i] == b'|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            run_chained(&line[start..i], sep, cwd);
            sep = Sep::Or; i += 2; start = i;
        } else {
            i += 1;
        }
    }
    run_chained(&line[start..], sep, cwd);
}

fn run_chained(seg: &str, sep: Sep, cwd: &mut usize) {
    let seg = trim(seg);
    if seg.is_empty() { return; }
    let run = match sep {
        Sep::Always => true,
        Sep::And => last_status() == 0,
        Sep::Or => last_status() != 0,
    };
    if !run { return; }
    let code = run_segment(seg, cwd);
    set_status(code);
}

/// Execute un segment : gere `$?` et la redirection `>`/`>>`, puis dispatche.
fn run_segment(seg: &str, cwd: &mut usize) -> i32 {
    let expanded = expand_vars(seg);
    let (cmd, redir) = parse_redirect(&expanded);
    let cmd = trim(cmd);
    if cmd.is_empty() { return 0; }

    match redir {
        Some((path, append)) => {
            vga::capture_start();
            let code = run_pipeline(cmd, cwd);
            let out = vga::capture_take().unwrap_or_default();
            let rc = commands::redirect(trim(path), &out, append, *cwd);
            if rc != 0 { rc } else { code }
        }
        None => run_pipeline(cmd, cwd),
    }
}

/// Execute une commande en gerant les pipes `cmd1 | cmd2 | ...`.
///
/// La sortie de chaque etage devient l'entree standard (`stdin`) de la suivante.
fn run_pipeline(cmd: &str, cwd: &mut usize) -> i32 {
    // Decoupe sur les `|` simples (les `||` ont deja ete traites en amont).
    let mut stages: Vec<&str> = Vec::new();
    let b = cmd.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'|' { stages.push(&cmd[start..i]); i += 1; start = i; }
        else { i += 1; }
    }
    stages.push(&cmd[start..]);

    if stages.len() == 1 {
        return dispatch(trim(cmd), cwd);
    }

    let mut input: Option<String> = None;
    let mut code = 0;
    let n = stages.len();
    for (idx, stage) in stages.iter().enumerate() {
        let stage = trim(stage);
        let last = idx == n - 1;
        set_stdin(input.take());
        if !last { vga::capture_start(); }
        code = dispatch(stage, cwd);
        if !last { input = Some(vga::capture_take().unwrap_or_default()); }
        set_stdin(None);
    }
    code
}

/// Developpe `$?`, `$NOM` et `${NOM}` dans un segment de commande.
fn expand_vars(seg: &str) -> String {
    if !contains(seg, "$") { return String::from(seg); }
    use core::fmt::Write;
    let mut out = String::new();
    let b = seg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' { out.push(b[i] as char); i += 1; continue; }
        // $?
        if i + 1 < b.len() && b[i + 1] == b'?' {
            let _ = write!(out, "{}", last_status());
            i += 2;
            continue;
        }
        // ${NOM}
        if i + 1 < b.len() && b[i + 1] == b'{' {
            let mut j = i + 2;
            while j < b.len() && b[j] != b'}' { j += 1; }
            let name = &seg[i + 2..j];
            if let Some(v) = env_get(name) { out.push_str(&v); }
            i = if j < b.len() { j + 1 } else { j };
            continue;
        }
        // $NOM
        let mut j = i + 1;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') { j += 1; }
        if j > i + 1 {
            let name = &seg[i + 1..j];
            if let Some(v) = env_get(name) { out.push_str(&v); }
            i = j;
            continue;
        }
        out.push('$');
        i += 1;
    }
    out
}

/// Execute un script : chaque ligne non vide (et non commentaire `#`) est lancee.
fn run_script(argc: usize, argv: &[&str; 12], cwd: &mut usize) -> i32 {
    if argc < 2 { println!("usage: run <script>"); return 1; }
    let fs = ramfs::fs();
    let idx = match fs.resolve_checked(argv[1], *cwd) {
        Ok(i) => i,
        Err(e) => { println!("run: {}", e); return 1; }
    };
    if fs.nodes[idx].kind != crate::fs::ramfs::NodeKind::File { println!("run: pas un fichier"); return 1; }
    if !fs.can(idx, crate::fs::ramfs::PERM_R) { println!("run: permission denied"); return 1; }
    // Copie le contenu (la suite peut modifier le FS pendant l'execution).
    let mut content = String::new();
    for k in 0..fs.nodes[idx].content_len { content.push(fs.nodes[idx].content[k] as char); }
    for raw in content.lines() {
        let l = trim(raw);
        if l.is_empty() || l.starts_with('#') { continue; }
        run_line(l, cwd);
    }
    last_status()
}

fn contains(hay: &str, needle: &str) -> bool {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() { return false; }
    for w in 0..=h.len() - n.len() {
        if &h[w..w + n.len()] == n { return true; }
    }
    false
}

/// Separe une commande de sa redirection `>`/`>>` eventuelle.
fn parse_redirect(s: &str) -> (&str, Option<(&str, bool)>) {
    let bytes = s.as_bytes();
    // Cherche ">>" puis ">".
    for w in 0..bytes.len() {
        if bytes[w] == b'>' {
            if w + 1 < bytes.len() && bytes[w + 1] == b'>' {
                return (&s[..w], Some((&s[w + 2..], true)));
            }
            return (&s[..w], Some((&s[w + 1..], false)));
        }
    }
    (s, None)
}

/// Dispatcher : route le premier token vers la commande, renvoie un code retour.
fn dispatch(line: &str, cwd: &mut usize) -> i32 {
    let mut argv = [""; 12];
    let argc = tokenize(line, &mut argv);
    if argc == 0 { return 0; }

    use commands as c;
    match argv[0] {
        "true" => 0,
        "false" => 1,
        "logout" | "exit" => 0,

        // Environnement & scripts
        "export" => { env_export(line); 0 }
        "env" => { env_list(); 0 }
        "unset" => { if argc >= 2 { env_unset(argv[1]); } 0 }
        "run" | "source" => run_script(argc, &argv, cwd),

        // Aide et systeme
        "help" => { c::help(); 0 }
        "clear" => { vga::clear(); 0 }
        "desktop" | "gui" => { crate::gui::run(); vga::clear(); 0 }
        "version" => { c::version(); 0 }
        "uname" => { c::uname(); 0 }
        "sysinfo" => { c::sysinfo(); 0 }
        "cpuinfo" => { c::cpuinfo(); 0 }
        "meminfo" => { c::meminfo(); 0 }
        "alloctest" => { c::alloctest(); 0 }
        "devices" => { c::devices(); 0 }
        "dmesg" => { dmesg::print(); 0 }
        "history" => { c::history(argc, &argv); 0 }
        "uptime" => { c::uptime(); 0 }
        "ticks" => { c::ticks(); 0 }
        "interrupts" => { c::interrupts(); 0 }
        "ps" => { crate::kernel::process::print_table(); 0 }
        "kill" => {
            match argv.get(1).and_then(|s| s.parse::<u32>().ok()) {
                Some(pid) => {
                    if crate::kernel::process::kill(pid) { println!("kill: {} termine", pid); }
                    else { println!("kill: pid {} introuvable ou protege", pid); }
                }
                None => println!("usage: kill <pid>"),
            }
            0
        }
        "free" => { crate::kernel::memory::print_info(); 0 }
        "syscalls" => { crate::kernel::syscall::print_table(); 0 }
        "apps" => { crate::app::launcher::list(); 0 }
        "launch" => { if argc >= 2 { crate::app::launcher::launch(argv[1]); } else { println!("usage: launch <app>"); } 0 }
        "breakpoint" => { c::breakpoint(); 0 }
        "serial-test" => { c::serial_test(); 0 }
        "panic-test" => { c::panic_test(); 0 }
        "roadmap" => { c::roadmap(); 0 }

        // Sessions / utilisateurs
        "whoami" => { println!("{}", users::session().username()); 0 }
        "id" => { c::id(); 0 }
        "users" => { c::users(); 0 }
        "useradd" => { c::useradd(argc, &argv); 0 }
        "userdel" => { c::userdel(argc, &argv); 0 }
        "passwd" => { c::passwd(argc, &argv); 0 }
        "su" => { c::su(argc, &argv, cwd); 0 }

        // Fichiers
        "pwd" => { ramfs::print_path(ramfs::fs(), *cwd); println!(""); 0 }
        "ls" => c::ls(argc, &argv, *cwd),
        "tree" => { c::tree(argc, &argv, *cwd); 0 }
        "cd" => c::cd(argc, &argv, cwd),
        "mkdir" => c::mkdir(argc, &argv, *cwd),
        "touch" => c::touch(argc, &argv, *cwd),
        "cat" => c::cat(argc, &argv, *cwd),
        "write" => { c::write(line, argc, &argv, *cwd); 0 }
        "append" => { c::append(line, argc, &argv, *cwd); 0 }
        "nano" => { c::nano(argc, &argv, *cwd); 0 }
        "edit" => { if argc >= 2 { editor::edit(argv[1], *cwd); } else { println!("usage: edit <fichier>"); } 0 }
        "rm" => c::rm(argc, &argv, *cwd),
        "rmdir" => c::rmdir(argc, &argv, *cwd),
        "cp" => c::cp(argc, &argv, *cwd),
        "mv" => c::mv(argc, &argv, *cwd),
        "stat" => c::stat(argc, &argv, *cwd),
        "chmod" => c::chmod(argc, &argv, *cwd),
        "chown" => c::chown(argc, &argv, *cwd),
        "echo" => { println!("{}", remainder_after_tokens(line, 1)); 0 }
        "date" => { c::date(); 0 }
        "grep" => c::grep(argc, &argv, *cwd),
        "wc" => c::wc(argc, &argv, *cwd),
        "head" => c::head(argc, &argv, *cwd),
        "tail" => c::tail(argc, &argv, *cwd),
        "find" => { c::find(argc, &argv, *cwd); 0 }
        "lspci" => { crate::arch::x86_64::pci::print_devices(); 0 }

        // Reseau : loopback actif, eth0/Internet en attente du driver NIC.
        "ping" => { crate::net::ping(argc, &argv); 0 }
        "ifconfig" => { crate::net::ifconfig(); 0 }
        "ip" => { crate::net::ip_cmd(); 0 }
        "route" => { crate::net::route_cmd(); 0 }
        "arp" => { crate::net::arp_cmd(); 0 }
        "ifup" => { crate::net::ifup(); 0 }
        "arping" => { crate::net::arping(argc, &argv); 0 }
        "ethinfo" => { crate::drivers::e1000::print_info(); 0 }
        "dns" | "nslookup" => { crate::net::dns_cmd(argc, &argv); 0 }
        "wget" | "curl" | "http" => { crate::net::wget_cmd(argc, &argv); 0 }
        "dhcp" => { crate::net::placeholder(argv[0]); 0 }

        // Disque (placeholders, roadmap BFS)
        "df" => { crate::drivers::disk::print_df(); 0 }
        "mount" | "sync" | "mkfs.bfs" => { c::disk_placeholder(argv[0]); 0 }

        _ => {
            vga::set_color(COLOR_RED);
            println!("{}: commande inconnue", argv[0]);
            vga::set_color(COLOR_DEFAULT);
            127
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
