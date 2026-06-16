//! Client TCP minimal (suffisant pour un GET HTTP).
//!
//! Implemente la poignee de main (SYN/SYN-ACK/ACK), l'envoi d'une requete et la
//! reception/ack des segments jusqu'au FIN. Pas de retransmission ni de controle
//! de congestion : suffisant pour de petites pages via SLIRP.

use crate::arch::x86_64::cpu;
use crate::net::ipv4::Ipv4Addr;
use crate::net::{self, ETH_IP};
use alloc::vec::Vec;

const FIN: u8 = 0x01;
const SYN: u8 = 0x02;
const RST: u8 = 0x04;
const PSH: u8 = 0x08;
const ACK: u8 = 0x10;

fn checksum(src: &Ipv4Addr, dst: &Ipv4Addr, seg: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    sum += ((src[0] as u32) << 8) | src[1] as u32;
    sum += ((src[2] as u32) << 8) | src[3] as u32;
    sum += ((dst[0] as u32) << 8) | dst[1] as u32;
    sum += ((dst[2] as u32) << 8) | dst[3] as u32;
    sum += 6; // proto TCP
    sum += seg.len() as u32;
    let mut i = 0;
    while i + 1 < seg.len() {
        sum += ((seg[i] as u32) << 8) | seg[i + 1] as u32;
        i += 2;
    }
    if i < seg.len() {
        sum += (seg[i] as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[allow(clippy::too_many_arguments)]
fn build(buf: &mut [u8], dst: &Ipv4Addr, sport: u16, dport: u16, seq: u32, ack: u32,
         flags: u8, window: u16, payload: &[u8]) -> usize {
    let total = 20 + payload.len();
    buf[0] = (sport >> 8) as u8; buf[1] = sport as u8;
    buf[2] = (dport >> 8) as u8; buf[3] = dport as u8;
    buf[4..8].copy_from_slice(&seq.to_be_bytes());
    buf[8..12].copy_from_slice(&ack.to_be_bytes());
    buf[12] = 0x50; // data offset = 5 mots (20 octets)
    buf[13] = flags;
    buf[14] = (window >> 8) as u8; buf[15] = window as u8;
    buf[16] = 0; buf[17] = 0; // checksum
    buf[18] = 0; buf[19] = 0; // urgent
    buf[20..total].copy_from_slice(payload);
    let c = checksum(&ETH_IP, dst, &buf[..total]);
    buf[16] = (c >> 8) as u8; buf[17] = c as u8;
    total
}

struct Seg {
    sport: u16,
    dport: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    data_off: usize,
}

fn parse(seg: &[u8]) -> Option<Seg> {
    if seg.len() < 20 { return None; }
    let data_off = ((seg[12] >> 4) as usize) * 4;
    if data_off < 20 || data_off > seg.len() { return None; }
    Some(Seg {
        sport: ((seg[0] as u16) << 8) | seg[1] as u16,
        dport: ((seg[2] as u16) << 8) | seg[3] as u16,
        seq: u32::from_be_bytes([seg[4], seg[5], seg[6], seg[7]]),
        ack: u32::from_be_bytes([seg[8], seg[9], seg[10], seg[11]]),
        flags: seg[13],
        data_off,
    })
}

const WINDOW: u16 = 64240;

/// Ouvre une connexion, envoie `request`, accumule la reponse dans `out`.
/// Renvoie true si la poignee de main a reussi.
pub fn fetch(dst: Ipv4Addr, port: u16, request: &[u8], out: &mut Vec<u8>) -> bool {
    let sport = 0xC000u16 | (cpu::rdtsc() as u16 & 0x0FFF);
    let isn = cpu::rdtsc() as u32;
    let mut seg = [0u8; 1600];
    let mut rb = [0u8; 2048];

    // --- SYN ---
    let l = build(&mut seg, &dst, sport, port, isn, 0, SYN, WINDOW, &[]);
    net::send_ip(dst, 6, &seg[..l]);

    // --- attend SYN-ACK ---
    let mut their_seq = 0u32;
    let mut established = false;
    for _ in 0..8_000_000u32 {
        if let Some((_, n)) = net::poll_ip(6, Some(dst), &mut rb) {
            if let Some(h) = parse(&rb[..n]) {
                if h.dport == sport {
                    if h.flags & RST != 0 { return false; }
                    if h.flags & SYN != 0 && h.flags & ACK != 0 {
                        their_seq = h.seq;
                        established = true;
                        break;
                    }
                }
            }
        }
    }
    if !established { return false; }

    let my_seq = isn.wrapping_add(1);
    let mut my_ack = their_seq.wrapping_add(1);

    // --- ACK de la poignee + envoi de la requete (PSH|ACK) ---
    let l = build(&mut seg, &dst, sport, port, my_seq, my_ack, ACK, WINDOW, &[]);
    net::send_ip(dst, 6, &seg[..l]);
    let l = build(&mut seg, &dst, sport, port, my_seq, my_ack, PSH | ACK, WINDOW, request);
    net::send_ip(dst, 6, &seg[..l]);
    let mut my_seq = my_seq.wrapping_add(request.len() as u32);

    // --- reception ---
    let mut idle = 0u32;
    loop {
        let mut activity = false;
        for _ in 0..2_000_000u32 {
            if let Some((_, n)) = net::poll_ip(6, Some(dst), &mut rb) {
                if let Some(h) = parse(&rb[..n]) {
                    if h.dport != sport { continue; }
                    activity = true;
                    let plen = n - h.data_off;
                    if plen > 0 {
                        if h.seq == my_ack {
                            out.extend_from_slice(&rb[h.data_off..n]);
                            my_ack = my_ack.wrapping_add(plen as u32);
                        }
                        // ACK (ou re-ACK si hors sequence)
                        let a = build(&mut seg, &dst, sport, port, my_seq, my_ack, ACK, WINDOW, &[]);
                        net::send_ip(dst, 6, &seg[..a]);
                    }
                    if h.flags & FIN != 0 {
                        my_ack = my_ack.wrapping_add(1);
                        let a = build(&mut seg, &dst, sport, port, my_seq, my_ack, ACK, WINDOW, &[]);
                        net::send_ip(dst, 6, &seg[..a]);
                        let f = build(&mut seg, &dst, sport, port, my_seq, my_ack, FIN | ACK, WINDOW, &[]);
                        net::send_ip(dst, 6, &seg[..f]);
                        return true;
                    }
                    if h.flags & RST != 0 { return true; }
                    break;
                }
            }
        }
        if out.len() > 200_000 { break; }
        if activity { idle = 0; } else { idle += 1; if idle >= 3 { break; } }
    }

    // Fermeture propre.
    let f = build(&mut seg, &dst, sport, port, my_seq, my_ack, FIN | ACK, WINDOW, &[]);
    net::send_ip(dst, 6, &seg[..f]);
    let _ = &mut my_seq;
    true
}
