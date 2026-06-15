//! Protocole ARP : resolution IPv4 -> MAC (encodage/decodage).
//!
//! Logique reelle prete pour le driver reseau. Le cache ARP est pour l'instant
//! vide tant qu'aucune trame n'est emise.

use crate::net::ethernet::MacAddr;
use crate::net::ipv4::Ipv4Addr;

pub const OP_REQUEST: u16 = 1;
pub const OP_REPLY: u16 = 2;
pub const PACKET_LEN: usize = 28;

/// Construit un paquet ARP (Ethernet/IPv4) dans `buf`.
pub fn build(buf: &mut [u8], op: u16, sender_mac: MacAddr, sender_ip: Ipv4Addr, target_mac: MacAddr, target_ip: Ipv4Addr) -> Option<usize> {
    if buf.len() < PACKET_LEN { return None; }
    buf[0] = 0x00; buf[1] = 0x01;       // type materiel : Ethernet
    buf[2] = 0x08; buf[3] = 0x00;       // type protocole : IPv4
    buf[4] = 6;                          // taille MAC
    buf[5] = 4;                          // taille IP
    buf[6] = (op >> 8) as u8; buf[7] = op as u8;
    buf[8..14].copy_from_slice(&sender_mac);
    buf[14..18].copy_from_slice(&sender_ip);
    buf[18..24].copy_from_slice(&target_mac);
    buf[24..28].copy_from_slice(&target_ip);
    Some(PACKET_LEN)
}

/// Paquet ARP decode.
pub struct Packet {
    pub op: u16,
    pub sender_mac: MacAddr,
    pub sender_ip: Ipv4Addr,
    pub target_ip: Ipv4Addr,
}

/// Decode un paquet ARP.
pub fn parse(buf: &[u8]) -> Option<Packet> {
    if buf.len() < PACKET_LEN { return None; }
    let op = ((buf[6] as u16) << 8) | buf[7] as u16;
    let mut sender_mac = [0u8; 6];
    let mut sender_ip = [0u8; 4];
    let mut target_ip = [0u8; 4];
    sender_mac.copy_from_slice(&buf[8..14]);
    sender_ip.copy_from_slice(&buf[14..18]);
    target_ip.copy_from_slice(&buf[24..28]);
    Some(Packet { op, sender_mac, sender_ip, target_ip })
}
