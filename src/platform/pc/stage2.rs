//! Stage 2 - vrai bureau Bouchaud interactif sur GOP UEFI.
//!
//! Runtime volontairement RAM-only et BSP-only.

use crate::boot::{BootInfo, FirmwareKind};

// BOUCHAUD_PAGE_ACCUEIL_UN_SEUL_CHEMIN_V1
//
// Le chemin de la page d'accueil existait en TROIS exemplaires qui devaient
// s'accorder sans que rien ne le verifie : la chaine exportee ici, la copie
// faite par tools/reference/prepare-reference-ladybird.ps1, et l'entree
// attendue par tools/reference/verify-reference-ladybird-image.py. Les deux
// dernieres ne la connaissaient tout simplement pas, et le navigateur s'est
// ouvert sur une page absente.
//
// Cote noyau il n'y a plus qu'une constante, et le chemin dans l'image se
// deduit de l'URL par construction. tools/verifie-page-accueil-navigateur.py
// verifie que les trois s'accordent encore.
/// Chemin de la page d'accueil dans le systeme de fichiers du guest.
const CHEMIN_ACCUEIL: &str = "/usr/share/ladybird/bouchaud-start.html";
/// La meme page, telle que le navigateur la demande.
const URL_ACCUEIL: &str = "file:///usr/share/ladybird/bouchaud-start.html";

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

/// Note l'etape de demarrage en cours, pour l'ecran de faute.
///
/// Une faute noyau affiche le dernier point franchi : c'est la difference
/// entre « le systeme s'est arrete quelque part » et « le systeme s'est
/// arrete APRES le bring-up NVMe et AVANT l'entree ».
fn point_de_controle(nom: &str) {
    // A checkpoint must remain safe with interrupts disabled and before the
    // scheduler owns the current stack.  Drawing here caused a double fault
    // on the Trigkey immediately after the network checkpoint.
    super::ecran_faute::point(nom);
}

pub fn run(boot: &'static BootInfo) -> ! {
    if boot.firmware != FirmwareKind::Uefi {
        panic!("stage2: firmware UEFI requis");
    }

    let framebuffer = super::bringup::validate_uefi_stage1(boot);

    // L'ecran de faute AVANT tout le reste : a partir d'ici, une exception
    // noyau a de quoi s'afficher au lieu de laisser un ecran arrete.
    super::ecran_faute::installe_framebuffer(framebuffer);
    point_de_controle("stage2-entree");

    // L'IDENTITE DU BINAIRE, EN CLAIR, AVANT TOUT LE RESTE.
    //
    // La session du 17 septembre a cherche une panne dans du code qui n'etait
    // pas dans l'image testee : `netetat` repondait « commande inconnue » et
    // l'archive ne portait aucun des champs ajoutes. Il a fallu compter les
    // occurrences d'un champ pour s'en apercevoir. Cette ligne-la coute six
    // mots et supprime la question, sur la liaison serie comme dans l'archive.
    crate::serial_println!(
        "BOUCHAUD_BUILD commit={} lot={}",
        crate::kernel::blackbox::BUILD_COMMIT,
        crate::kernel::blackbox::BUILD_LOTS,
    );
    // L'OBSERVATOIRE EXISTE DES L'ENTREE, et non au moment du reseau : une
    // phase declaree apres coup ne date rien, et le pic de reveil de douze
    // secondes du releve physique est tombe AVANT que le reseau ne demarre.
    crate::kernel::services::declare_arbre();
    crate::kernel::services::phase("sys");

    // Breadcrumb physique : reutilise le renderer GOP du Stage 1 deja
    // prouve sur le TRIGKEY. Si cet ecran apparait, le noyau a bien atteint
    // Stage 2 et le blocage est necessairement apres ce point.
    // L'ECRAN DE DEMARRAGE, ET PLUS AUCUN LOGO.
    //
    // `boot_begin` ne peint plus que le fond ; `demarrage_ouvre` y installe le
    // titre et la barre, et chaque point de controle l'avance. Voir
    // `ecran_faute::demarrage_ouvre` pour la raison du choix du moteur de
    // rendu, et `reference_gop::boot_begin` pour celle du retrait du logo.
    //
    // Le prechargeur UEFI porte le MEME ecran depuis
    // tools/reference/uefi-preboot-probe : c'est lui qu'on voit en premier, et
    // c'est lui qui affichait le « B » que ce lot retire.
    super::reference_gop::boot_begin(framebuffer);
    super::ecran_faute::demarrage_ouvre();
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
    point_de_controle("sysroot");

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
    point_de_controle("journal");
    crate::kernel::process::init();

    // PCI est maintenant autorise au Stage 2 FINAL V2 pour une seule raison :
    // rendre le navigateur vraiment utilisable. `net::demarre` ne charge le
    // driver e1000 que sur une carte Intel ; un Realtek physique est refuse
    // proprement par le driver e1000.
    crate::arch::x86_64::pci::init();
    point_de_controle("pci");

    // Inventaire physique read-only. Les rapports sont crees dans
    // /diagnostics avant les pilotes reseau/USB actifs.
    crate::platform::pc::hardware_probe::run(boot, framebuffer);
    point_de_controle("hardware-probe-xhci");
    crate::kernel::services::phase("sys.usb");

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
    point_de_controle("nvme-bring-up");

    // LA PERSISTANCE N'EST PLUS UNE ETAPE D'AMORCAGE.
    //
    // Sonder la table de partitions du disque interne ici faisait dependre le
    // premier affichage du bureau d'un peripherique dont on ne sait rien. Un
    // controleur qui n'acheve pas ses commandes n'echouait pas : il faisait
    // attendre, interruptions masquees, une commande apres l'autre -- et un
    // probe GPT en emet des dizaines. Vu de la machine, c'est un ecran fige
    // sans clavier ni souris.
    //
    // Rien de ce que le bureau affiche ne vient de ce disque : l'archive
    // Ladybird voyage dans l'image UEFI et vit en RAM. La persistance n'ajoute
    // que la survie a une coupure. Elle est donc DEMANDEE ici et executee par
    // le bureau, une fois la premiere image rendue.
    if nvme_pret {
        crate::platform::pc::installation::differe_le_montage();
    } else {
        crate::serial_println!(
            "BOUCHAUD_NVME_PERSISTENCE_DEFERRED raison=aucun-disque-interne"
        );
    }
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

    crate::kernel::services::phase("net");
    let _network_state = crate::net::demarre();
    crate::kernel::services::etat(
        crate::kernel::services::carte_active(),
        if crate::drivers::e1000::is_ready() {
            crate::kernel::services::Etat::Actif
        } else {
            crate::kernel::services::Etat::Panne
        },
    );
    crate::kernel::services::etat(
        "net.link",
        if crate::drivers::e1000::link_up() {
            crate::kernel::services::Etat::Actif
        } else {
            crate::kernel::services::Etat::Attente
        },
    );

    // LE LIEN SE VEILLE, IL NE SE CONSTATE PAS UNE FOIS.
    //
    // Sur la machine de reference, l'autonegociation cuivre n'avait pas fini
    // trois secondes apres la mise sous tension : le verdict « lien bas »
    // etait definitif, et le navigateur repondait « Unable to resolve host »
    // pour le reste de la session.
    crate::serial_println!("BOUCHAUD_NET_VEILLEUR_DIFFERE raison=smp-bootstrap");
    point_de_controle("reseau");

    // Le run historique posait ces variables via /autorun. Le Stage 2 entre
    // directement dans le bureau, donc il doit fournir le meme contrat avant
    // que l'utilisateur double-clique sur Ladybird.
    crate::shell::set_exported_for_boot("BOUCHAUD_M9", "1");
    // Une page locale ne depend ni du DHCP ni d'un moteur distant au lancement.
    crate::shell::set_exported_for_boot("BOUCHAUD_M9_URL", URL_ACCUEIL);
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

    let (browser_present, page_accueil_presente) = {
        let fs = crate::fs::ramfs::fs();
        (
            fs.resolve("/bo-navigateur", 0).is_some(),
            fs.resolve(CHEMIN_ACCUEIL, 0).is_some(),
        )
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
    // Le systeme demarre TOUJOURS en live. La persistance, si le disque en
    // porte une, s'ajoute apres le premier rendu du bureau et le dit
    // elle-meme (`BOUCHAUD_STAGE2_PERSIST_NVME ... (differe)`).
    crate::serial_println!(
        "BOUCHAUD_STAGE2_LIVE persist=ram montage_differe={}",
        crate::platform::pc::installation::montage_differe_en_attente() as u8,
    );
    // BOUCHAUD_PAGE_ACCUEIL_ANNONCEE_AU_DEMARRAGE_V1
    //
    // La page d'accueil a manque a l'image pendant toute une session sans que
    // l'amorcage en dise un mot : le defaut ne s'est annonce que 83 s plus
    // tard, dans le journal de WebContent, sous la forme d'un « errno=2 »
    // precede de 9,4 s de recherche. Le noyau peut le savoir en une
    // resolution de chemin, avant meme de lancer le navigateur. Il le dit.
    crate::serial_println!(
        "BOUCHAUD_STAGE2_PAGE_ACCUEIL presente={} chemin={}",
        page_accueil_presente as u8,
        CHEMIN_ACCUEIL,
    );
    if !page_accueil_presente {
        crate::serial_println!(
            "BOUCHAUD_STAGE2_PAGE_ACCUEIL_ABSENTE url={} remede=prepare-reference-ladybird.ps1",
            URL_ACCUEIL,
        );
    }

    crate::serial_println!("BOUCHAUD_STAGE2_WM_RUNTIME_READY");

    if browser_present && data_mounted && network_ready {
        // L'URL annoncee ici est celle qu'on EXPORTE, pas une adresse de
        // demonstration. Elle disait « https://www.google.com/ » alors que
        // BOUCHAUD_M9_URL portait la page locale : le journal d'amorcage
        // contredisait le BROWSER_HOST_URL imprime trois secondes plus loin.
        crate::serial_println!(
            "BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK url={}",
            URL_ACCUEIL,
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
    // LES PAGES DE DIAGNOSTIC NE SONT PLUS SUR LE CHEMIN DU BUREAU.
    //
    // Elles retenaient l'amorcage vingt-quatre secondes, et le retenaient avec
    // `hlt` : sans IRQ0, la derniere page devenait l'ecran definitif de la
    // machine. Les marqueurs serie et les rapports `/diagnostics` restent
    // ecrits ; seul l'affichage bloquant est conditionnel.
    // `physical_diag::montre_les_pages(true)` les rend pour une session de
    // bring-up.
    if crate::platform::pc::physical_diag::pages_visibles() {
        crate::platform::pc::physical_diag::show_usb();
    }

    crate::serial_println!("BOUCHAUD_STAGE2_SMP_PROBE_V31_BEGIN");
    crate::arch::x86_64::smp::init_probe();
    point_de_controle("smp");
    if crate::platform::pc::physical_diag::pages_visibles() {
        crate::platform::pc::physical_diag::show_smp();
    }

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
    // BOUCHAUD_SMP_BOOTSTRAP_GUARD_V1
    // Le veilleur reseau est une tache noyau recurrente. Il demarre seulement
    // apres le cablage SMP, afin de ne pas publier/reveiller `net-lien` pendant
    // la fenetre INIT/SIPI qui a declenche la double faute Trigkey.
    crate::net::demarre_le_veilleur_de_lien();
    crate::serial_println!("BOUCHAUD_NET_VEILLEUR_APRES_SMP");
    // L'AUDITEUR PART ICI, ET APRES LE VEILLEUR.
    //
    // Il lit ce que le pilote publie ; le lancer avant que le pilote existe
    // lui ferait poser ses temoins sur des zeros et croire, une seconde, a
    // une machine muette. Comme le veilleur, il attend le cablage SMP : c'est
    // une tache noyau recurrente, et la fenetre INIT/SIPI est celle qui a
    // declenche la double faute Trigkey.
    crate::kernel::lab::auditd::demarre();
    crate::kernel::services::etat("net.link", crate::kernel::services::Etat::Actif);
    crate::kernel::services::phase("sys.graphique");

    // Le montage differe part ICI, et non a la troisieme trame du bureau : il
    // n'a aucun resultat que le premier rendu attende, et le faire dependre du
    // compositeur le rendait absent partout ou le compositeur ne demarre pas.
    // Voir la note d'amorcage equivalente dans `main.rs`.
    crate::platform::pc::installation::lance_le_montage_differe();

    // LE MODE DE SECOURS EST CONSULTE ICI, ET NULLE PART AILLEURS.
    //
    // Un bureau qui ne demarre pas laisse une machine sans aucun moyen de dire
    // pourquoi : ni journal lisible, ni commande a taper. La console, elle,
    // suffit a lancer `hwtest` et a relever un `bootlog`.
    if crate::platform::pc::trigkey::secours_demande() {
        crate::serial_println!("BOUCHAUD_STAGE2_SAFE_MODE bureau=non-lance console=texte");
        crate::drivers::vga::set_serial_mirror(true);
        crate::shell::run();
    }

    // L'ENTREE AVANT LE BUREAU.
    //
    // Le fil de scrutation doit exister AVANT que le compositeur ne prenne la
    // main : sinon la premiere seconde du bureau -- celle du chargement des
    // polices, la plus lente -- se passe encore sans souris.
    crate::drivers::xhci_active::demarre_le_fil_hid();

    // LE PONT EP0 SUR SON PROPRE FIL.
    //
    // Un peripherique muet en Interrupt-IN -- le clavier de la machine de
    // reference -- n'a pas d'autre transport que `GET_REPORT`, et ce transfert
    // est synchrone. Le laisser dans la boucle de scrutation ramenait celle-ci
    // de mille tours par seconde a cent soixante-six.
    crate::drivers::xhci_active::demarre_le_fil_repli_ep0();

    // L'enregistreur de vol part avec son propre fil : il ecrit sur la cle
    // USB par transferts synchrones, et ce cout n'a rien a faire sur le
    // chemin de l'entree.
    crate::drivers::xhci_active::demarre_le_fil_blackbox();

    // LE BRING-UP EST FINI : LES BUDGETS PASSENT EN MODE RUNTIME.
    //
    // Jusqu'ici, une cle lente avait le droit de prendre une demi-seconde par
    // transfert -- personne n'attendait devant l'ecran. A partir de la, le
    // clavier et la souris attendent, et une commande de stockage qui traine
    // se voit a l'ecran. Voir `budget_bot`.
    crate::drivers::xhci_active::bring_up_termine();

    // LE BANC DE CHARGE, S'IL A ETE COMPILE.
    //
    // Il part APRES `bring_up_termine` : lancer une charge de lecture pendant
    // que les budgets sont encore ceux de l'enumeration mesurerait un pilote
    // qui n'est pas celui qui tourne devant l'utilisateur.
    #[cfg(feature = "banc-io")]
    crate::platform::pc::banc_io::demarre();

    // POURQUOI `prechauffage::demarre()` N'EST PAS APPELE ICI.
    //
    // `main.rs` le lance, et ce chemin-ci ne le lance pas. Ce n'est PAS un
    // oubli, et la question se repose a chaque relecture -- d'ou cette note.
    //
    // Sur la machine de reference, le navigateur demarre TOUT SEUL cinq cents
    // millisecondes apres la premiere trame du bureau, en icone : voir
    // `window_manager`, `services_initialises`. Son vrai jeu de travail
    // remplit donc le cache de pages propres de lui-meme, et tout de suite.
    //
    // Un prechauffage speculatif lance en plus ne ferait pas double emploi :
    // il ferait CONCURRENCE. Le cache de pages propres retient au plus seize
    // mille trois cent quatre-vingt-quatre pages, et les deux remplissages
    // viseraient les memes binaires -- celui qui arrive en second chasserait
    // ce que le premier vient de charger.
    //
    // Si le demarrage automatique du navigateur disparaissait, ce raisonnement
    // tomberait avec lui et l'appel devrait revenir.

    // Vrai desktop -> vrai window_manager -> vrai handle_click.
    crate::serial_println!("[STAGE2] entering real Bouchaud window manager");
    point_de_controle("bureau");
    // L'ECRAN DE DEMARRAGE S'ARRETE ICI, ET PAS PLUS TARD.
    //
    // Des points de controle existent encore apres l'arrivee au bureau ; les
    // laisser peindre repeindrait une barre de progression par-dessus les
    // fenetres.
    super::ecran_faute::demarrage_ferme();
    crate::gui::desktop::run();

    crate::serial_println!("[STAGE2] window manager exited");
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}
