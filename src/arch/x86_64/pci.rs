//! Scan du bus PCI via le mecanisme de configuration #1 (ports 0xCF8/0xCFC).
//!
//! Premier etage concret de la pile materielle/reseau : on enumere les
//! peripheriques presents (vendor/device/classe). Fonctionne sans interruptions
//! et sans allocation. C'est la base sur laquelle viendra le futur driver
//! reseau (e1000 / virtio-net).

use crate::kernel::dmesg;

// Le decodage pur vit dans son propre fichier, dans le MEME module : il ne
// touche ni 0xCF8 ni 0xCFC, ce qui permet de le mettre a l'epreuve sur l'hote
// avec un espace de configuration fabrique -- y compris une liste de capacites
// qui boucle sur elle-meme.
include!("pci/decodage.rs");

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

/// Un peripherique PCI decouvert.
#[derive(Copy, Clone)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    /// Interface de programmation. C'est elle qui distingue un NVM Express
    /// d'un autre controleur de stockage de la meme sous-classe.
    pub prog_if: u8,
    /// Type d'en-tete. Le bit 7 dit « multifonction », les bits bas
    /// distinguent un peripherique (0) d'un PONT (1).
    pub header_type: u8,
}

fn config_read32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = (1u32 << 31)
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);
    unsafe {
        // outl / inl via deux acces : on passe par le port 32 bits.
        out32(CONFIG_ADDRESS, address);
        in32(CONFIG_DATA)
    }
}

fn config_write32(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let address = (1u32 << 31)
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);
    unsafe {
        out32(CONFIG_ADDRESS, address);
        out32(CONFIG_DATA, value);
    }
}

/// Lit un BAR (Base Address Register) brut, index 0..5.
pub fn bar(d: &PciDevice, index: u8) -> u32 {
    config_read32(d.bus, d.slot, d.func, 0x10 + index * 4)
}

/// Active le bus mastering + l'espace memoire/IO pour un peripherique (necessaire
/// au DMA d'une carte reseau).
pub fn enable_bus_master(d: &PciDevice) {
    let cmd = config_read32(d.bus, d.slot, d.func, 0x04);
    // bit0 = I/O space, bit1 = memory space, bit2 = bus master.
    config_write32(d.bus, d.slot, d.func, 0x04, cmd | 0x07);
}

unsafe fn out32(port: u16, value: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
}

unsafe fn in32(port: u16) -> u32 {
    let value: u32;
    core::arch::asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

fn read_device(bus: u8, slot: u8, func: u8) -> Option<PciDevice> {
    let id = config_read32(bus, slot, func, 0x00);
    let vendor = (id & 0xFFFF) as u16;
    if vendor == 0xFFFF {
        return None; // emplacement vide
    }
    let device = (id >> 16) as u16;
    let class_reg = config_read32(bus, slot, func, 0x08);
    let class = (class_reg >> 24) as u8;
    let subclass = (class_reg >> 16) as u8;
    let prog_if = (class_reg >> 8) as u8;
    let header_type = (config_read32(bus, slot, func, 0x0C) >> 16) as u8;
    Some(PciDevice {
        bus, slot, func, vendor, device, class, subclass, prog_if, header_type,
    })
}

// BOUCHAUD_C10_ENUMERATION_RECURSIVE_V1
//
// CE QUE LE BALAYAGE DU BUS 0 NE VOYAIT PAS
//
// Il balayait le bus 0, et rien d'autre. C'est suffisant sur i440fx, ou tout
// est branche sur le bus unique -- et c'est exactement faux sur Q35, la
// plateforme de reference moderne : les peripheriques y sont derriere des PONTS
// RACINE PCIe, donc sur les bus 1, 2, 3... Un controleur NVMe attache a un port
// racine est INVISIBLE a un balayage du bus 0, et le systeme conclut « pas de
// disque » alors que le disque est la.
//
// La recursion suit les ponts par leur registre de bus secondaire. Elle est
// BORNEE par une profondeur et par un compte de bus visites : une topologie
// abimee peut chainer un pont sur lui-meme, et un parcours naif y tournerait
// pour toujours -- au boot, sans console.

/// Profondeur de ponts suivie. Une machine reelle en a deux ou trois.
const PROFONDEUR_PONTS_MAX: u8 = 8;

/// Applique `visite` a chaque fonction PCI atteignable depuis `bus`.
///
/// `visite` rend `false` pour arreter le parcours -- ce qui evite de balayer
/// toute la topologie quand on cherche le PREMIER peripherique d'une classe.
fn parcours_bus(bus: u8, profondeur: u8, visite: &mut dyn FnMut(&PciDevice) -> bool) -> bool {
    if profondeur > PROFONDEUR_PONTS_MAX {
        return true;
    }
    for slot in 0..32u8 {
        let Some(fonction_zero) = read_device(bus, slot, 0) else { continue };
        // Une fonction unique ne demande pas huit acces de configuration.
        let fonctions = if multifonction(fonction_zero.header_type) { 8 } else { 1 };
        for func in 0..fonctions {
            let Some(d) = read_device(bus, slot, func) else { continue };
            if !visite(&d) {
                return false;
            }
            if est_pont(d.class, d.subclass, d.header_type) {
                let mot = config_read32(d.bus, d.slot, d.func, 0x18);
                let secondaire = bus_secondaire(mot);
                // Un pont dont le bus secondaire vaut le sien -- ou zero --
                // est mal configure : le suivre serait une boucle.
                if secondaire != 0 && secondaire != bus
                    && !parcours_bus(secondaire, profondeur + 1, visite)
                {
                    return false;
                }
            }
        }
    }
    true
}

/// Applique `visite` a toute la topologie, ponts compris.
pub fn parcours(visite: &mut dyn FnMut(&PciDevice) -> bool) {
    parcours_bus(0, 0, visite);
}

/// Les capacites d'un peripherique, dans `sortie`. Rend combien ont ete lues.
///
/// MSI et MSI-X y vivent : sans cette liste, un controleur moderne ne peut
/// delivrer ses interruptions que par la ligne heritee -- partagee, lente, et
/// absente de certaines topologies PCIe. C'est la premiere brique que NVMe
/// demande.
pub fn capacites_de(d: &PciDevice, sortie: &mut [Capacite]) -> usize {
    let statut_commande = config_read32(d.bus, d.slot, d.func, 0x04);
    capacites(
        statut_commande,
        |decalage| config_read32(d.bus, d.slot, d.func, decalage),
        sortie,
    )
}

/// Le BAR `index`, en tenant compte des BAR memoire 64 bits.
///
/// `bar()` lisait des mots de 32 bits. Le BAR0 d'un NVMe est un BAR MEMOIRE
/// 64 BITS : lu sur 32 bits, il donne la moitie basse d'une adresse, ce qui est
/// pire qu'une erreur -- c'est une adresse plausible qui pointe ailleurs.
pub fn bar_decode(d: &PciDevice, index: u8) -> Bar {
    if index >= 6 {
        return Bar::Absent;
    }
    let bas = config_read32(d.bus, d.slot, d.func, 0x10 + index * 4);
    let haut = if index < 5 {
        config_read32(d.bus, d.slot, d.func, 0x10 + (index + 1) * 4)
    } else {
        0
    };
    decode_bar(bas, haut)
}

/// Le premier controleur NVM Express de la machine, ou `None`.
///
/// Aucun pilote ne le programme encore. Le DETECTER est ce qui manquait : tant
/// que l'enumeration ne voyait pas au-dela du bus 0, on ne pouvait meme pas
/// savoir s'il y en avait un.
pub fn find_nvme() -> Option<PciDevice> {
    let mut trouve = None;
    parcours(&mut |d| {
        if est_nvme(d.class, d.subclass, d.prog_if) {
            trouve = Some(*d);
            return false;
        }
        true
    });
    trouve
}

/// Compte les peripheriques PCI presents, PONTS COMPRIS.
pub fn count() -> usize {
    let mut n = 0;
    parcours(&mut |_| { n += 1; true });
    n
}

/// Nom lisible d'un constructeur connu.
pub fn vendor_name(vendor: u16) -> &'static str {
    match vendor {
        0x8086 => "Intel",
        0x1022 => "AMD",
        0x10EC => "Realtek",
        0x1AF4 => "Red Hat / virtio",
        0x1234 => "QEMU/Bochs",
        0x1B36 => "Red Hat QEMU",
        _ => "inconnu",
    }
}

/// Description courte d'une classe PCI.
pub fn class_name(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (0x01, _) => "controleur de stockage",
        (0x02, _) => "controleur reseau",
        (0x03, _) => "controleur graphique",
        (0x06, 0x00) => "pont hote",
        (0x06, 0x01) => "pont ISA",
        (0x06, _) => "pont",
        (0x0C, _) => "controleur serie/USB",
        _ => "peripherique",
    }
}

/// Indique si un peripherique est une carte reseau connue.
pub fn is_network(dev: &PciDevice) -> bool {
    dev.class == 0x02
}

/// Affiche tous les peripheriques PCI (commande `lspci`).
pub fn print_devices() {
    let mut found = false;
    for slot in 0..32u8 {
        for func in 0..8u8 {
            if let Some(d) = read_device(0, slot, func) {
                found = true;
                crate::println!(
                    "{:02x}:{:02x}.{} {:04x}:{:04x} {} - {}",
                    d.bus, d.slot, d.func, d.vendor, d.device,
                    vendor_name(d.vendor), class_name(d.class, d.subclass)
                );
            }
        }
    }
    if !found {
        crate::println!("lspci: aucun peripherique PCI detecte");
    }
}

/// Cherche le premier controleur graphique PCI (classe 0x03), p.ex. la carte
/// VGA `std`/Bochs de QEMU (1234:1111) dont le BAR0 est le framebuffer lineaire.
pub fn find_display() -> Option<PciDevice> {
    let mut trouve = None;
    parcours(&mut |d| {
        if d.class == 0x03 { trouve = Some(*d); return false; }
        true
    });
    trouve
}

/// Cherche le premier peripherique audio PCI (classe 0x04, sous-classe 0x01).
///
/// C'est ainsi que se declarent aussi bien l'AC'97 d'Intel que l'ES1370
/// d'Ensoniq ; le pilote verifie ensuite qu'il sait parler a celui qu'il a
/// trouve.
pub fn find_audio() -> Option<PciDevice> {
    let mut trouve = None;
    parcours(&mut |d| {
        if d.class == 0x04 && (d.subclass == 0x01 || d.subclass == 0x03) {
            trouve = Some(*d);
            return false;
        }
        true
    });
    trouve
}

/// Ligne d'interruption affectee au peripherique (registre 0x3C).
pub fn interrupt_line(d: &PciDevice) -> u8 {
    (config_read32(d.bus, d.slot, d.func, 0x3C) & 0xFF) as u8
}

/// Cherche la premiere carte reseau PCI presente, ponts compris.
pub fn find_network() -> Option<PciDevice> {
    let mut trouve = None;
    parcours(&mut |d| {
        if is_network(d) { trouve = Some(*d); return false; }
        true
    });
    trouve
}

/// Scan de boot : inventaire de la topologie, ponts compris.
pub fn init() {
    let n = count();
    let mut bus_vus = 0usize;
    let mut ponts = 0usize;
    let mut dernier_bus = u16::MAX;
    parcours(&mut |d| {
        if d.bus as u16 != dernier_bus {
            dernier_bus = d.bus as u16;
            bus_vus += 1;
        }
        if est_pont(d.class, d.subclass, d.header_type) { ponts += 1; }
        true
    });
    // Ce que le scan sait DIRE, et rien de plus. « scan du bus 0 effectue »
    // etait vrai et trompeur : sur Q35, ce qui compte est justement ce qui est
    // derriere les ponts.
    crate::serial_println!(
        "[PCI-NG] peripheriques={} bus={} ponts={}", n, bus_vus, ponts
    );
    if let Some(nvme) = find_nvme() {
        let mut capacites = [Capacite { identifiant: 0, decalage: 0 }; 16];
        let trouvees = capacites_de(&nvme, &mut capacites);
        let msix = trouve_capacite(&capacites[..trouvees], CAP_MSIX).is_some();
        let msi = trouve_capacite(&capacites[..trouvees], CAP_MSI).is_some();
        crate::serial_println!(
            "[PCI-NG] nvme {:04x}:{:04x} bus={} bar0={:#x} msi={} msix={}",
            nvme.vendor, nvme.device, nvme.bus,
            bar_decode(&nvme, 0).adresse(), msi, msix
        );
        // Aucun pilote ne le programme encore. Le DETECTER est ce qui manquait :
        // tant que l'enumeration s'arretait au bus 0, on ne pouvait meme pas
        // savoir s'il y en avait un.
        dmesg::log("pci: controleur NVMe detecte (pilote absent)");
    }
    dmesg::log("pci: topologie parcourue, ponts compris");
    match find_network() {
        Some(d) => {
            // Trace lisible cote serie sans format complexe dans dmesg.
            crate::serial_println!(
                "[kernel] pci: {} peripheriques, carte reseau {:04x}:{:04x} ({})",
                n, d.vendor, d.device, vendor_name(d.vendor)
            );
            // Ce que le scan sait, et rien de plus. La ligne disait
            // « driver non charge » : c'etait vrai du temps ou il fallait
            // taper `ifup`, et c'est devenu faux le jour ou `net::demarre()`
            // a charge le pilote quelques lignes plus bas dans le meme
            // demarrage. Le journal affichait donc « driver non charge »
            // juste avant « e1000: initialise », ce qui fait douter de la
            // seconde ligne plutot que de la premiere.
            dmesg::log("pci: carte reseau detectee");
        }
        None => {
            crate::serial_println!("[kernel] pci: {} peripheriques, aucune carte reseau", n);
            dmesg::log("pci: aucune carte reseau detectee");
        }
    }
}
