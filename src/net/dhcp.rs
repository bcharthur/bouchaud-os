//! Client DHCP (DORA : Discover/Offer/Request/Ack) sur UDP.
//!
//! Obtient automatiquement l'adresse IP, la passerelle et le DNS aupres du
//! serveur DHCP du reseau (SLIRP sous QEMU), puis applique la configuration.

use crate::arch::x86_64::cpu;
use crate::drivers::e1000;
use crate::net::ipv4::Ipv4Addr;
use crate::net::{self, ethernet, ipv4, udp};

const COOKIE: [u8; 4] = [99, 130, 83, 99];

#[derive(Default, Clone, Copy)]
struct Lease {
    msg_type: u8,
    your_ip: Ipv4Addr,
    server_id: Ipv4Addr,
    router: Ipv4Addr,
    dns: Ipv4Addr,
}

/// Construit un message DHCP (BOOTREQUEST). Renvoie la longueur (>= 300).
fn build_msg(buf: &mut [u8], xid: u32, mac: [u8; 6], msg_type: u8,
             req_ip: Option<Ipv4Addr>, server_id: Option<Ipv4Addr>) -> usize {
    for b in buf[..300].iter_mut() { *b = 0; }
    buf[0] = 1;            // op = BOOTREQUEST
    buf[1] = 1;            // htype = Ethernet
    buf[2] = 6;            // hlen
    buf[4..8].copy_from_slice(&xid.to_be_bytes());
    buf[10] = 0x80;        // flags : broadcast (on n'a pas encore d'IP)
    buf[28..34].copy_from_slice(&mac);
    buf[236..240].copy_from_slice(&COOKIE);

    let mut p = 240;
    buf[p] = 53; buf[p + 1] = 1; buf[p + 2] = msg_type; p += 3;
    if let Some(ip) = req_ip {
        buf[p] = 50; buf[p + 1] = 4; buf[p + 2..p + 6].copy_from_slice(&ip); p += 6;
    }
    if let Some(sid) = server_id {
        buf[p] = 54; buf[p + 1] = 4; buf[p + 2..p + 6].copy_from_slice(&sid); p += 6;
    }
    buf[p] = 55; buf[p + 1] = 4; buf[p + 2] = 1; buf[p + 3] = 3; buf[p + 4] = 6; buf[p + 5] = 15; p += 6;
    buf[p] = 255; p += 1; // fin des options
    if p < 300 { 300 } else { p }
}

/// Analyse une reponse DHCP (yiaddr + options utiles).
fn parse_reply(buf: &[u8]) -> Option<Lease> {
    if buf.len() < 240 || buf[236..240] != COOKIE { return None; }
    let mut lease = Lease::default();
    lease.your_ip = [buf[16], buf[17], buf[18], buf[19]];
    let mut p = 240;
    while p + 1 < buf.len() {
        let code = buf[p];
        if code == 255 { break; }
        if code == 0 { p += 1; continue; }
        let len = buf[p + 1] as usize;
        let data = p + 2;
        if data + len > buf.len() { break; }
        match code {
            53 if len >= 1 => lease.msg_type = buf[data],
            54 if len >= 4 => lease.server_id = [buf[data], buf[data + 1], buf[data + 2], buf[data + 3]],
            3 if len >= 4 => lease.router = [buf[data], buf[data + 1], buf[data + 2], buf[data + 3]],
            6 if len >= 4 => lease.dns = [buf[data], buf[data + 1], buf[data + 2], buf[data + 3]],
            _ => {}
        }
        p = data + len;
    }
    Some(lease)
}

/// Emet un message DHCP en diffusion (broadcast L2 + IP).
fn send(mac: [u8; 6], msg: &[u8]) -> bool {
    let mut udp_buf = [0u8; 600];
    let ulen = match udp::build(&mut udp_buf, 68, 67, msg) { Some(n) => n, None => return false };
    let mut ip = [0u8; 700];
    let ipl = match ipv4::build_packet(&mut ip, [0, 0, 0, 0], [255, 255, 255, 255], ipv4::PROTO_UDP, 0, &udp_buf[..ulen]) {
        Some(n) => n, None => return false,
    };
    let mut frame = [0u8; 760];
    let fl = match ethernet::build_frame(&mut frame, ethernet::BROADCAST, mac, ethernet::ETHERTYPE_IPV4, &ip[..ipl]) {
        Some(n) => n, None => return false,
    };
    e1000::send(&frame[..fl])
}

/// Attend une reponse DHCP du bon xid (et type voulu si != 0).
fn recv(xid: u32, want_type: u8) -> Option<Lease> {
    let mut buf = [0u8; 2048];
    for _ in 0..8_000_000u32 {
        let n = match e1000::receive(&mut buf) { Some(n) => n, None => continue };
        let h = match ethernet::parse_header(&buf[..n]) { Some(h) => h, None => continue };
        if h.ethertype != ethernet::ETHERTYPE_IPV4 { continue; }
        let iph = match ipv4::parse_header(&buf[ethernet::HEADER_LEN..n]) { Some(i) => i, None => continue };
        if iph.proto != ipv4::PROTO_UDP { continue; }
        let uoff = ethernet::HEADER_LEN + iph.header_len;
        if uoff + 8 > n { continue; }
        let u = match udp::parse(&buf[uoff..n]) { Some(u) => u, None => continue };
        if u.dst_port != 68 { continue; }
        let doff = uoff + u.payload_off;
        if doff + 8 > n { continue; }
        // verifie xid + BOOTREPLY
        if buf[doff] != 2 { continue; }
        let rxid = u32::from_be_bytes([buf[doff + 4], buf[doff + 5], buf[doff + 6], buf[doff + 7]]);
        if rxid != xid { continue; }
        if let Some(l) = parse_reply(&buf[doff..n]) {
            if want_type == 0 || l.msg_type == want_type {
                return Some(l);
            }
        }
    }
    None
}

/// Commande `dhcp` : configuration automatique de l'interface.
pub fn run() {
    if !e1000::is_ready() && !e1000::init() {
        crate::println!("dhcp: carte reseau indisponible (essaie 'ifup')");
        return;
    }
    let mac = e1000::mac();
    let xid = cpu::rdtsc() as u32;
    let mut msg = [0u8; 400];

    // DISCOVER
    let l = build_msg(&mut msg, xid, mac, 1, None, None);
    send(mac, &msg[..l]);
    crate::println!("DHCP: DISCOVER envoye...");
    let offer = match recv(xid, 2) {
        Some(o) => o,
        None => { crate::println!("dhcp: pas d'OFFER (timeout)"); return; }
    };
    crate::print!("DHCP: OFFER "); ipv4::print_addr(&offer.your_ip); crate::println!("");

    // REQUEST
    let l = build_msg(&mut msg, xid, mac, 3, Some(offer.your_ip), Some(offer.server_id));
    send(mac, &msg[..l]);
    let ack = match recv(xid, 5) {
        Some(a) => a,
        None => { crate::println!("dhcp: pas d'ACK (timeout)"); return; }
    };

    // Applique la configuration (avec valeurs de repli).
    let gw = if ack.router == [0, 0, 0, 0] { net::gateway() } else { ack.router };
    let dns = if ack.dns == [0, 0, 0, 0] { net::dns_server() } else { ack.dns };
    net::set_config(ack.your_ip, gw, dns);

    crate::print!("DHCP: bail obtenu  inet "); ipv4::print_addr(&ack.your_ip);
    crate::print!("  gw "); ipv4::print_addr(&gw);
    crate::print!("  dns "); ipv4::print_addr(&dns);
    crate::println!("");
}
