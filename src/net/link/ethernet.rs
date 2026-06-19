//! Couche 2 : trames Ethernet (encodage/decodage).
//!
//! Logique reelle prete pour le futur driver de carte reseau. Non encore
//! emise sur le fil tant que le driver e1000/virtio-net n'est pas ecrit.

/// Adresse MAC (6 octets).
pub type MacAddr = [u8; 6];

pub const BROADCAST: MacAddr = [0xFF; 6];

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;

pub const HEADER_LEN: usize = 14;

/// Construit une trame Ethernet (en-tete + charge utile) dans `buf`.
pub fn build_frame(buf: &mut [u8], dst: MacAddr, src: MacAddr, ethertype: u16, payload: &[u8]) -> Option<usize> {
    let total = HEADER_LEN + payload.len();
    if buf.len() < total { return None; }
    buf[0..6].copy_from_slice(&dst);
    buf[6..12].copy_from_slice(&src);
    buf[12] = (ethertype >> 8) as u8;
    buf[13] = ethertype as u8;
    buf[HEADER_LEN..total].copy_from_slice(payload);
    Some(total)
}

/// En-tete Ethernet decode.
pub struct Header {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ethertype: u16,
}

/// Decode l'en-tete d'une trame Ethernet.
pub fn parse_header(buf: &[u8]) -> Option<Header> {
    if buf.len() < HEADER_LEN { return None; }
    let mut dst = [0u8; 6];
    let mut src = [0u8; 6];
    dst.copy_from_slice(&buf[0..6]);
    src.copy_from_slice(&buf[6..12]);
    Some(Header { dst, src, ethertype: ((buf[12] as u16) << 8) | buf[13] as u16 })
}

/// Affiche une adresse MAC sous forme "aa:bb:cc:dd:ee:ff".
pub fn print_mac(mac: &MacAddr) {
    crate::print!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
}
