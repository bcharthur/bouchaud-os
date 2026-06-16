//! Scan du bus PCI via le mecanisme de configuration #1 (ports 0xCF8/0xCFC).
//!
//! Premier etage concret de la pile materielle/reseau : on enumere les
//! peripheriques presents (vendor/device/classe). Fonctionne sans interruptions
//! et sans allocation. C'est la base sur laquelle viendra le futur driver
//! reseau (e1000 / virtio-net).

use crate::kernel::dmesg;

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
    Some(PciDevice { bus, slot, func, vendor, device, class, subclass })
}

/// Compte les peripheriques PCI presents (bus 0, suffisant sous QEMU).
pub fn count() -> usize {
    let mut n = 0;
    for slot in 0..32u8 {
        for func in 0..8u8 {
            if read_device(0, slot, func).is_some() { n += 1; }
        }
    }
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

/// Cherche la premiere carte reseau PCI presente.
pub fn find_network() -> Option<PciDevice> {
    for slot in 0..32u8 {
        for func in 0..8u8 {
            if let Some(d) = read_device(0, slot, func) {
                if is_network(&d) { return Some(d); }
            }
        }
    }
    None
}

/// Scan de boot : journalise le nombre de peripheriques et la carte reseau.
pub fn init() {
    let n = count();
    dmesg::log("pci: scan du bus 0 effectue");
    match find_network() {
        Some(d) => {
            // Trace lisible cote serie sans format complexe dans dmesg.
            crate::serial_println!(
                "[kernel] pci: {} peripheriques, carte reseau {:04x}:{:04x} ({})",
                n, d.vendor, d.device, vendor_name(d.vendor)
            );
            dmesg::log("pci: carte reseau detectee (driver non charge)");
        }
        None => {
            crate::serial_println!("[kernel] pci: {} peripheriques, aucune carte reseau", n);
            dmesg::log("pci: aucune carte reseau detectee");
        }
    }
}
