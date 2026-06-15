//! Couche ICMP : echo request / echo reply (le coeur de `ping`).

use crate::net::ipv4;

pub const ECHO_REPLY: u8 = 0;
pub const ECHO_REQUEST: u8 = 8;

/// Construit un message ICMP (type/code + id/seq + payload) dans `buf`.
/// Renvoie la longueur du message ICMP ecrit.
pub fn build(buf: &mut [u8], msg_type: u8, id: u16, seq: u16, payload: &[u8]) -> Option<usize> {
    let total = 8 + payload.len();
    if buf.len() < total { return None; }
    buf[0] = msg_type;
    buf[1] = 0; // code
    buf[2] = 0; // checksum (calcule ensuite)
    buf[3] = 0;
    buf[4] = (id >> 8) as u8;
    buf[5] = id as u8;
    buf[6] = (seq >> 8) as u8;
    buf[7] = seq as u8;
    buf[8..total].copy_from_slice(payload);

    let csum = ipv4::checksum(&buf[..total]);
    buf[2] = (csum >> 8) as u8;
    buf[3] = csum as u8;
    Some(total)
}

/// Message ICMP decode.
pub struct Message {
    pub msg_type: u8,
    pub id: u16,
    pub seq: u16,
}

/// Decode un message ICMP.
pub fn parse(buf: &[u8]) -> Option<Message> {
    if buf.len() < 8 { return None; }
    Some(Message {
        msg_type: buf[0],
        id: ((buf[4] as u16) << 8) | buf[5] as u16,
        seq: ((buf[6] as u16) << 8) | buf[7] as u16,
    })
}
