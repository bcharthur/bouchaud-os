//! Moteur de pile : traitement d'un paquet IPv4 entrant.
//!
//! C'est le code reel partage par l'interface loopback (actif aujourd'hui) et,
//! plus tard, par le driver de carte reseau. Aujourd'hui il sait repondre a une
//! requete ICMP echo (ping).

use crate::net::{icmp, ipv4};

/// Traite un paquet IPv4 et, si une reponse doit etre emise, l'ecrit dans `out`.
/// Renvoie la longueur de la reponse, ou `None` si aucune reponse.
pub fn handle_ipv4(packet: &[u8], out: &mut [u8]) -> Option<usize> {
    let hdr = ipv4::parse_header(packet)?;
    // No IPv4 reassembly in this stack: a partial fragment is not an ICMP
    // packet, even if its payload happens to resemble an Echo Request.
    let fragment = u16::from_be_bytes([packet[6], packet[7]]);
    if fragment & 0x3fff != 0 { return None; } // MF or fragment offset
    let payload = &packet[hdr.header_len..hdr.total_len];

    match hdr.proto {
        ipv4::PROTO_ICMP => {
            // ICMP checksum is mandatory. A malformed packet must never
            // trigger a reply or be treated as a valid ping.
            if ipv4::checksum(payload) != 0 { return None; }
            let msg = icmp::parse(payload)?;
            if msg.msg_type != icmp::ECHO_REQUEST {
                return None;
            }
            // Construit une reponse echo reply : on renvoie la meme charge utile.
            let icmp_payload = &payload[8..];
            let mut reply_icmp = [0u8; 64];
            let icmp_len = icmp::build(&mut reply_icmp, icmp::ECHO_REPLY, msg.id, msg.seq, icmp_payload)?;
            // On inverse source et destination.
            ipv4::build_packet(out, hdr.dst, hdr.src, ipv4::PROTO_ICMP, 0, &reply_icmp[..icmp_len])
        }
        _ => None,
    }
}
