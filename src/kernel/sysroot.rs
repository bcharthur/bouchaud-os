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
use alloc::string::String;

use crate::fs::ramfs;

/// Polices deposees dans `/usr/share/fonts`.
///
/// La romaine est celle deja embarquee pour le moteur de rendu du noyau : on
/// la reference au lieu de l'inclure une seconde fois, ce qui eviterait
/// d'alourdir l'image de boot de 750 Kio pour rien.


/// Cree (ou retrouve) un repertoire par son chemin absolu.
fn mkdir_path(path: &str) -> usize {
    let mut fs = ramfs::fs();
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
    let mut fs = ramfs::fs();
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
    let mut fs = ramfs::fs();
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

    let fixtures = mkdir_path("/usr/share/bouchaud");
    write_file(fixtures, "scroll-test.html", include_str!("scroll-test.html"), 0o444);

    install_fonts();
    install_proc();
    install_sys();
    install_etc();

    crate::kernel::dmesg::log("sysroot: fonts, fixtures, /proc, /sys et /etc installes");
}

/// Depose les polices DejaVu dans `/usr/share/fonts`.
///
/// Sans au moins une police lisible, une interface graphique demarre mais
/// n'affiche aucun texte — panne d'autant plus deroutante qu'elle ne produit
/// aucune erreur.
fn install_fonts() {
    // Le manifeste est la seule liste. `DejaVuSansMono-Bold.ttf` etait embarque
    // dans le binaire et n'arrivait jamais ici : une graisse manquante ne
    // produit pas d'erreur, seulement du texte qui tombe sur une autre police.
    let dir = mkdir_path(crate::gui::polices::REPERTOIRE);
    if dir == 0 {
        return;
    }
    for police in crate::gui::polices::manifeste().iter() {
        write_binary(dir, police.fichier, police.octets, 0o644);
    }
}

/// Fils logiques par coeur physique sur les machines visees.
///
/// Deux : la TRIGKEY porte un Ryzen 7 5700U, huit coeurs et seize fils. Ce
/// n'est pas lu du materiel, parce que rien dans ce noyau ne l'expose encore.
/// C'est donc une HYPOTHESE, et il faut savoir ce qu'elle influe : `cpu cores`
/// et `core id` de `/proc/cpuinfo`, et rien d'autre. Le NOMBRE de processeurs
/// annonce, lui, est mesure.
const FILS_PAR_COEUR: usize = 2;

/// Le nombre de processeurs a annoncer au monde utilisateur.
///
/// Une seule fonction pour `/proc/cpuinfo`, `/proc/stat` et les trois fichiers
/// de `/sys/devices/system/cpu` : deux calculs pourraient diverger, et un
/// `online` qui ne s'accorde pas avec le nombre de lignes de `cpuinfo` est
/// exactement le genre d'incoherence qui fait choisir a une bibliotheque la
/// plus petite des deux.
fn processeurs_annonces() -> usize {
    crate::kernel::cpu_topologie::annonces(
        crate::arch::x86_64::smp::schedulable_cpus(),
        crate::arch::x86_64::smp::MAX_CPUS,
    )
}

/// `/proc` reduit aux fichiers lus au demarrage d'un programme.
fn install_proc() {
    let proc = mkdir_path("/proc");
    if proc == 0 {
        return;
    }
    let logiques = processeurs_annonces();

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

    // BOUCHAUD_C24_TOPOLOGIE_CPU
    //
    // UN BLOC PAR PROCESSEUR, et `siblings`/`cpu cores` renseignes.
    //
    // Ce fichier n'en portait qu'un seul, avec `siblings: 1` et
    // `cpu cores: 1`. Les bibliotheques qui comptent les lignes
    // `processor` -- et il y en a beaucoup, parce que c'est la methode
    // portable -- concluaient qu'il y avait un processeur, sur une machine
    // qui en ordonnance seize. Chaque pool de threads du navigateur etait
    // alors cree avec un seul fil.
    let vendor = crate::arch::x86_64::cpu::vendor();
    let vendor = core::str::from_utf8(&vendor).unwrap_or("unknown");
    let mut cpuinfo = String::new();
    let mut index = 0usize;
    while let Some(bloc) = crate::kernel::cpu_topologie::bloc(logiques, FILS_PAR_COEUR, index) {
        cpuinfo.push_str(&format!(
            "processor\t: {}\nvendor_id\t: {}\ncpu family\t: 6\nmodel name\t: Bouchaud OS virtual CPU\ncpu MHz\t\t: 1000.000\ncache size\t: 0 KB\nphysical id\t: 0\nsiblings\t: {}\ncore id\t\t: {}\ncpu cores\t: {}\nflags\t\t: fpu tsc msr pae cx8 sep cmov pat mmx fxsr sse sse2\n\n",
            bloc.processeur, vendor, bloc.siblings, bloc.core_id, bloc.coeurs,
        ));
        index += 1;
    }
    write_file(proc, "cpuinfo", &cpuinfo, 0o444);

    // `/proc/stat` : refuse sur la machine physique parce qu'il n'existait
    // pas. Beaucoup de bibliotheques le lisent pour compter les processeurs
    // quand `/sys` leur est ferme, et certaines pour mesurer la charge.
    //
    // Les compteurs de temps sont a ZERO, et c'est deliberé : le noyau ne
    // tient pas de comptabilite par processeur au format `jiffies`, et
    // inventer des nombres ferait calculer a un moniteur des pourcentages
    // faux. Zero se lit « pas de temps ecoule », ce qui est visiblement faux
    // et donc lisible comme une absence de mesure ; un nombre plausible ne le
    // serait pas. Le nombre de LIGNES, lui, est exact -- et c'est ce que les
    // compteurs de processeurs viennent chercher ici.
    let mut stat = String::from("cpu  0 0 0 0 0 0 0 0 0 0\n");
    for cpu in 0..logiques {
        stat.push_str(&format!("cpu{}  0 0 0 0 0 0 0 0 0 0\n", cpu));
    }
    stat.push_str(&format!("ctxt 0\nbtime 0\nprocesses 0\nprocs_running 1\nprocs_blocked 0\n"));
    write_file(proc, "stat", &stat, 0o444);

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
        //
        // BOUCHAUD_C24_TOPOLOGIE_CPU
        //
        // Ces trois fichiers disaient « 0 ». Ce n'est pas un compte, c'est
        // une PLAGE, et celle-la ne contient que le processeur zero : la
        // glibc en concluait UN processeur sur une machine qui en ordonnance
        // seize, et tous les pools de threads du navigateur naissaient a un
        // fil. Le releve physique le decrivait sans le nommer -- « la charge
        // WebContent peut etre tres elevee sur un seul coeur alors que le CPU
        // global parait faible ».
        let logiques = processeurs_annonces();
        let mut tampon = [0u8; 32];
        let n = crate::kernel::cpu_topologie::plage(logiques, &mut tampon);
        let plage = core::str::from_utf8(&tampon[..n]).unwrap_or("0");
        let ligne = format!("{}\n", plage);
        write_file(cpu, "online", &ligne, 0o444);
        write_file(cpu, "possible", &ligne, 0o444);
        write_file(cpu, "present", &ligne, 0o444);
        // Un repertoire par processeur : c'est ce que comptent les
        // bibliotheques qui enumerent `/sys/devices/system/cpu/cpu*` plutot
        // que de lire `online`.
        for index in 0..logiques {
            mkdir_path(&format!("/sys/devices/system/cpu/cpu{}", index));
        }
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

/// Publie le DNS courant apres DHCP, y compris si le navigateur tourne deja.
/// Une configuration physique absente ne doit pas exposer le DNS de QEMU.
pub fn refresh_resolver() {
    let etc = mkdir_path("/etc");
    if etc == 0 { return; }
    let content = if matches!(crate::net::etat_demarrage(),
        crate::net::Demarrage::Pret | crate::net::Demarrage::SansBail) {
        let dns = crate::net::dns_server();
        format!("nameserver {}.{}.{}.{}\n", dns[0], dns[1], dns[2], dns[3])
    } else {
        format!("# Bouchaud: network not configured\n")
    };
    write_file(etc, "resolv.conf", &content, 0o644);
}

/// `/etc` : les fichiers que la libc consulte pour resoudre noms et locales.
fn install_etc() {
    let etc = mkdir_path("/etc");
    if etc == 0 {
        return;
    }
    write_file(etc, "hostname", "bouchaud\n", 0o644);
    write_file(etc, "hosts", "127.0.0.1\tlocalhost bouchaud\n", 0o644);
    refresh_resolver();
    write_file(etc, "passwd", "root:x:0:0:root:/:/bin/sh\nguest:x:1000:1000:guest:/home/guest:/bin/sh\n", 0o644);
    write_file(etc, "group", "root:x:0:\nguest:x:1000:\n", 0o644);
    write_file(etc, "localtime", "", 0o644);
    // Chemins de recherche de `ld.so` pour les binaires dynamiques.
    write_file(etc, "ld-musl-x86_64.path", "/lib\n/usr/lib\n/usr/local/lib\n", 0o644);
    write_file(etc, "ld.so.conf", "/lib\n/usr/lib\n/usr/local/lib\n", 0o644);
}
