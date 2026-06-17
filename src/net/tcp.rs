//! Client TCP minimal (suffisant pour un GET HTTP).
//!
//! Implemente la poignee de main (SYN/SYN-ACK/ACK), l'envoi d'une requete et la
//! reception/ack des segments jusqu'au FIN. Pas de retransmission ni de controle
//! de congestion : suffisant pour de petites pages via SLIRP.

use crate::arch::x86_64::cpu;
use crate::net::ipv4::Ipv4Addr;
use crate::net;
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
    let c = checksum(&net::our_ip(), dst, &buf[..total]);
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

struct PendingSeg {
    seq: u32,
    data: Vec<u8>,
}

fn seq_less(a: u32, b: u32) -> bool {
    (a as i32).wrapping_sub(b as i32) < 0
}

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

/// Connexion TCP avec etat (necessaire pour TLS : plusieurs envois/receptions).
pub struct TcpConn {
    dst: Ipv4Addr,
    sport: u16,
    dport: u16,
    seq: u32,         // notre prochain numero de sequence
    ack: u32,         // prochain octet attendu du pair
    pub rx: Vec<u8>,  // donnees applicatives recues, en attente de lecture
    ooo: Vec<PendingSeg>, // petits segments arrives hors ordre, remis en ordre ensuite
    pub peer_fin: bool,
    pub closed: bool,
}

impl TcpConn {
    /// Ouvre une connexion (poignee SYN/SYN-ACK/ACK).
    pub fn connect(dst: Ipv4Addr, port: u16) -> Option<TcpConn> {
        let sport = 0xC000u16 | (cpu::rdtsc() as u16 & 0x0FFF);
        let isn = cpu::rdtsc() as u32;
        let mut seg = [0u8; 64];
        let mut rb = [0u8; 2048];

        let l = build(&mut seg, &dst, sport, port, isn, 0, SYN, WINDOW, &[]);
        net::send_ip(dst, 6, &seg[..l]);

        let mut their_seq = 0u32;
        let mut ok = false;
        for _ in 0..8_000_000u32 {
            if let Some((_, n)) = net::poll_ip(6, Some(dst), &mut rb) {
                if let Some(h) = parse(&rb[..n]) {
                    if h.dport == sport {
                        if h.flags & RST != 0 { return None; }
                        if h.flags & SYN != 0 && h.flags & ACK != 0 {
                            their_seq = h.seq;
                            ok = true;
                            break;
                        }
                    }
                }
            }
        }
        if !ok { return None; }

        let seq = isn.wrapping_add(1);
        let ack = their_seq.wrapping_add(1);
        let a = build(&mut seg, &dst, sport, port, seq, ack, ACK, WINDOW, &[]);
        net::send_ip(dst, 6, &seg[..a]);

        Some(TcpConn { dst, sport, dport: port, seq, ack, rx: Vec::new(), ooo: Vec::new(), peer_fin: false, closed: false })
    }

    /// Envoie des donnees (segmente si necessaire).
    pub fn send(&mut self, data: &[u8]) -> bool {
        if self.closed { return false; }
        let mut seg = [0u8; 1600];
        let mss = 1400usize;
        let mut off = 0;
        while off < data.len() {
            let end = (off + mss).min(data.len());
            let chunk = &data[off..end];
            let l = build(&mut seg, &self.dst, self.sport, self.dport, self.seq, self.ack, PSH | ACK, WINDOW, chunk);
            net::send_ip(self.dst, 6, &seg[..l]);
            self.seq = self.seq.wrapping_add(chunk.len() as u32);
            off = end;
            // Petite fenetre de drainage des ACK/segments entrants.
            self.pump(200_000);
        }
        true
    }

    // Traite les segments entrants disponibles pendant `budget` iterations.
    fn pump(&mut self, budget: u32) {
        let mut rb = [0u8; 2048];
        let mut seg = [0u8; 64];
        for _ in 0..budget {
            if let Some((_, n)) = net::poll_ip(6, Some(self.dst), &mut rb) {
                if let Some(h) = parse(&rb[..n]) {
                    if h.dport != self.sport { continue; }
                    if h.flags & RST != 0 { self.closed = true; self.peer_fin = true; return; }
                    let plen = n - h.data_off;
                    if plen > 0 {
                        self.accept_segment(h.seq, &rb[h.data_off..n]);
                        // ACK cumulatif. Si le segment etait hors sequence, on re-ACK
                        // volontairement le prochain octet attendu.
                        let a = build(&mut seg, &self.dst, self.sport, self.dport, self.seq, self.ack, ACK, WINDOW, &[]);
                        net::send_ip(self.dst, 6, &seg[..a]);
                    }
                    if h.flags & FIN != 0 && h.seq.wrapping_add(plen as u32) == self.ack {
                        self.ack = self.ack.wrapping_add(1);
                        self.peer_fin = true;
                        let a = build(&mut seg, &self.dst, self.sport, self.dport, self.seq, self.ack, ACK, WINDOW, &[]);
                        net::send_ip(self.dst, 6, &seg[..a]);
                    }
                }
            }
        }
    }

    fn accept_segment(&mut self, seq: u32, data: &[u8]) {
        if data.is_empty() { return; }

        if seq == self.ack {
            self.rx.extend_from_slice(data);
            self.ack = self.ack.wrapping_add(data.len() as u32);
            self.flush_ooo();
            return;
        }

        // Segment deja recu/retransmis. Si c'est un chevauchement, on garde
        // uniquement la partie nouvelle.
        if seq_less(seq, self.ack) {
            let skip = self.ack.wrapping_sub(seq) as usize;
            if skip >= data.len() { return; }
            self.rx.extend_from_slice(&data[skip..]);
            self.ack = self.ack.wrapping_add((data.len() - skip) as u32);
            self.flush_ooo();
            return;
        }

        // Petit tampon de reordonnancement : suffisant pour les bursts TLS de
        // Google/QEMU sans transformer ce client en pile TCP complete.
        if self.ooo.len() >= 16 { return; }
        for p in &self.ooo {
            if p.seq == seq { return; }
        }
        self.ooo.push(PendingSeg { seq, data: data.to_vec() });
    }

    fn flush_ooo(&mut self) {
        loop {
            let mut found = None;
            let mut i = 0usize;
            while i < self.ooo.len() {
                if self.ooo[i].seq == self.ack {
                    found = Some(i);
                    break;
                }
                i += 1;
            }
            let idx = match found { Some(i) => i, None => break };
            let p = self.ooo.remove(idx);
            self.rx.extend_from_slice(&p.data);
            self.ack = self.ack.wrapping_add(p.data.len() as u32);
        }
    }

    /// Attend qu'au moins `want` octets soient disponibles (ou FIN/timeout).
    /// Renvoie true si `rx.len() >= want`.
    ///
    /// Patient : tolere un aller-retour reseau complet (la reponse applicative
    /// arrive apres un RTT, contrairement au flight de handshake qui suit
    /// immediatement le ServerHello).
    pub fn fill(&mut self, want: usize) -> bool {
        let mut idle = 0u32;
        while self.rx.len() < want && !self.peer_fin && !self.closed {
            let before = self.rx.len();
            self.pump(1_000_000);
            if self.rx.len() == before {
                idle += 1;
                if idle >= 60 { break; }
            } else {
                idle = 0;
            }
        }
        self.rx.len() >= want
    }

    /// Consomme `n` octets en tete du tampon de reception.
    pub fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.rx.len());
        let out = self.rx[..n].to_vec();
        self.rx.drain(..n);
        out
    }

    /// Ferme la connexion (FIN).
    pub fn close(&mut self) {
        if self.closed { return; }
        let mut seg = [0u8; 64];
        let f = build(&mut seg, &self.dst, self.sport, self.dport, self.seq, self.ack, FIN | ACK, WINDOW, &[]);
        net::send_ip(self.dst, 6, &seg[..f]);
        self.seq = self.seq.wrapping_add(1);
        self.closed = true;
    }
}
