//! Pilote reseau (NIC) — socle.
//!
//! La carte reseau est detectee par le scan PCI mais aucun driver n'est encore
//! charge. La logique des couches (Ethernet/ARP/IPv4/ICMP) vit dans `crate::net`
//! et fonctionne en loopback. Prochaine etape : driver e1000/virtio-net (anneaux
//! RX/TX + DMA) pour l'acces reseau reel, puis UDP/DHCP/DNS/TCP/HTTP/TLS.

use crate::arch::x86_64::pci;

/// Le driver de carte reseau est-il charge ?
pub fn driver_loaded() -> bool {
    false
}

/// Affiche l'etat du pilote reseau (commande `devices`/`netinfo`).
pub fn print_info() {
    match pci::find_network() {
        Some(d) => crate::println!("net: carte {:04x}:{:04x} ({}) detectee, driver non charge",
            d.vendor, d.device, pci::vendor_name(d.vendor)),
        None => crate::println!("net: aucune carte reseau PCI"),
    }
    crate::println!("  loopback 127.0.0.1 actif ; pile L2-L4 en logique (voir 'ping')");
}
