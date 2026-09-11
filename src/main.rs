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

#[cfg(feature = "legacy-boot")]
use bootloader::{entry_point, BootInfo as LegacyBootInfo};

#[cfg(all(feature = "legacy-boot", feature = "uefi-boot"))]
compile_error!("legacy-boot et uefi-boot sont mutuellement exclusifs");

#[cfg(not(any(feature = "legacy-boot", feature = "uefi-boot")))]
compile_error!("un chemin de boot doit etre selectionne");

#[cfg(all(feature = "reference-desktop", not(feature = "uefi-boot")))]
compile_error!("reference-desktop exige le chemin uefi-boot");

#[macro_use]
mod macros;

mod app;
mod arch;
mod boot;
mod diag;
mod drivers;
mod fs;
mod git;
mod gui;
mod kernel;
mod lang;
mod net;
mod platform;
mod shell;
mod users;
mod wasm;

/// Version courante de Bouchaud OS.
pub const VERSION: &str = "0.35.0";
/// Nom du systeme.
pub const OS_NAME: &str = "Bouchaud OS";

#[cfg(feature = "legacy-boot")]
entry_point!(legacy_boot_entry);

/// Frontiere temporaire du chargeur historique.
#[cfg(feature = "legacy-boot")]
fn legacy_boot_entry(legacy: &'static LegacyBootInfo) -> ! {
    let boot_info = boot::from_bootloader_09(legacy);
    kernel_main(boot_info)
}

#[cfg(feature = "uefi-boot")]
static UEFI_BOOTLOADER_CONFIG: bootloader_api::BootloaderConfig = {
    let mut config = bootloader_api::BootloaderConfig::new_default();
    config.mappings.physical_memory =
        Some(bootloader_api::config::Mapping::Dynamic);
    config
};

#[cfg(feature = "uefi-boot")]
bootloader_api::entry_point!(
    uefi_boot_entry,
    config = &UEFI_BOOTLOADER_CONFIG
);

#[cfg(feature = "uefi-boot")]
fn uefi_boot_entry(api: &'static mut bootloader_api::BootInfo) -> ! {
    let boot_info = boot::from_bootloader_api(api);
    kernel_main(boot_info)
}

/// Entree generique du noyau, independante du chargeur.
fn kernel_main(boot_info: &'static boot::BootInfo) -> ! {
    // 1. Sorties de base : serie d'abord (pour tracer le boot), puis VGA.
    drivers::serial::init();
    if boot_info.firmware == boot::FirmwareKind::Uefi {
        crate::serial_println!("BOUCHAUD_UEFI_ENTRY_OK");
    }
    if boot_info.firmware == boot::FirmwareKind::LegacyBios {
        drivers::vga::clear();
    }

    // L'ecran de faute AVANT l'horloge et le tas : une exception noyau
    // pendant l'initialisation doit s'afficher, pas laisser un ecran arrete.
    // Le chemin de faute n'alloue pas et ne prend aucun verrou ; il n'a besoin
    // que de l'adresse du framebuffer, que le chargeur a deja mappee.
    if let Some(fb) = boot_info.framebuffer {
        platform::pc::ecran_faute::installe_framebuffer(fb);
    }
    platform::pc::ecran_faute::point("kernel-main");

    // 2. Horloge, journal noyau, puis tas (alloc).
    kernel::timer::init();
    platform::pc::ecran_faute::point("horloge");
    kernel::dmesg::init();
    kernel::heap::init();
    let reference_bringup = platform::pc::bringup::enabled();
    if reference_bringup {
        platform::pc::bringup::announce(boot_info);
    }
    kernel::memory::init(boot_info);
    kernel::dmesg::log("kernel: boot Bouchaud OS");
    if boot_info.firmware == boot::FirmwareKind::LegacyBios {
        kernel::dmesg::log("vga: text mode initialise");
    } else {
        kernel::dmesg::log("boot: framebuffer firmware transmis");
    }
    kernel::dmesg::log("serial: COM1 initialise (debug QEMU)");

    // 3. Briques architecture. La pagination par processus doit etre prete
    //    avant `usermode::init` (appele par `arch::init`) : c'est elle qui
    //    fournit les frames et le creneau d'adressage du ring 3.
    kernel::vmm::init();

    // LA PAT ICI, ET PAS DANS `arch::init`.
    //
    // Sur la machine de reference -- UEFI, `reference-desktop` --, `stage2::run`
    // est appele quelques lignes plus bas et NE REND JAMAIS LA MAIN :
    // `arch::x86_64::init()` n'est donc jamais atteint. La configuration y
    // vivait, et le releve le disait sans que je le lise -- `coeurs_pat=0`,
    // `pat=0x0007040600070406`, soit la valeur de sortie d'usine.
    //
    // Elle vit desormais au seul endroit que TOUS les chemins traversent :
    // apres la pagination, qui est tout ce dont elle a besoin, et avant le
    // premier pixel.
    arch::x86_64::pat::configure_ce_coeur();

    // L'EXTINCTION SE PREPARE AU DEMARRAGE, PAS AU MOMENT DE COUPER.
    //
    // Chercher le RSDP, la FADT puis `\_S5_` alors que le systeme est deja en
    // train de s'arreter, c'est parcourir la memoire physique au pire moment.
    // Ici, la pagination est prete, rien n'a encore d'effet de bord, et le
    // releve de vol portera la preuve que la machine SAIT s'eteindre -- bien
    // avant qu'on le lui demande.
    kernel::acpi_s5::prepare(boot_info);

    // Stage 1 UEFI: preuve memoire + vraie ecriture framebuffer, toujours
    // AVANT GDT/IDT/PIC/PCI et avant tout pilote a effets de bord.
    if reference_bringup && boot_info.firmware == boot::FirmwareKind::Uefi {
        #[cfg(feature = "reference-desktop")]
        {
            platform::pc::stage2::run(boot_info);
        }
        #[cfg(not(feature = "reference-desktop"))]
        {
            platform::pc::bringup::complete_uefi_stage1_and_halt(boot_info);
        }
    }

    arch::x86_64::init();
    if reference_bringup {
        kernel::dmesg::log("bringup: SMP/AP startup volontairement ignore");
    } else {
        arch::x86_64::smp::init_probe();
    }

    // Calibre le TSC (cycles -> ms reels) maintenant que IRQ0 fait avancer les
    // ticks PIT : necessaire pour que les logs de diagnostic (reseau, layout,
    // peinture) affichent un temps reel exploitable, pas juste des "Mc" bruts.
    kernel::timer::calibrate();

    if reference_bringup {
        platform::pc::bringup::complete_legacy_foundation_and_halt(boot_info);
    }

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
    // La couche bloc generique, juste apres la detection : le systeme de
    // fichiers peut alors parler a un VOLUME plutot qu'a une nappe, et un
    // pilote NVMe s'ajoutera en s'enregistrant, sans qu'un appelant change.
    drivers::ata_bloc::installe();
    // LE DISQUE INTERNE NE DEPEND PAS DU MICROLOGICIEL QUI NOUS A DEMARRES.
    //
    // Le NVMe n'etait mis en service que sur le chemin UEFI du bureau de
    // reference. Un demarrage BIOS -- celui de TOUTES les campagnes QEMU --
    // n'initialisait donc jamais le pilote. Il n'etait execute que sur la
    // machine physique, ou il a double-faute : un pilote qui ne tourne que la
    // ou l'on ne peut pas l'observer n'a pas de preuve, il a des temoignages.
    //
    // Le montage de la partition est DEMANDE, pas fait : il part dans son
    // propre fil une fois le bureau peint. Voir `installation::differe_le_montage`.
    if drivers::nvme::bring_up() {
        platform::pc::installation::differe_le_montage();
    }
    fs::tar::mount_data_disk();
    // Ce que la machine a retenu du demarrage precedent. Vient apres l'archive :
    // un fichier persistant doit pouvoir remplacer celui que l'archive depose,
    // pas l'inverse.
    fs::persistance::monte();
    // A partir d'ici le tas, le RAMFS et le timer sont prets : le journal peut
    // mesurer la charge de la machine sans risquer d'echouer dans la ligne meme
    // qui devait l'expliquer.
    kernel::journal::demarre();
    kernel::process::init();
    kernel::dmesg::log("process: table initialisee (init, desktop, shell)");
    kernel::dmesg::log("abi: appels systeme POSIX/Linux x86-64 disponibles (ring 3)");
    // Le reseau est mis en service ici, pas laisse a une commande. Le driver
    // e1000 existait et marchait, mais il fallait taper `ifup` puis `dhcp`
    // avant de pouvoir ouvrir une page : un systeme dont l'interface graphique
    // demarre doit avoir son reseau en service, comme il a son clavier.
    // `net::demarre` n'echoue jamais — voir sa documentation.
    net::demarre();
    kernel::dmesg::log("shell: initialise");

    // 5. Banniere d'accueil.
    banner();

    // Le BSP a fini toute l'initialisation materielle et systeme. Les AP, deja
    // en long mode, peuvent maintenant entrer dans la runqueue SMP. Tant
    // qu'aucune tache n'est enregistree ils restent en HLT et ne touchent
    // ni le tas ni les structures historiques.
    arch::x86_64::smp::enable_scheduler();

    // LA PERSISTANCE NE DEPEND PAS DU BUREAU GRAPHIQUE.
    //
    // Le montage differe n'avait qu'un seul declencheur : la troisieme trame
    // du compositeur. Un demarrage sans bureau -- celui du shell interactif,
    // celui de toutes les campagnes QEMU en mode serie -- armait donc le
    // drapeau et ne le consommait JAMAIS. Le journal disait
    // `BOUCHAUD_NVME_PERSISTENCE_DEFERRED`, puis plus rien, indefiniment.
    //
    // Le defaut n'est pas seulement un scenario de campagne muet : sur une
    // machine ou le bureau ne demarre pas, la persistance restait absente
    // sans qu'aucune ligne ne dise pourquoi.
    //
    // Le lancement appartient donc a l'amorcage, ici, des que l'ordonnanceur
    // peut faire tourner une tache -- et pas une trame plus tard. L'appel du
    // compositeur reste en place : il ne fait plus rien quand celui-ci a deja
    // eu lieu (`montage_differe_en_attente`), et rattrape le cas ou la tache
    // n'a pas pu etre creee a cet instant precis.
    platform::pc::installation::lance_le_montage_differe();
    // Voir la note d'amorcage equivalente dans `stage2.rs` : l'entree ne doit
    // pas dependre de la cadence du rendu.
    drivers::xhci_active::demarre_le_fil_hid();

    // L'enregistreur de vol part avec son propre fil : il ecrit sur la cle
    // USB par transferts synchrones, et ce cout n'a rien a faire sur le
    // chemin de l'entree.
    drivers::xhci_active::demarre_le_fil_blackbox();

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
