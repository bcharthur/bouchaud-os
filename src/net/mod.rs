//! Pile reseau de Bouchaud OS.
//!
//! Les couches sont implementees comme logique reelle (encodage/decodage sans
//! allocation) :
//!   - `ethernet` (L2), `arp`, `ipv4` (L3), `icmp`, `stack` (moteur).
//!
//! Etat actuel :
//!   - Interface **loopback `lo` (127.0.0.1) active** : `ping 127.0.0.1`
//!     traverse reellement la pile (ICMP echo request -> reply).
//!   - Interface **`eth0` : carte detectee par le scan PCI mais driver non
//!     charge** -> pas encore d'acces Internet externe. C'est la prochaine etape
//!     (driver e1000/virtio-net : rings RX/TX + DMA).

pub mod ethernet;
pub mod arp;
pub mod ipv4;
pub mod icmp;
pub mod stack;

use crate::arch::x86_64::pci;
use crate::drivers::e1000;
use crate::drivers::vga::{self, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW, COLOR_DEFAULT};
use crate::net::ipv4::Ipv4Addr;

/// Adresse de l'interface loopback.
pub const LO_ADDR: Ipv4Addr = [127, 0, 0, 1];
/// Adresse IPv4 statique d'eth0 (reseau utilisateur QEMU SLIRP).
pub const ETH_IP: Ipv4Addr = [10, 0, 2, 15];

/// Indique si une interface routable vers l'exterieur est active.
pub fn external_enabled() -> bool {
    e1000::is_ready()
}

// ---------------------------------------------------------------------------
// eth0 / e1000 : activation et ARP reel
// ---------------------------------------------------------------------------

/// Active l'interface eth0 (initialise le driver e1000).
pub fn ifup() {
    if e1000::init() {
        let m = e1000::mac();
        vga::set_color(COLOR_GREEN);
        println!("eth0 active");
        vga::set_color(COLOR_DEFAULT);
        println!("  MAC : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5]);
        crate::print!("  inet: "); ipv4::print_addr(&ETH_IP); println!("  lien={}", if e1000::link_up() { "UP" } else { "DOWN" });
    } else {
        vga::set_color(COLOR_YELLOW);
        println!("ifup: echec d'initialisation e1000 (lance QEMU avec -device e1000 -netdev user,id=n0)");
        vga::set_color(COLOR_DEFAULT);
    }
}

/// Envoie une requete ARP et attend une reponse (commande `arping <ip>`).
pub fn arping(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: arping <ip>"); return; }
    let target = match ipv4::parse_addr(argv[1]) {
        Some(a) => a,
        None => { println!("arping: adresse invalide"); return; }
    };
    if !e1000::is_ready() && !e1000::init() {
        println!("arping: carte reseau indisponible (essaie 'ifup')");
        return;
    }
    let mac = e1000::mac();

    // Construit la requete ARP encapsulee dans une trame Ethernet.
    let mut arp_buf = [0u8; arp::PACKET_LEN];
    if arp::build(&mut arp_buf, arp::OP_REQUEST, mac, ETH_IP, [0; 6], target).is_none() {
        println!("arping: erreur ARP");
        return;
    }
    let mut frame = [0u8; ethernet::HEADER_LEN + arp::PACKET_LEN];
    let flen = match ethernet::build_frame(&mut frame, ethernet::BROADCAST, mac, ethernet::ETHERTYPE_ARP, &arp_buf) {
        Some(n) => n,
        None => { println!("arping: erreur trame"); return; }
    };

    crate::print!("ARP qui a "); ipv4::print_addr(&target); println!(" ?");
    e1000::send(&frame[..flen]);

    // Attend une reponse ARP correspondant a la cible.
    let mut buf = [0u8; 2048];
    for _ in 0..3_000_000u32 {
        if let Some(n) = e1000::receive(&mut buf) {
            if n >= ethernet::HEADER_LEN + arp::PACKET_LEN {
                if let Some(h) = ethernet::parse_header(&buf[..n]) {
                    if h.ethertype == ethernet::ETHERTYPE_ARP {
                        if let Some(p) = arp::parse(&buf[ethernet::HEADER_LEN..n]) {
                            if p.op == arp::OP_REPLY && p.sender_ip == target {
                                vga::set_color(COLOR_GREEN);
                                crate::print!("reponse de "); ipv4::print_addr(&target);
                                println!(" : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                                    p.sender_mac[0], p.sender_mac[1], p.sender_mac[2],
                                    p.sender_mac[3], p.sender_mac[4], p.sender_mac[5]);
                                vga::set_color(COLOR_DEFAULT);
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
    println!("arping: pas de reponse (timeout)");
}

// ---------------------------------------------------------------------------
// ping
// ---------------------------------------------------------------------------

pub fn ping(argc: usize, argv: &[&str; 12]) {
    if argc < 2 {
        println!("usage: ping <ip>");
        return;
    }
    let target = match ipv4::parse_addr(argv[1]) {
        Some(a) => a,
        None => { println!("ping: adresse invalide (attendu a.b.c.d)"); return; }
    };

    crate::print!("PING ");
    ipv4::print_addr(&target);
    println!(" : 16 octets de donnees");

    if ipv4::is_loopback(&target) {
        ping_loopback(target);
    } else {
        // Pas de driver NIC : aucune interface ne peut router vers l'exterieur.
        vga::set_color(COLOR_YELLOW);
        println!("ping: no route to host");
        vga::set_color(COLOR_DEFAULT);
        match pci::find_network() {
            Some(d) => println!("  eth0 {:04x}:{:04x} detectee mais driver non charge (interface DOWN)", d.vendor, d.device),
            None => println!("  aucune carte reseau PCI; lance QEMU avec -device e1000 -netdev user,id=n0"),
        }
        println!("  seul loopback (127.0.0.1) est routable pour l'instant");
    }
}

/// Envoie 4 echo requests sur loopback en passant par la vraie pile ICMP.
fn ping_loopback(target: Ipv4Addr) {
    let payload = b"bouchaud-os-ping";
    let mut sent = 0u32;
    let mut recv = 0u32;

    for seq in 0..4u16 {
        let mut icmp_buf = [0u8; 64];
        let il = match icmp::build(&mut icmp_buf, icmp::ECHO_REQUEST, 0x4243, seq, payload) {
            Some(n) => n,
            None => continue,
        };
        let mut pkt = [0u8; 128];
        let pl = match ipv4::build_packet(&mut pkt, LO_ADDR, target, ipv4::PROTO_ICMP, seq, &icmp_buf[..il]) {
            Some(n) => n,
            None => continue,
        };
        sent += 1;

        // Le paquet "boucle" : il est traite par notre propre moteur de pile.
        let mut out = [0u8; 128];
        if let Some(rl) = stack::handle_ipv4(&pkt[..pl], &mut out) {
            if let Some(h) = ipv4::parse_header(&out[..rl]) {
                let reply = &out[h.header_len..h.total_len];
                if let Some(m) = icmp::parse(reply) {
                    if m.msg_type == icmp::ECHO_REPLY && m.seq == seq {
                        recv += 1;
                        crate::print!("{} octets de ", h.total_len);
                        ipv4::print_addr(&h.src);
                        println!(": icmp_seq={} ttl=64 temps<1ms (loopback)", seq);
                    }
                }
            }
        }
    }

    let lost = if sent > 0 { (sent - recv) * 100 / sent } else { 100 };
    println!("--- statistiques ping ---");
    println!("{} paquets transmis, {} recus, {}% perdus", sent, recv, lost);
}

// ---------------------------------------------------------------------------
// ifconfig / ip / route / arp
// ---------------------------------------------------------------------------

fn print_eth0_state() {
    if e1000::is_ready() {
        let m = e1000::mac();
        crate::print!("eth0: flags=<UP,RUNNING>  inet ");
        ipv4::print_addr(&ETH_IP);
        println!("  lien={}", if e1000::link_up() { "UP" } else { "DOWN" });
        println!("      ether {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5]);
        return;
    }
    match pci::find_network() {
        Some(d) => {
            crate::print!("eth0: flags=<DOWN>  carte PCI {:04x}:{:04x} (", d.vendor, d.device);
            crate::print!("{}", pci::vendor_name(d.vendor));
            println!(") - driver non charge (lance 'ifup')");
        }
        None => println!("eth0: absente (aucune carte reseau PCI detectee)"),
    }
}

pub fn ifconfig() {
    crate::print!("lo: flags=<UP,LOOPBACK,RUNNING>  inet ");
    ipv4::print_addr(&LO_ADDR);
    println!("  netmask 255.0.0.0");
    print_eth0_state();
}

pub fn ip_cmd() {
    println!("1: lo: <UP,LOOPBACK>");
    crate::print!("   inet ");
    ipv4::print_addr(&LO_ADDR);
    println!("/8 scope host lo");
    println!("2: eth0:");
    print_eth0_state();
}

pub fn route_cmd() {
    println!("Table de routage IPv4:");
    println!("  Destination     Masque          Interface");
    println!("  127.0.0.0       255.0.0.0       lo");
    println!("  (pas de route par defaut: eth0 DOWN, driver NIC non charge)");
}

pub fn arp_cmd() {
    println!("Cache ARP:");
    println!("  Adresse         HWaddr             Iface");
    println!("  (vide: aucune trame Ethernet emise tant que le driver NIC manque)");
}

// ---------------------------------------------------------------------------
// Roadmap + placeholders (couches non encore actives)
// ---------------------------------------------------------------------------

/// Affiche la feuille de route OSI (commande `roadmap`, section reseau).
pub fn print_roadmap() {
    vga::set_color(COLOR_CYAN);
    println!("pile reseau OSI:");
    vga::set_color(COLOR_DEFAULT);
    println!("  L2 Ethernet    encode/decode                   [code OK]");
    println!("  ARP            encode/decode                   [code OK]");
    println!("  L3 IPv4        en-tete + checksum              [code OK]");
    println!("  ICMP           echo (ping loopback)            [actif sur lo]");
    println!("  interface lo   127.0.0.1                       [active]");
    println!("  driver NIC     e1000/virtio-net (RX/TX DMA)    [a ecrire]");
    println!("  UDP/DHCP/DNS                                   [planifie]");
    println!("  TCP/HTTP                                       [planifie]");
    println!("  TLS                                            [plus tard]");
}

fn missing_layer(cmd: &str) -> &'static str {
    match cmd {
        "dhcp" => "driver NIC + UDP + client DHCP",
        "dns" => "driver NIC + UDP + resolveur DNS",
        "wget" | "curl" => "driver NIC + TCP + HTTP",
        _ => "driver NIC + couches superieures",
    }
}

/// Message standard d'une commande reseau pas encore active.
pub fn placeholder(cmd: &str) {
    vga::set_color(COLOR_YELLOW);
    println!("{}: non disponible (couches superieures non actives)", cmd);
    vga::set_color(COLOR_DEFAULT);
    println!("  couche manquante: {}", missing_layer(cmd));
    match pci::find_network() {
        Some(d) => println!("  note: carte reseau {:04x}:{:04x} detectee, driver a ecrire", d.vendor, d.device),
        None => println!("  note: aucune carte reseau PCI (essaie QEMU avec -device e1000)"),
    }
}
