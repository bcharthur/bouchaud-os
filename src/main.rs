//! Bouchaud OS — noyau experimental from scratch en Rust `no_std`.
//!
//! Point d'entree du noyau. La logique est decoupee en modules :
//!   - `arch`    : code dependant de l'architecture (x86_64 : ports, CPU, GDT/IDT) ;
//!   - `drivers` : pilotes materiels (VGA, serie COM1, clavier AZERTY-FR) ;
//!   - `fs`      : systeme de fichiers (RAMFS) ;
//!   - `kernel`  : coeur du noyau (dmesg, timer, panic) ;
//!   - `users`   : modele utilisateur et sessions ;
//!   - `shell`   : shell interactif Unix-like ;
//!   - `net`     : feuille de route reseau (non activee).
//!
//! Objectif long terme : un OS souverain francais experimental, Unix-like,
//! pedagogique et extensible.

#![no_std]
#![no_main]
#![allow(dead_code)]
#![allow(static_mut_refs)]
#![feature(abi_x86_interrupt)]

extern crate alloc;

use bootloader::{entry_point, BootInfo};

#[macro_use]
mod macros;

mod app;
mod arch;
mod diag;
mod drivers;
mod fs;
mod git;
mod gui;
mod kernel;
mod lang;
mod net;
mod shell;
mod users;
mod wasm;

/// Version courante de Bouchaud OS.
pub const VERSION: &str = "0.35.0";
/// Nom du systeme.
pub const OS_NAME: &str = "Bouchaud OS";

entry_point!(kernel_main);

/// Point d'entree appele par le bootloader une fois en long mode 64 bits.
fn kernel_main(boot_info: &'static BootInfo) -> ! {
    // 1. Sorties de base : serie d'abord (pour tracer le boot), puis VGA.
    drivers::serial::init();
    drivers::vga::clear();

    // 2. Horloge, journal noyau, puis tas (alloc).
    kernel::timer::init();
    kernel::dmesg::init();
    kernel::heap::init();
    kernel::memory::init(boot_info);
    kernel::dmesg::log("kernel: boot Bouchaud OS");
    kernel::dmesg::log("vga: text mode initialise");
    kernel::dmesg::log("serial: COM1 initialise (debug QEMU)");

    // 3. Briques architecture. La pagination par processus doit etre prete
    //    avant `usermode::init` (appele par `arch::init`) : c'est elle qui
    //    fournit les frames et le creneau d'adressage du ring 3.
    kernel::vmm::init();
    arch::x86_64::init();

    // Calibre le TSC (cycles -> ms reels) maintenant que IRQ0 fait avancer les
    // ticks PIT : necessaire pour que les logs de diagnostic (reseau, layout,
    // peinture) affichent un temps reel exploitable, pas juste des "Mc" bruts.
    kernel::timer::calibrate();

    // 4. Pilotes et sous-systemes.
    drivers::keyboard::init();
    kernel::dmesg::log("keyboard: PS/2 AZERTY-FR pilote par IRQ1");
    users::init();
    kernel::dmesg::log("users: base initialisee (root, guest)");
    fs::ramfs::fs().init();
    users::create_home_dirs();
    kernel::dmesg::log("ramfs: monte sur /");
    lang::pyweb::install();
    kernel::dmesg::log("pyweb: /dev/web pret, browser.py installe");
    kernel::sysroot::install();
    // Archive userland du second disque : c'est ce qui permet d'installer un
    // programme sans reconstruire le noyau. Le pilote est sonde juste avant,
    // pour que le journal montre les disques avant ce qu'on en a tire.
    drivers::ata::probe();
    fs::tar::mount_data_disk();
    kernel::process::init();
    kernel::dmesg::log("process: table initialisee (init, desktop, shell)");
    kernel::dmesg::log("abi: appels systeme POSIX/Linux x86-64 disponibles (ring 3)");
    kernel::dmesg::log("net: loopback lo 127.0.0.1 actif (ping ok); eth0 sans driver");
    kernel::dmesg::log("shell: initialise");

    // 5. Banniere d'accueil.
    banner();

    // 6. Mode non interactif : si le disque de donnees a depose un `/autorun`,
    //    on le joue et la machine s'eteint. Ne rend la main que sans script.
    kernel::autorun::run_if_present();

    // 7. Boucle interactive.
    shell::run();
}

/// Affiche la banniere d'accueil de Bouchaud OS.
fn banner() {
    use drivers::vga::{self, COLOR_CYAN, COLOR_DEFAULT};
    vga::set_color(COLOR_CYAN);
    println!("Bouchaud OS");
    vga::set_color(COLOR_DEFAULT);
    println!("Version: {} - kernel foundation", VERSION);
    println!("Clavier: AZERTY-FR");
    println!("Shell: Unix-like CLI");
    println!("FS: RAMFS");
    println!("Serial: COM1 debug enabled");
    println!("Objectif: OS souverain francais experimental");
    println!("");
}
