//! Stage 2 - vrai bureau Bouchaud interactif sur GOP UEFI.
//!
//! Runtime volontairement RAM-only et BSP-only.

use crate::boot::{BootInfo, FirmwareKind};

// DEUX DRAPEAUX, ET NON UN.
//
// Un seul drapeau force a choisir entre « tout le PS/2 » et « rien ». Une
// machine dont le clavier USB est reconnu et la souris non se retrouvait alors
// sans pointeur : un bureau ou l'on peut taper et rien cliquer.
//
// Et l'inverse compte tout autant : initialiser le PS/2 alors qu'un
// peripherique USB du meme genre repond deja ferait arriver chaque frappe DEUX
// FOIS, si le micrologiciel emule encore un 8042 par-dessus l'USB. On decide
// donc genre par genre.
static LEGACY_PS2_CLAVIER: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(true);
static LEGACY_PS2_SOURIS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(true);

/// Le clavier PS/2 doit-il etre initialise ?
pub fn legacy_ps2_clavier() -> bool {
    LEGACY_PS2_CLAVIER.load(core::sync::atomic::Ordering::Acquire)
}

/// La souris PS/2 doit-elle etre initialisee ?
pub fn legacy_ps2_souris() -> bool {
    LEGACY_PS2_SOURIS.load(core::sync::atomic::Ordering::Acquire)
}

/// Conserve pour les appelants qui ne distinguent pas les deux.
pub fn legacy_ps2_allowed() -> bool {
    legacy_ps2_clavier() || legacy_ps2_souris()
}

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

    // Breadcrumb physique : reutilise le renderer GOP du Stage 1 deja
    // prouve sur le TRIGKEY. Si cet ecran apparait, le noyau a bien atteint
    // Stage 2 et le blocage est necessairement apres ce point.
    let _ = super::reference_gop::render_stage1(boot, framebuffer);
    crate::serial_println!("BOUCHAUD_TRIGKEY_STAGE2_EARLY_GOP_OK");

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
        "[STAGE2] smp=physical-probe-v31 audio=off storage=read-mostly network=auto pci=scan"
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

    // Le clavier n'est plus initialise ici. Sur le TRIGKEY les
    // peripheriques utilisateur sont USB/xHCI ; on decide apres le scan PCI
    // s'il existe reellement un chemin legacy 8042 a utiliser.

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
            // L'installateur recopiera CETTE archive sur le disque : le
            // systeme installe recoit donc, octet pour octet, celle qui vient
            // de tourner.
            crate::platform::pc::installation::note_archive(
                ramdisk.address, ramdisk.byte_len,
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

    // Inventaire physique read-only. Les rapports sont crees dans
    // /diagnostics avant les pilotes reseau/USB actifs.
    crate::platform::pc::hardware_probe::run(boot, framebuffer);

    // Le disque interne. C'est lui, et rien d'autre, qui separe un systeme
    // LIVE d'un systeme INSTALLE : tant que le NVMe n'etait pas pilote, aucune
    // ecriture ne survivait a une coupure, et le runtime ne POUVAIT etre que
    // RAM-only. La sonde le detectait deja et ecrivait « runtime driver
    // missing » ; c'etait un constat, pas une fatalite.
    //
    // L'echec n'est pas fatal : une machine sans NVMe -- QEMU, une cle seule --
    // doit continuer de demarrer en live. C'est l'installateur, et lui seul,
    // qui exige un disque.
    let nvme_pret = crate::drivers::nvme::bring_up();

    // Le disque interne porte-t-il DEJA une installation ? Si oui, sa
    // partition systeme devient le volume des donnees, et la persistance
    // ecrit dessus au lieu de disparaitre a l'extinction.
    //
    // Cela se decide APRES la preparation de `/persist` en RAM, et c'est le
    // bon ordre : l'arborescence existe d'abord, le contenu sauvegarde vient
    // se poser dessus. L'inverse donnerait un montage qui restaure des
    // fichiers dans des repertoires qui n'existent pas encore.
    let persist_disque = if nvme_pret
        && crate::platform::pc::installation::monte_le_systeme_installe()
    {
        let restaures = crate::fs::persistance::monte();
        crate::serial_println!(
            "BOUCHAUD_STAGE2_PERSIST_NVME fichiers={}", restaures
        );
        Some(restaures)
    } else {
        None
    };
    let xhci_present =
        if let Some(xhci) = crate::arch::x86_64::pci::find_xhci() {
            crate::serial_println!(
                "BOUCHAUD_TRIGKEY_XHCI_PRESENT pci={:04x}:{:04x} bus={:02x}:{:02x}.{} bar0={:#x}",
                xhci.vendor, xhci.device, xhci.bus, xhci.slot, xhci.func,
                crate::arch::x86_64::pci::bar_decode(&xhci, 0).adresse(),
            );
            true
        } else {
            crate::serial_println!("BOUCHAUD_TRIGKEY_XHCI_ABSENT");
            false
        };

    // CE QUI DECIDE, C'EST UN CLAVIER TROUVE -- PAS UN CONTROLEUR PRESENT.
    //
    // La regle etait « un xHCI existe, donc on saute le PS/2 ». Elle est vraie
    // quand l'enumeration USB reussit, et elle transforme un echec en machine
    // SANS AUCUNE ENTREE : ni clavier USB, parce que l'enumeration a echoue,
    // ni clavier PS/2, parce qu'on ne l'a pas essaye. L'utilisateur voit alors
    // un bureau sur lequel il ne peut rien faire, et rien ne lui dit pourquoi.
    //
    // On regarde donc ce qui a ete TROUVE. Le repli ne coute qu'une sonde du
    // controleur 8042, qui n'existe simplement pas sur une machine qui n'en a
    // pas -- et beaucoup de mini-PC en gardent un, emule par le micrologiciel
    // ou porte par leur puce d'entree-sortie.
    //
    // Une reserve, et il faut la dire : nous venons de desarmer les SMI du
    // micrologiciel. Si son 8042 etait une EMULATION de nos peripheriques USB,
    // elle ne repondra plus. Le repli ne rattrape donc que les machines dont
    // le 8042 est reel. C'est peu, et c'est plus que rien.
    let claviers_usb = crate::drivers::xhci_active::hid_keyboards();
    let souris_usb = crate::drivers::xhci_active::hid_mice();

    LEGACY_PS2_CLAVIER.store(claviers_usb == 0, core::sync::atomic::Ordering::Release);
    LEGACY_PS2_SOURIS.store(souris_usb == 0, core::sync::atomic::Ordering::Release);

    if claviers_usb == 0 {
        crate::drivers::keyboard::init();
        if xhci_present {
            // Le cas qui compte : un controleur existe, aucun clavier n'en est
            // sorti. Le dire nommement evite de chercher le defaut ailleurs.
            crate::serial_println!(
                "BOUCHAUD_TRIGKEY_REPLI_PS2 raison=xhci-present-sans-clavier concentrateurs_non_traverses={}",
                crate::drivers::xhci_active::concentrateurs_non_traverses(),
            );
        }
        crate::serial_println!(
            "BOUCHAUD_STAGE2_LEGACY_PS2_KEYBOARD_READY xhci_present={}",
            xhci_present as u8,
        );
    } else {
        crate::serial_println!(
            "BOUCHAUD_TRIGKEY_PS2_CLAVIER_SAUTE claviers_usb={}",
            claviers_usb,
        );
    }
    crate::serial_println!(
        "BOUCHAUD_STAGE2_ENTREE_DECIDEE claviers_usb={} souris_usb={} ps2_clavier={} ps2_souris={}",
        claviers_usb,
        souris_usb,
        (claviers_usb == 0) as u8,
        (souris_usb == 0) as u8,
    );

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
        "[STAGE2] runtime users=ready ramfs=ready process=ready bsp=1 data={} source={} persist={} net={} nic={} nvme={}",
        data_mounted as u8,
        data_source,
        persist_restored,
        network_ready as u8,
        if crate::drivers::e1000::using_rtl8168() { "rtl8168" } else { "e1000" },
        nvme_pret as u8,
    );
    match persist_disque {
        Some(restaures) => crate::serial_println!(
            "BOUCHAUD_STAGE2_INSTALLE persist=nvme restaures={}", restaures
        ),
        None => crate::serial_println!("BOUCHAUD_STAGE2_LIVE persist=ram"),
    }
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

    // BOUCHAUD_STAGE2_VISIBLE_DIAG_SMP_V31
    // First expose the USB evidence while the machine is still BSP-only. If the
    // AP bootstrap itself regresses on physical hardware, the xHCI photos are
    // still obtainable and the two investigations remain separable.
    crate::platform::pc::physical_diag::show_usb();

    crate::serial_println!("BOUCHAUD_STAGE2_SMP_PROBE_V31_BEGIN");
    crate::arch::x86_64::smp::init_probe();
    crate::platform::pc::physical_diag::show_smp();

    // The APs have waited behind SCHEDULER_ENABLED throughout boot/diagnostic.
    // Release them only now, once process/runtime initialization is complete.
    if !crate::arch::x86_64::smp::scheduler_enabled() {
        crate::arch::x86_64::smp::enable_scheduler();
    }
    crate::serial_println!(
        "BOUCHAUD_STAGE2_SMP_WIRED_V31 online={} detected={}",
        crate::arch::x86_64::smp::schedulable_cpus(),
        crate::arch::x86_64::smp::discovered_cpus(),
    );

    // Vrai desktop -> vrai window_manager -> vrai handle_click.
    crate::serial_println!("[STAGE2] entering real Bouchaud window manager");
    crate::gui::desktop::run();

    crate::serial_println!("[STAGE2] window manager exited");
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}
