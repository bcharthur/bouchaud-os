//! Couche UDP (datagrammes). Checksum mis a 0 (autorise en IPv4) pour rester
//! simple : SLIRP/QEMU l'accepte.

/// Construit un datagramme UDP (en-tete 8 octets + charge utile).
pub fn build(buf: &mut [u8], src_port: u16, dst_port: u16, payload: &[u8]) -> Option<usize> {
    let total = 8 + payload.len();
    if buf.len() < total { return None; }
    buf[0] = (src_port >> 8) as u8;
    buf[1] = src_port as u8;
    buf[2] = (dst_port >> 8) as u8;
    buf[3] = dst_port as u8;
    buf[4] = (total >> 8) as u8;
    buf[5] = total as u8;
    buf[6] = 0; // checksum desactive
    buf[7] = 0;
    buf[8..total].copy_from_slice(payload);
    Some(total)
}

/// En-tete UDP decode.
pub struct Header {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload_off: usize,
    pub payload_len: usize,
}

/// Decode un datagramme UDP.
pub fn parse(buf: &[u8]) -> Option<Header> {
    if buf.len() < 8 { return None; }
    let len = ((buf[4] as usize) << 8) | buf[5] as usize;
    if len < 8 || len > buf.len() { return None; }
    Some(Header {
        src_port: ((buf[0] as u16) << 8) | buf[1] as u16,
        dst_port: ((buf[2] as u16) << 8) | buf[3] as u16,
        payload_off: 8,
        payload_len: len - 8,
    })
}
