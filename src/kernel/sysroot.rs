//! Arborescence systeme attendue par un programme Linux.
//!
//! Une libc et une boite a outils graphique ne se contentent pas d'appels
//! systeme : elles lisent des fichiers, et se comportent mal quand ils
//! manquent. `sysconf(_SC_NPROCESSORS_ONLN)` passe par
//! `/sys/devices/system/cpu/online`, Qt cherche des polices dans un repertoire
//! avant de renoncer a dessiner du texte, une pile de threads consulte
//! `/proc/meminfo` pour dimensionner ses caches.
//!
//! Ce module depose donc au boot, dans le RAMFS :
//!
//! - `/usr/share/fonts` : les polices DejaVu deja embarquees dans le noyau,
//!   celles-la memes que le moteur de rendu maison utilise ;
//! - un `/proc` et un `/sys` synthetiques, reduits aux fichiers reellement
//!   interroges au demarrage d'un programme ;
//! - les repertoires standards (`/bin`, `/lib`, `/tmp`, `/etc`...) que le
//!   chargeur dynamique et les chemins par defaut supposent presents.
//!
//! Ce sont des instantanes, pas des vues vivantes : `/proc/meminfo` reflete
//! l'etat du boot. Les valeurs qui doivent etre exactes a l'instant T (heure,
//! memoire libre) sont fournies par des appels systeme (`sysinfo`,
//! `clock_gettime`), pas par ces fichiers.

use alloc::format;

use crate::fs::ramfs;

/// Polices deposees dans `/usr/share/fonts`.
///
/// La romaine est celle deja embarquee pour le moteur de rendu du noyau : on
/// la reference au lieu de l'inclure une seconde fois, ce qui eviterait
/// d'alourdir l'image de boot de 750 Kio pour rien.
fn fonts() -> [(&'static str, &'static [u8]); 3] {
    [
        ("DejaVuSans.ttf", crate::gui::font::FONT_DATA),
        ("DejaVuSans-Bold.ttf", include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf")),
        ("DejaVuSansMono.ttf", include_bytes!("../assets/fonts/DejaVuSansMono.ttf")),
    ]
}

/// Cree (ou retrouve) un repertoire par son chemin absolu.
fn mkdir_path(path: &str) -> usize {
    let fs = ramfs::fs();
    let mut current = 0usize;
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        current = match fs.find_child(current, segment) {
            Some(existing) => existing,
            None => match fs.mkdir_at(current, segment) {
                Ok(created) => created,
                Err(_) => return 0,
            },
        };
    }
    current
}

/// Ecrit un fichier texte dans un repertoire deja cree.
fn write_file(parent: usize, name: &str, content: &str, mode: u16) {
    if parent == 0 && name.is_empty() {
        return;
    }
    let fs = ramfs::fs();
    let node = match fs.find_child(parent, name) {
        Some(existing) => existing,
        None => match fs.touch_at(parent, name) {
            Ok(created) => created,
            Err(_) => return,
        },
    };
    fs.write_node(node, content);
    fs.nodes[node].mode = mode;
}

/// Ecrit un fichier binaire (police).
fn write_binary(parent: usize, name: &str, content: &[u8], mode: u16) {
    let fs = ramfs::fs();
    let node = match fs.find_child(parent, name) {
        Some(existing) => existing,
        None => match fs.touch_at(parent, name) {
            Ok(created) => created,
            Err(_) => return,
        },
    };
    fs.write_node_bytes(node, content);
    fs.nodes[node].mode = mode;
}

/// Installe l'arborescence systeme. Idempotent.
pub fn install() {
    // Repertoires standards : `ld.so` cherche les bibliotheques dans /lib et
    // /usr/lib, la libc ecrit ses fichiers temporaires dans /tmp, et le PATH
    // par defaut pointe sur /bin et /usr/bin.
    for path in ["/bin", "/lib", "/tmp", "/etc", "/usr/bin", "/usr/lib", "/usr/local/bin", "/var/run"] {
        mkdir_path(path);
    }

    install_fonts();
    install_proc();
    install_sys();
    install_etc();

    crate::kernel::dmesg::log("sysroot: /usr/share/fonts, /proc, /sys et /etc installes");
}

/// Depose les polices DejaVu dans `/usr/share/fonts`.
///
/// Sans au moins une police lisible, une interface graphique demarre mais
/// n'affiche aucun texte — panne d'autant plus deroutante qu'elle ne produit
/// aucune erreur.
fn install_fonts() {
    let dir = mkdir_path("/usr/share/fonts/truetype/dejavu");
    if dir == 0 {
        return;
    }
    for (name, data) in fonts().iter() {
        write_binary(dir, name, data, 0o644);
    }
}

/// `/proc` reduit aux fichiers lus au demarrage d'un programme.
fn install_proc() {
    let proc = mkdir_path("/proc");
    if proc == 0 {
        return;
    }

    let (_, free_frames, total_frames) = crate::kernel::vmm::frame_stats();
    let total_kb = total_frames * 4;
    let free_kb = free_frames * 4;
    write_file(
        proc,
        "meminfo",
        &format!(
            "MemTotal:       {:>8} kB\nMemFree:        {:>8} kB\nMemAvailable:   {:>8} kB\nSwapTotal:             0 kB\nSwapFree:              0 kB\n",
            total_kb, free_kb, free_kb
        ),
        0o444,
    );

    let vendor = crate::arch::x86_64::cpu::vendor();
    let vendor = core::str::from_utf8(&vendor).unwrap_or("unknown");
    write_file(
        proc,
        "cpuinfo",
        &format!(
            "processor\t: 0\nvendor_id\t: {}\ncpu family\t: 6\nmodel name\t: Bouchaud OS virtual CPU\ncpu MHz\t\t: 1000.000\ncache size\t: 0 KB\nsiblings\t: 1\ncpu cores\t: 1\nflags\t\t: fpu tsc msr pae cx8 sep cmov pat mmx fxsr sse sse2\n\n",
            vendor
        ),
        0o444,
    );

    write_file(proc, "uptime", &format!("{}.00 {}.00\n", crate::kernel::timer::seconds(), crate::kernel::timer::seconds()), 0o444);
    write_file(proc, "version", &format!("Linux version 6.1.0-bouchaud (Bouchaud OS {})\n", crate::VERSION), 0o444);
    write_file(proc, "filesystems", "nodev\tramfs\n", 0o444);
    // `/proc/sys/vm/overcommit_memory` : consulte par certains allocateurs.
    let vm = mkdir_path("/proc/sys/vm");
    if vm != 0 {
        write_file(vm, "overcommit_memory", "0\n", 0o444);
        write_file(vm, "max_map_count", "65530\n", 0o444);
    }
    let kernel = mkdir_path("/proc/sys/kernel");
    if kernel != 0 {
        write_file(kernel, "osrelease", "6.1.0-bouchaud\n", 0o444);
        write_file(kernel, "random_uuid", "00000000-0000-4000-8000-000000000000\n", 0o444);
    }
    // `/proc/self` : seul `exe` est resolu, par `readlink` (voir abi::file).
    mkdir_path("/proc/self");
}

/// `/sys` reduit a la description du CPU et de l'ecran.
fn install_sys() {
    let cpu = mkdir_path("/sys/devices/system/cpu");
    if cpu != 0 {
        // C'est ici que passe `sysconf(_SC_NPROCESSORS_ONLN)` de la glibc :
        // un fichier absent la fait retomber sur des heuristiques, un fichier
        // present et coherent evite toute surprise.
        write_file(cpu, "online", "0\n", 0o444);
        write_file(cpu, "possible", "0\n", 0o444);
        write_file(cpu, "present", "0\n", 0o444);
        mkdir_path("/sys/devices/system/cpu/cpu0");
    }

    let fb = mkdir_path("/sys/class/graphics/fb0");
    if fb != 0 {
        let (width, height) = crate::drivers::gfx::resolution();
        write_file(fb, "name", "bouchaudfb\n", 0o444);
        write_file(fb, "virtual_size", &format!("{},{}\n", width, height), 0o444);
        write_file(fb, "bits_per_pixel", "32\n", 0o444);
        write_file(fb, "stride", &format!("{}\n", width * 4), 0o444);
    }
    mkdir_path("/sys/class/input");
}

/// `/etc` : les fichiers que la libc consulte pour resoudre noms et locales.
fn install_etc() {
    let etc = mkdir_path("/etc");
    if etc == 0 {
        return;
    }
    write_file(etc, "hostname", "bouchaud\n", 0o644);
    write_file(etc, "hosts", "127.0.0.1\tlocalhost bouchaud\n", 0o644);
    // Le serveur reellement configure, et non une adresse en dur : une libc
    // fait sa resolution elle-meme, par ce fichier.
    let dns = crate::net::dns_server();
    write_file(etc, "resolv.conf", &format!("nameserver {}.{}.{}.{}\n", dns[0], dns[1], dns[2], dns[3]), 0o644);
    write_file(etc, "passwd", "root:x:0:0:root:/:/bin/sh\nguest:x:1000:1000:guest:/home/guest:/bin/sh\n", 0o644);
    write_file(etc, "group", "root:x:0:\nguest:x:1000:\n", 0o644);
    write_file(etc, "localtime", "", 0o644);
    // Chemins de recherche de `ld.so` pour les binaires dynamiques.
    write_file(etc, "ld-musl-x86_64.path", "/lib\n/usr/lib\n/usr/local/lib\n", 0o644);
    write_file(etc, "ld.so.conf", "/lib\n/usr/lib\n/usr/local/lib\n", 0o644);
}
