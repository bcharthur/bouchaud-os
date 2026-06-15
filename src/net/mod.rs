//! Pile reseau de Bouchaud OS — ROADMAP V0.6 (non active).
//!
//! Aucune couche reseau n'est encore implementee. Ce module centralise la
//! feuille de route OSI et fournit les messages des commandes placeholder
//! (`ifconfig`, `ping`, ...), en indiquant a chaque fois quelle couche manque.
//!
//! Ordre d'implementation vise :
//!   1. PCI            : enumeration du bus pour trouver la carte reseau
//!   2. driver         : e1000 ou virtio-net
//!   3. Ethernet (L2)  : trames, adresses MAC
//!   4. ARP            : resolution MAC <-> IPv4
//!   5. IPv4 (L3)      : adressage, routage simple
//!   6. ICMP           : echo (ping)
//!   7. UDP (L4)       : datagrammes
//!   8. DHCP           : configuration auto de l'adresse
//!   9. DNS            : resolution de noms
//!  10. TCP (L4)       : flux fiables
//!  11. HTTP (L7)      : wget / curl
//!  12. TLS            : securisation (plus tard)

use crate::drivers::vga::{self, COLOR_YELLOW, COLOR_DEFAULT};

/// Indique si la pile reseau est active.
pub fn enabled() -> bool {
    false
}

/// Affiche la feuille de route OSI complete (commande `roadmap`, section reseau).
pub fn print_roadmap() {
    println!("pile reseau OSI (etat: non activee):");
    println!("  1. PCI            scan du bus                      [planifie]");
    println!("  2. driver         e1000 / virtio-net              [planifie]");
    println!("  3. Ethernet L2    trames + MAC                    [planifie]");
    println!("  4. ARP            resolution MAC<->IPv4           [planifie]");
    println!("  5. IPv4 L3        adressage + routage             [planifie]");
    println!("  6. ICMP           echo / ping                     [planifie]");
    println!("  7. UDP L4         datagrammes                     [planifie]");
    println!("  8. DHCP           config auto                     [planifie]");
    println!("  9. DNS            resolution de noms              [planifie]");
    println!(" 10. TCP L4         flux fiables                    [planifie]");
    println!(" 11. HTTP L7        wget / curl                     [planifie]");
    println!(" 12. TLS            securite                        [plus tard]");
}

/// Renvoie la couche OSI manquante pour une commande reseau donnee.
fn missing_layer(cmd: &str) -> &'static str {
    match cmd {
        "ifconfig" | "ip" => "driver carte reseau (PCI + e1000/virtio-net)",
        "route" => "couche IPv4 (L3) + table de routage",
        "arp" => "couche Ethernet (L2) + protocole ARP",
        "ping" => "couches IPv4 + ICMP",
        "dhcp" => "couches UDP + client DHCP",
        "dns" => "couches UDP + resolveur DNS",
        "wget" | "curl" => "couches TCP + HTTP",
        _ => "pile reseau complete",
    }
}

/// Message standard d'une commande reseau placeholder.
pub fn placeholder(cmd: &str) {
    vga::set_color(COLOR_YELLOW);
    println!("{}: pile reseau non activee dans V0.6", cmd);
    vga::set_color(COLOR_DEFAULT);
    println!("  couche manquante: {}", missing_layer(cmd));
    println!("  roadmap: PCI -> driver -> Ethernet -> ARP -> IPv4 -> ICMP -> UDP -> DHCP/DNS -> TCP -> HTTP");
}
