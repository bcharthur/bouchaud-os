//! Stage 2 - vrai bureau Bouchaud interactif sur GOP UEFI.
//!
//! Runtime volontairement RAM-only et BSP-only.

use crate::boot::{BootInfo, FirmwareKind};

fn prepare_ram_persist() -> bool {
    let mut fs = crate::fs::ramfs::fs();
    let root = 0usize;
    let persist = match fs.find_child(root, "persist") {
        Some(node) => node,
        None => match fs.mkdir_at(root, "persist") {
            Ok(node) => node,
            Err(_) => return false,
        },
    };
    let ladybird = match fs.find_child(persist, "ladybird") {
        Some(node) => node,
        None => match fs.mkdir_at(persist, "ladybird") {
            Ok(node) => node,
            Err(_) => return false,
        },
    };
    if fs.find_child(ladybird, "profile").is_none()
        && fs.mkdir_at(ladybird, "profile").is_err()
    {
        return false;
    }
    if fs.find_child(persist, "Downloads").is_none()
        && fs.mkdir_at(persist, "Downloads").is_err()
    {
        return false;
    }
    true
}

pub fn run(boot: &'static BootInfo) -> ! {
    if boot.firmware != FirmwareKind::Uefi {
        panic!("stage2: firmware UEFI requis");
    }

    let framebuffer = super::bringup::validate_uefi_stage1(boot);
    if !crate::drivers::gfx::install_firmware_framebuffer(framebuffer) {
        panic!("stage2: framebuffer GOP incompatible avec le bureau");
    }

    let (physical_w, physical_h) = crate::drivers::gfx::firmware_resolution()
        .expect("stage2: viewport GOP absent");
    let (scale_x, scale_y) = crate::drivers::gfx::firmware_scale_milli()
        .unwrap_or((1000, 1000));

    crate::serial_println!(
        "[STAGE2] viewport physical={}x{} logical={}x{} scale={}milli x {}milli",
        physical_w, physical_h,
        crate::drivers::gfx::width(), crate::drivers::gfx::height(),
        scale_x, scale_y,
    );
    if (physical_w, physical_h)
        != (crate::drivers::gfx::width(), crate::drivers::gfx::height())
        || (scale_x, scale_y) != (1000, 1000)
    {
        panic!("stage2: viewport GOP non natif");
    }
    crate::serial_println!("BOUCHAUD_STAGE2_NATIVE_VIEWPORT_OK");
    crate::serial_println!(
        "[STAGE2] smp=off audio=off storage=read-mostly network=auto pci=scan"
    );

    // CPU0 uniquement. Aucun AP n'est demarre.
    let bsp = crate::arch::x86_64::cpu_local::register_bsp();
    crate::arch::x86_64::cpu_local::mark_online(bsp, true);

    // Sous-ensemble CPU de arch::init(), sans pci::init().
    crate::arch::x86_64::gdt::init();
    crate::arch::x86_64::idt::init();
    crate::arch::x86_64::interrupts::init();
    crate::arch::x86_64::usermode::init();
    crate::kernel::timer::calibrate();

    crate::drivers::keyboard::init();

    // Base userspace du vrai window manager et de Ladybird.
    crate::users::init();
    crate::fs::ramfs::fs().init();
    crate::users::create_home_dirs();

    // Arborescence POSIX minimale avant l'archive Ladybird : le disque de
    // donnees peut ensuite remplacer/ajouter ses fichiers sans perdre /proc,
    // /sys, /tmp et les polices systeme.
    crate::kernel::sysroot::install();

    // Chemin physique TRIGKEY: l'archive Ladybird est dans la MEME image UEFI.
    // Aucun acces au NVMe interne n'est necessaire. L'ancien hdb ATA reste un
    // fallback QEMU pour conserver la regression Stage 2 FINAL V2.
    // `persistance::monte()` rend un NOMBRE de fichiers restaures (usize),
    // pas un booleen. Le chemin ramdisk n'a aucune zone persistante disque a
    // restaurer : son compteur est donc explicitement 0.
    let (data_source, persist_restored): (&str, usize) =
        if let Some(ramdisk) = boot.ramdisk {
            if !crate::fs::tar::mount_boot_ramdisk(ramdisk.address, ramdisk.byte_len) {
                panic!("stage2: ramdisk Ladybird invalide");
            }
            if !prepare_ram_persist() {
                panic!("stage2: impossible de preparer /persist RAM-only");
            }
            crate::serial_println!(
                "BOUCHAUD_TRIGKEY_LADYBIRD_RAMDISK_OK bytes={}", ramdisk.byte_len
            );
            ("uefi-ramdisk", 0usize)
        } else {
            crate::drivers::ata::probe();
            crate::drivers::ata_bloc::installe();
            crate::fs::tar::mount_data_disk();
            ("ata-hdb", crate::fs::persistance::monte())
        };

    crate::kernel::journal::demarre();
    crate::kernel::process::init();

    // PCI est maintenant autorise au Stage 2 FINAL V2 pour une seule raison :
    // rendre le navigateur vraiment utilisable. `net::demarre` ne charge le
    // driver e1000 que sur une carte Intel ; un Realtek physique est refuse
    // proprement par le driver e1000.
    crate::arch::x86_64::pci::init();
    if let Some(xhci) = crate::arch::x86_64::pci::find_xhci() {
        crate::serial_println!(
            "BOUCHAUD_TRIGKEY_XHCI_PRESENT pci={:04x}:{:04x} bus={:02x}:{:02x}.{} bar0={:#x}",
            xhci.vendor, xhci.device, xhci.bus, xhci.slot, xhci.func,
            crate::arch::x86_64::pci::bar_decode(&xhci, 0).adresse(),
        );
    } else {
        crate::serial_println!("BOUCHAUD_TRIGKEY_XHCI_ABSENT");
    }
    let _network_state = crate::net::demarre();

    // Le run historique posait ces variables via /autorun. Le Stage 2 entre
    // directement dans le bureau, donc il doit fournir le meme contrat avant
    // que l'utilisateur double-clique sur Ladybird.
    crate::shell::set_exported_for_boot("BOUCHAUD_M9", "1");
    crate::shell::set_exported_for_boot("BOUCHAUD_M9_URL", "https://example.com/");
    crate::shell::set_exported_for_boot("BOUCHAUD_M11", "1");
    crate::shell::set_exported_for_boot("BOUCHAUD_BROWSER_HOST", "1");
    crate::shell::set_exported_for_boot("BOUCHAUD_TIME_ZONE", "Europe/Paris");
    crate::shell::set_exported_for_boot("BOUCHAUD_ALLOW_POPUPS", "1");

    // Stage 2 single-USB : aucun stockage persistant writable n'est requis.
    // Le BrowserHost platform-complete sait basculer nativement vers son
    // profil/cache RAM-only via ces variables.
    crate::shell::set_exported_for_boot("BOUCHAUD_LADYBIRD_EPHEMERAL", "1");
    crate::shell::set_exported_for_boot("BOUCHAUD_DISABLE_SQL", "1");
    crate::shell::set_exported_for_boot("BOUCHAUD_DISABLE_DISK_CACHE", "1");
    crate::shell::set_exported_for_boot("BOUCHAUD_DISABLE_ASYNC_SCROLLING", "1");
    crate::shell::set_exported_for_boot("BOUCHAUD_DISABLE_AUDIO", "1");

    crate::serial_println!("BOUCHAUD_STAGE2_LADYBIRD_RAMONLY_ENV_OK");

    let browser_present = {
        let fs = crate::fs::ramfs::fs();
        fs.resolve("/bo-navigateur", 0).is_some()
    };
    let data_mounted = crate::fs::tar::mounted().is_some();
    let network_ready = crate::net::external_enabled();

    crate::serial_println!(
        "[STAGE2] runtime users=ready ramfs=ready process=ready bsp=1 data={} source={} persist={} net={} nic={}",
        data_mounted as u8,
        data_source,
        persist_restored,
        network_ready as u8,
        if crate::drivers::e1000::using_rtl8168() { "rtl8168" } else { "e1000" },
    );
    crate::serial_println!("BOUCHAUD_STAGE2_WM_RUNTIME_READY");

    if browser_present && data_mounted && network_ready {
        crate::serial_println!(
            "BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK url=https://example.com/"
        );
    } else {
        crate::serial_println!(
            "BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_DEGRADED browser={} data={} net={}",
            browser_present as u8,
            data_mounted as u8,
            network_ready as u8,
        );
    }

    // Vrai desktop -> vrai window_manager -> vrai handle_click.
    crate::serial_println!("[STAGE2] entering real Bouchaud window manager");
    crate::gui::desktop::run();

    crate::serial_println!("[STAGE2] window manager exited");
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}
