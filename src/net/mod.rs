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
pub mod udp;
pub mod dns;
pub mod tcp;
pub mod http;

use crate::arch::x86_64::pci;
use crate::drivers::e1000;
use crate::drivers::vga::{self, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW, COLOR_DEFAULT};
use alloc::format;
use alloc::string::String;
use crate::net::ipv4::Ipv4Addr;

/// Adresse de l'interface loopback.
pub const LO_ADDR: Ipv4Addr = [127, 0, 0, 1];
/// Adresse IPv4 statique d'eth0 (reseau utilisateur QEMU SLIRP).
pub const ETH_IP: Ipv4Addr = [10, 0, 2, 15];
/// Passerelle par defaut (SLIRP).
pub const GATEWAY: Ipv4Addr = [10, 0, 2, 2];

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

/// Resout l'adresse MAC d'une IP via ARP. Renvoie None en cas de timeout.
fn arp_resolve(target: Ipv4Addr) -> Option<[u8; 6]> {
    let mac = e1000::mac();
    let mut arp_buf = [0u8; arp::PACKET_LEN];
    arp::build(&mut arp_buf, arp::OP_REQUEST, mac, ETH_IP, [0; 6], target)?;
    let mut frame = [0u8; ethernet::HEADER_LEN + arp::PACKET_LEN];
    let flen = ethernet::build_frame(&mut frame, ethernet::BROADCAST, mac, ethernet::ETHERTYPE_ARP, &arp_buf)?;
    e1000::send(&frame[..flen]);

    let mut buf = [0u8; 2048];
    for _ in 0..3_000_000u32 {
        if let Some(n) = e1000::receive(&mut buf) {
            if n >= ethernet::HEADER_LEN + arp::PACKET_LEN {
                if let Some(h) = ethernet::parse_header(&buf[..n]) {
                    if h.ethertype == ethernet::ETHERTYPE_ARP {
                        if let Some(p) = arp::parse(&buf[ethernet::HEADER_LEN..n]) {
                            if p.op == arp::OP_REPLY && p.sender_ip == target {
                                return Some(p.sender_mac);
                            }
                        }
                    }
                }
            }
        }
    }
    None
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
    crate::print!("ARP qui a "); ipv4::print_addr(&target); println!(" ?");
    match arp_resolve(target) {
        Some(m) => {
            vga::set_color(COLOR_GREEN);
            crate::print!("reponse de "); ipv4::print_addr(&target);
            println!(" : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                m[0], m[1], m[2], m[3], m[4], m[5]);
            vga::set_color(COLOR_DEFAULT);
        }
        None => println!("arping: pas de reponse (timeout)"),
    }
}

/// Meme reseau /24 que eth0 ?
fn same_subnet(ip: &Ipv4Addr) -> bool {
    ip[0] == ETH_IP[0] && ip[1] == ETH_IP[1] && ip[2] == ETH_IP[2]
}

/// Serveur DNS (resolveur SLIRP de QEMU).
pub const DNS_SERVER: Ipv4Addr = [10, 0, 2, 3];

static mut IP_ID: u16 = 0x4000;
static mut GW_MAC: Option<[u8; 6]> = None;

fn next_ip_id() -> u16 {
    unsafe { IP_ID = IP_ID.wrapping_add(1); IP_ID }
}

/// MAC du prochain saut pour atteindre `dst` (cache la MAC de la passerelle).
fn hop_mac(dst: &Ipv4Addr) -> Option<[u8; 6]> {
    if same_subnet(dst) {
        return arp_resolve(*dst);
    }
    unsafe {
        if let Some(m) = GW_MAC { return Some(m); }
        let m = arp_resolve(GATEWAY)?;
        GW_MAC = Some(m);
        Some(m)
    }
}

/// Emet un paquet IPv4 (`proto`/`payload`) vers `dst` via e1000.
pub(crate) fn send_ip(dst: Ipv4Addr, proto: u8, payload: &[u8]) -> bool {
    if !e1000::is_ready() && !e1000::init() { return false; }
    let mac = match hop_mac(&dst) { Some(m) => m, None => return false };
    let mut ip = [0u8; 1500];
    let ipl = match ipv4::build_packet(&mut ip, ETH_IP, dst, proto, next_ip_id(), payload) {
        Some(n) => n, None => return false,
    };
    let mut frame = [0u8; 1514];
    let fl = match ethernet::build_frame(&mut frame, mac, e1000::mac(), ethernet::ETHERTYPE_IPV4, &ip[..ipl]) {
        Some(n) => n, None => return false,
    };
    e1000::send(&frame[..fl])
}

/// Recoit un paquet IPv4 du protocole `proto` (et source optionnelle). Copie la
/// charge utile dans `out`, renvoie (source, longueur). Non bloquant.
pub(crate) fn poll_ip(proto: u8, src_filter: Option<Ipv4Addr>, out: &mut [u8]) -> Option<(Ipv4Addr, usize)> {
    let mut buf = [0u8; 2048];
    let n = e1000::receive(&mut buf)?;
    let h = ethernet::parse_header(&buf[..n])?;
    if h.ethertype != ethernet::ETHERTYPE_IPV4 { return None; }
    let iph = ipv4::parse_header(&buf[ethernet::HEADER_LEN..n])?;
    if iph.proto != proto { return None; }
    if let Some(s) = src_filter {
        if iph.src != s { return None; }
    }
    let start = ethernet::HEADER_LEN + iph.header_len;
    let end = ethernet::HEADER_LEN + iph.total_len;
    if start > end || end > n { return None; }
    let len = end - start;
    let m = len.min(out.len());
    out[..m].copy_from_slice(&buf[start..start + m]);
    Some((iph.src, m))
}

/// Resout un nom d'hote en IPv4 via DNS (None en cas d'echec/timeout).
pub fn resolve(name: &str) -> Option<Ipv4Addr> {
    // Deja une IP ?
    if let Some(ip) = ipv4::parse_addr(name) { return Some(ip); }
    if !e1000::is_ready() && !e1000::init() { return None; }

    let id = (next_ip_id() ^ 0x1234) as u16;
    let mut q = [0u8; 256];
    let qlen = dns::build_query(&mut q, id, name)?;
    let mut udp_buf = [0u8; 300];
    let ulen = udp::build(&mut udp_buf, 0xC000, 53, &q[..qlen])?;
    send_ip(DNS_SERVER, ipv4::PROTO_UDP, &udp_buf[..ulen]);

    let mut payload = [0u8; 1500];
    for _ in 0..4_000_000u32 {
        if let Some((src, n)) = poll_ip(ipv4::PROTO_UDP, Some(DNS_SERVER), &mut payload) {
            let _ = src;
            if let Some(u) = udp::parse(&payload[..n]) {
                if u.dst_port == 0xC000 {
                    let off = u.payload_off;
                    if let Some(ip) = dns::parse_response(&payload[off..off + u.payload_len], id) {
                        return Some(ip);
                    }
                }
            }
        }
    }
    None
}

/// Commande `dns <nom>` / `nslookup`.
pub fn dns_cmd(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: dns <nom>"); return; }
    if !e1000::is_ready() && !e1000::init() {
        println!("dns: carte reseau indisponible (essaie 'ifup')");
        return;
    }
    match resolve(argv[1]) {
        Some(ip) => { crate::print!("{} -> ", argv[1]); ipv4::print_addr(&ip); println!(""); }
        None => println!("dns: pas de reponse pour {}", argv[1]),
    }
}

/// Recupere une URL HTTP et renvoie les lignes a afficher (statut + corps).
/// Utilise par la commande `wget` et par le navigateur.
pub fn http_get(url: &str) -> alloc::vec::Vec<String> {
    use alloc::string::ToString;
    let mut out: alloc::vec::Vec<String> = alloc::vec::Vec::new();

    let rest = if let Some(r) = url.strip_prefix("http://") {
        r
    } else if url.starts_with("https://") {
        out.push("HTTPS non supporte : TLS pas encore implemente.".to_string());
        out.push("Utilise http:// pour l'instant.".to_string());
        return out;
    } else {
        url
    };

    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (hostname, port) = match host.find(':') {
        Some(i) => (&host[..i], host[i + 1..].parse::<u16>().unwrap_or(80)),
        None => (host, 80u16),
    };

    if !e1000::is_ready() && !e1000::init() {
        out.push("reseau indisponible (lance 'ifup')".to_string());
        return out;
    }
    let ip = match resolve(hostname) {
        Some(ip) => ip,
        None => { out.push(format!("DNS: echec pour {}", hostname)); return out; }
    };

    let req = http::build_get(hostname, path);
    let mut resp: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if !tcp::fetch(ip, port, req.as_bytes(), &mut resp) {
        out.push(format!("connexion TCP echouee vers {}:{}", hostname, port));
        return out;
    }
    if resp.is_empty() {
        out.push("reponse vide".to_string());
        return out;
    }

    // Ligne de statut (premiere ligne de la reponse).
    let mut status_end = 0;
    while status_end < resp.len() && resp[status_end] != b'\r' && resp[status_end] != b'\n' {
        status_end += 1;
    }
    let mut status = String::new();
    for &b in &resp[..status_end] { status.push(b as char); }
    out.push(status);

    // Corps.
    let body_off = http::body_offset(&resp).unwrap_or(0);
    let mut line = String::new();
    for &b in &resp[body_off..] {
        match b {
            b'\n' => { out.push(core::mem::take(&mut line)); if out.len() > 200 { break; } }
            b'\r' => {}
            0x20..=0x7e => line.push(b as char),
            _ => line.push('.'),
        }
    }
    if !line.is_empty() { out.push(line); }
    out
}

/// Commande `wget`/`curl`/`http <url>`.
pub fn wget_cmd(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: wget http://hote/chemin"); return; }
    for l in http_get(argv[1]) {
        println!("{}", l);
    }
}

/// Ping reel via e1000 : ARP -> ICMP echo sur 4 paquets.
fn ping_remote(target: Ipv4Addr) {
    // Adresse de niveau lien : la cible si locale, sinon la passerelle.
    let next_hop = if same_subnet(&target) { target } else { GATEWAY };
    let dst_mac = match arp_resolve(next_hop) {
        Some(m) => m,
        None => {
            vga::set_color(COLOR_YELLOW);
            crate::print!("ping: ARP sans reponse pour "); ipv4::print_addr(&next_hop); println!("");
            vga::set_color(COLOR_DEFAULT);
            return;
        }
    };
    let our_mac = e1000::mac();
    let payload = b"bouchaud-os-ping";
    let id = 0x4243u16;
    let mut sent = 0u32;
    let mut recv = 0u32;

    for seq in 0..4u16 {
        let mut icmp_buf = [0u8; 64];
        let il = match icmp::build(&mut icmp_buf, icmp::ECHO_REQUEST, id, seq, payload) {
            Some(n) => n, None => continue,
        };
        let mut ip_buf = [0u8; 128];
        let ipl = match ipv4::build_packet(&mut ip_buf, ETH_IP, target, ipv4::PROTO_ICMP, seq, &icmp_buf[..il]) {
            Some(n) => n, None => continue,
        };
        let mut frame = [0u8; ethernet::HEADER_LEN + 128];
        let fl = match ethernet::build_frame(&mut frame, dst_mac, our_mac, ethernet::ETHERTYPE_IPV4, &ip_buf[..ipl]) {
            Some(n) => n, None => continue,
        };
        e1000::send(&frame[..fl]);
        sent += 1;

        // Attend l'echo reply correspondant.
        let mut buf = [0u8; 2048];
        let mut got = false;
        for _ in 0..3_000_000u32 {
            if let Some(n) = e1000::receive(&mut buf) {
                if let Some(h) = ethernet::parse_header(&buf[..n]) {
                    if h.ethertype == ethernet::ETHERTYPE_IPV4 {
                        if let Some(iph) = ipv4::parse_header(&buf[ethernet::HEADER_LEN..n]) {
                            if iph.proto == ipv4::PROTO_ICMP && iph.src == target {
                                let off = ethernet::HEADER_LEN + iph.header_len;
                                if off < n {
                                    if let Some(m) = icmp::parse(&buf[off..n]) {
                                        if m.msg_type == icmp::ECHO_REPLY && m.id == id && m.seq == seq {
                                            recv += 1;
                                            crate::print!("reponse de "); ipv4::print_addr(&target);
                                            println!(" : icmp_seq={} ttl=64", seq);
                                            got = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !got {
            println!("  delai depasse pour icmp_seq={}", seq);
        }
    }
    let lost = if sent > 0 { (sent - recv) * 100 / sent } else { 100 };
    println!("--- statistiques ping ---");
    println!("{} transmis, {} recus, {}% perdus", sent, recv, lost);
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
    } else if e1000::is_ready() || e1000::init() {
        // Ping reel via la carte e1000.
        ping_remote(target);
    } else {
        vga::set_color(COLOR_YELLOW);
        println!("ping: carte reseau indisponible");
        vga::set_color(COLOR_DEFAULT);
        match pci::find_network() {
            Some(d) => println!("  eth0 {:04x}:{:04x} detectee ; lance 'ifup' (QEMU -device e1000 -netdev user,id=n0)", d.vendor, d.device),
            None => println!("  aucune carte reseau PCI; lance QEMU avec -device e1000 -netdev user,id=n0"),
        }
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
