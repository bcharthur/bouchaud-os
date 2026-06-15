//! Couche 3 : IPv4 (adresses, en-tete, checksum).
//!
//! Encodage/decodage reels d'un en-tete IPv4 dans des tampons d'octets, sans
//! allocation. Utilise par la pile loopback (`net::stack`) et, a terme, par le
//! futur driver de carte reseau.

/// Adresse IPv4 (4 octets).
pub type Ipv4Addr = [u8; 4];

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_UDP: u8 = 17;
pub const PROTO_TCP: u8 = 6;

pub const HEADER_LEN: usize = 20;

/// Calcule le checksum Internet (complement a un sur 16 bits) d'un tampon.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | data[i + 1] as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Analyse une adresse IPv4 textuelle "a.b.c.d".
pub fn parse_addr(s: &str) -> Option<Ipv4Addr> {
    let mut addr = [0u8; 4];
    let mut part = 0usize;
    let mut value: u32 = 0;
    let mut seen = false;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                value = value * 10 + (b - b'0') as u32;
                if value > 255 { return None; }
                seen = true;
            }
            b'.' => {
                if !seen || part >= 3 { return None; }
                addr[part] = value as u8;
                part += 1;
                value = 0;
                seen = false;
            }
            _ => return None,
        }
    }
    if !seen || part != 3 { return None; }
    addr[3] = value as u8;
    Some(addr)
}

/// Affiche une adresse IPv4 sous forme "a.b.c.d".
pub fn print_addr(addr: &Ipv4Addr) {
    crate::print!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3]);
}

/// Indique si l'adresse appartient au reseau loopback 127.0.0.0/8.
pub fn is_loopback(addr: &Ipv4Addr) -> bool {
    addr[0] == 127
}

/// Construit un en-tete IPv4 + place la charge utile a la suite.
/// Renvoie la longueur totale du paquet ecrit dans `buf`.
pub fn build_packet(buf: &mut [u8], src: Ipv4Addr, dst: Ipv4Addr, proto: u8, ident: u16, payload: &[u8]) -> Option<usize> {
    let total = HEADER_LEN + payload.len();
    if buf.len() < total { return None; }

    for b in buf[..HEADER_LEN].iter_mut() { *b = 0; }
    buf[0] = 0x45; // version 4, IHL 5 (20 octets)
    buf[1] = 0x00; // DSCP/ECN
    buf[2] = (total >> 8) as u8;
    buf[3] = total as u8;
    buf[4] = (ident >> 8) as u8;
    buf[5] = ident as u8;
    buf[6] = 0x40; // flag "don't fragment"
    buf[7] = 0x00;
    buf[8] = 64;   // TTL
    buf[9] = proto;
    buf[12..16].copy_from_slice(&src);
    buf[16..20].copy_from_slice(&dst);

    let csum = checksum(&buf[..HEADER_LEN]);
    buf[10] = (csum >> 8) as u8;
    buf[11] = csum as u8;

    buf[HEADER_LEN..total].copy_from_slice(payload);
    Some(total)
}

/// En-tete IPv4 decode.
pub struct Header {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub proto: u8,
    pub header_len: usize,
    pub total_len: usize,
}

/// Decode un en-tete IPv4 en debut de tampon.
pub fn parse_header(buf: &[u8]) -> Option<Header> {
    if buf.len() < HEADER_LEN { return None; }
    if (buf[0] >> 4) != 4 { return None; }
    let ihl = (buf[0] & 0x0F) as usize * 4;
    if ihl < HEADER_LEN || buf.len() < ihl { return None; }
    let total_len = ((buf[2] as usize) << 8) | buf[3] as usize;
    let mut src = [0u8; 4];
    let mut dst = [0u8; 4];
    src.copy_from_slice(&buf[12..16]);
    dst.copy_from_slice(&buf[16..20]);
    Some(Header { src, dst, proto: buf[9], header_len: ihl, total_len })
}
