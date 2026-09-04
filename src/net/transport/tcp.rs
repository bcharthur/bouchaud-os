//! Client TCP minimal (suffisant pour un GET HTTP).
//!
//! Implemente la poignee de main (SYN/SYN-ACK/ACK), l'envoi d'une requete et la
//! reception/ack des segments jusqu'au FIN.
//!
//! # Chantier 9 : ce qui vient d'apparaitre
//!
//! Cet en-tete disait « Pas de retransmission ni de controle de congestion ».
//! La premiere moitie n'est plus vraie : `transport::retransmission` tient la
//! file des segments ENVOYES ET NON ACQUITTES, mesure le RTT (algorithme de
//! Karn), calcule le RTO (RFC 6298) et le double a chaque expiration. Le SYN et
//! la requete y sont enregistres, et sont RENVOYES quand ils expirent -- avant,
//! un `GET` perdu faisait attendre les trente secondes du delai d'inactivite.
//!
//! La seconde moitie reste vraie : il n'y a toujours pas de controle de
//! congestion. `cwnd` demande de decider quand REDUIRE le debit, et cette
//! decision se prend sur des mesures qu'on n'a pas encore ; la file d'emission
//! est ce qui la rendra possible.
//!
//! # Le busy-poll
//!
//! L'attente etait un `pause` en boucle pendant huit secondes, choisi parce
//! qu'on ne savait pas combien de temps attendre. Le RTO le dit maintenant :
//! l'attente active est bornee par lui, ce qui la ramene a l'ordre du RTT reel
//! au lieu d'une constante. Elle est aussi COMPTEE -- `[NET-TCP] busy_poll_tours=`
//! --, sans quoi « le busy-poll a disparu » resterait une affirmation.

use crate::arch::x86_64::cpu;
use crate::net::transport::retransmission::{self, Emission, Expiration};
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

// Fenetre de reception annoncee, au maximum. Combinee a l'option MSS 1460
// (segments plus gros -> ~31 segments pour un PNG de 44 Ko, ce qui tient dans
// l'anneau RX de 64 descripteurs) et au reassemblage HORS-ORDRE ci-dessous, le
// pair (SLIRP) peut envoyer tout le corps en une rafale qu'on draine en une
// passe. Une fenetre PLUS PETITE etait contre-productive : elle forcait SLIRP
// a envoyer par petites tranches espacees de plusieurs secondes (cwnd bloquee),
// faisant durer un transfert de 44 Ko plus de 2 minutes.
const WINDOW: u16 = 65535;

/// Construit un segment SYN avec l'option MSS (1460) : sans elle le pair
/// suppose un MSS de 536 o et fragmente en ~2,7x plus de segments, ce qui
/// aggrave le risque de debordement de l'anneau RX. Renvoie la longueur.
fn build_syn(buf: &mut [u8], dst: &Ipv4Addr, sport: u16, dport: u16, isn: u32) -> usize {
    // En-tete de 24 octets (data offset = 6 mots) : 20 fixes + option MSS 4 o.
    let total = 24;
    buf[0] = (sport >> 8) as u8; buf[1] = sport as u8;
    buf[2] = (dport >> 8) as u8; buf[3] = dport as u8;
    buf[4..8].copy_from_slice(&isn.to_be_bytes());
    buf[8..12].copy_from_slice(&0u32.to_be_bytes());
    buf[12] = 0x60; // data offset = 6 mots (24 octets)
    buf[13] = SYN;
    buf[14] = (WINDOW >> 8) as u8; buf[15] = WINDOW as u8;
    buf[16] = 0; buf[17] = 0; // checksum
    buf[18] = 0; buf[19] = 0; // urgent
    // Option MSS : kind=2, len=4, valeur=1460.
    buf[20] = 2; buf[21] = 4;
    buf[22] = (1460u16 >> 8) as u8; buf[23] = 1460u16 as u8;
    let c = checksum(&net::our_ip(), dst, &buf[..total]);
    buf[16] = (c >> 8) as u8; buf[17] = c as u8;
    total
}

struct PendingSeg {
    seq: u32,
    data: Vec<u8>,
}

fn seq_less(a: u32, b: u32) -> bool {
    (a as i32).wrapping_sub(b as i32) < 0
}

/// Ouvre une connexion, envoie `request`, accumule la reponse dans `out`.
/// Renvoie true si la poignee de main a reussi.
/// Recolle les segments recus en avance devenus contigus avec `ack` : tant
/// qu'un segment memorise commence exactement a `ack`, on l'ajoute au flux et
/// on avance. Les doublons/segments deja couverts sont ecartes.
fn drain_ooo(ooo: &mut Vec<(u32, Vec<u8>)>, ack: &mut u32, out: &mut Vec<u8>) {
    loop {
        let mut best: Option<usize> = None;
        for (i, (s, _)) in ooo.iter().enumerate() {
            if *s == *ack {
                best = Some(i);
                break;
            }
        }
        match best {
            Some(i) => {
                let (_, data) = ooo.remove(i);
                *ack = ack.wrapping_add(data.len() as u32);
                out.extend_from_slice(&data);
            }
            None => {
                // Ecarte les segments deja entierement couverts (seq < ack).
                ooo.retain(|(s, d)| {
                    let rel = s.wrapping_sub(*ack) as i32;
                    rel >= 0 || (rel + d.len() as i32) > 0
                });
                break;
            }
        }
    }
}

/// Millisecondes ecoulees, telles que la retransmission les compte.
fn maintenant_ms() -> u64 {
    use crate::kernel::timer;
    let par_seconde = timer::TICKS_PER_SECOND.max(1) as u64;
    (timer::ticks() as u64).saturating_mul(1000) / par_seconde
}

pub fn fetch(dst: Ipv4Addr, port: u16, request: &[u8], out: &mut Vec<u8>) -> bool {
    let sport = 0xC000u16 | (cpu::rdtsc() as u16 & 0x0FFF);
    let isn = cpu::rdtsc() as u32;
    let mut seg = [0u8; 1600];
    let mut rb = [0u8; 2048];

    // La file des segments non acquittes. Le SYN et la requete y entrent ; sans
    // elle, un `GET` perdu n'etait jamais renvoye et la connexion attendait ses
    // trente secondes d'inactivite avant d'abandonner.
    let mut emission = Emission::neuve(isn);
    let mut busy_poll_tours = 0u64;
    let mut attentes_dormies = 0u64;

    // --- SYN (avec option MSS) ---
    let l = build_syn(&mut seg, &dst, sport, port, isn);
    net::send_ip(dst, 6, &seg[..l]);
    // Le SYN consomme un numero de sequence sans porter d'octet : c'est
    // exactement ce que `controle` decrit, et c'est ce qui permet a l'ACK du
    // SYN-ACK de le retirer de la file et d'en tirer le PREMIER echantillon de
    // RTT -- celui qui remplace le RTO initial d'une seconde.
    let syn_index = emission.enregistre(isn, &[], true, maintenant_ms());

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
                        emission.acquitte(h.ack, maintenant_ms());
                        established = true;
                        break;
                    }
                }
            }
        }
        // Le SYN peut se perdre. Sans retransmission, la boucle tournait ses
        // huit millions de tours puis rendait `false` : une connexion refusee
        // pour un paquet perdu.
        if let Expiration::Retransmettre(index) = emission.expire(maintenant_ms()) {
            if Some(index) == syn_index {
                let l = build_syn(&mut seg, &dst, sport, port, isn);
                net::send_ip(dst, 6, &seg[..l]);
                emission.note_retransmission(index, maintenant_ms());
            }
        }
    }
    if !established {
        retransmission::note_connexion(&emission, busy_poll_tours, attentes_dormies);
        return false;
    }

    let my_seq = isn.wrapping_add(1);
    let mut my_ack = their_seq.wrapping_add(1);

    // --- ACK de la poignee + envoi de la requete (PSH|ACK) ---
    let l = build(&mut seg, &dst, sport, port, my_seq, my_ack, ACK, WINDOW, &[]);
    net::send_ip(dst, 6, &seg[..l]);
    let l = build(&mut seg, &dst, sport, port, my_seq, my_ack, PSH | ACK, WINDOW, request);
    net::send_ip(dst, 6, &seg[..l]);
    // La requete tient dans un segment (un `GET` fait quelques centaines
    // d'octets). Elle entre dans la file : c'est elle qui, perdue, coutait le
    // plus cher.
    let requete_index = if request.len() <= retransmission::CHARGE_MAX {
        emission.enregistre(my_seq, request, false, maintenant_ms())
    } else {
        None
    };
    let mut my_seq = my_seq.wrapping_add(request.len() as u32);

    // --- reception ---
    // Reassemblage HORS-ORDRE : les segments qui arrivent en avance (gap) sont
    // MEMORISES (`ooo`) au lieu d'etre jetes. Sans ca, un seul segment reordonne
    // faisait re-ACKer l'ancienne position et attendre le RTO de retransmission
    // du pair (backoff exponentiel) -> un PNG de 44 Ko mettait ~250 s. Avec le
    // buffer, une rafale reordonnee est recollee sans retransmission.
    //
    // Econome en CPU : on draine tout ce qui est dispo sans dormir ; quand il
    // n'arrive plus rien on met le CPU en veille (`hlt`) jusqu'a la prochaine
    // interruption. Timeout d'INACTIVITE repousse a chaque octet recu (un gros
    // transfert n'est jamais coupe).
    use crate::kernel::timer;
    let deadline = 30 * timer::TICKS_PER_SECOND.max(1);
    let mut last = timer::ticks();
    // Segments recus en avance, tries par numero de sequence.
    let mut ooo: Vec<(u32, Vec<u8>)> = Vec::new();
    loop {
        let mut got = false;
        let mut advanced = false;
        // Draine tout ce qui est dispo SANS dormir tant qu'il arrive des paquets.
        for _ in 0..8192u32 {
            match net::poll_ip(6, Some(dst), &mut rb) {
                Some((_, n)) => {
                    if let Some(h) = parse(&rb[..n]) {
                        if h.dport != sport { continue; }
                        got = true;
                        // Tout ACK nourrit la file d'emission : c'est de la
                        // qu'on tire le RTT, et c'est la que les doublons se
                        // comptent.
                        if h.flags & ACK != 0 {
                            emission.acquitte(h.ack, maintenant_ms());
                            if let Some(index) = emission.retransmission_rapide() {
                                if let Some(perdu) = emission.segment(index) {
                                    let (pseq, pdata, pctrl) =
                                        (perdu.seq, perdu.charge().to_vec(), perdu.controle);
                                    let l = if pctrl {
                                        build_syn(&mut seg, &dst, sport, port, pseq)
                                    } else {
                                        build(&mut seg, &dst, sport, port, pseq, my_ack,
                                              PSH | ACK, WINDOW, &pdata)
                                    };
                                    net::send_ip(dst, 6, &seg[..l]);
                                    emission.note_retransmission(index, maintenant_ms());
                                }
                            }
                        }
                        let plen = n - h.data_off;
                        if plen > 0 {
                            let rel = h.seq.wrapping_sub(my_ack) as i32;
                            if h.seq == my_ack {
                                // Segment attendu : on l'ajoute puis on recolle
                                // les segments en avance devenus contigus.
                                out.extend_from_slice(&rb[h.data_off..n]);
                                my_ack = my_ack.wrapping_add(plen as u32);
                                advanced = true;
                                drain_ooo(&mut ooo, &mut my_ack, out);
                            } else if rel > 0 && (rel as usize) < 2 * 1024 * 1024 {
                                // Segment en avance (gap) : on le memorise au lieu
                                // de le jeter. Dedup par seq, buffer borne.
                                if !ooo.iter().any(|(s, _)| *s == h.seq) && ooo.len() < 512 {
                                    ooo.push((h.seq, rb[h.data_off..n].to_vec()));
                                }
                            }
                            // ACK cumulatif de tout ce qui est en ordre.
                            let a = build(&mut seg, &dst, sport, port, my_seq, my_ack, ACK, WINDOW, &[]);
                            net::send_ip(dst, 6, &seg[..a]);
                        }
                        if h.flags & FIN != 0 && ooo.is_empty() {
                            my_ack = my_ack.wrapping_add(1);
                            let a = build(&mut seg, &dst, sport, port, my_seq, my_ack, ACK, WINDOW, &[]);
                            net::send_ip(dst, 6, &seg[..a]);
                            let f = build(&mut seg, &dst, sport, port, my_seq, my_ack, FIN | ACK, WINDOW, &[]);
                            net::send_ip(dst, 6, &seg[..f]);
                            retransmission::note_connexion(
                                &emission, busy_poll_tours, attentes_dormies);
                            return true;
                        }
                        if h.flags & RST != 0 {
                            retransmission::note_connexion(
                                &emission, busy_poll_tours, attentes_dormies);
                            return true;
                        }
                    }
                }
                None => break, // plus rien a lire pour l'instant
            }
        }
        // Plafond du corps : assez large pour une image PNG de page rendue.
        if out.len() > 4_000_000 { break; }
        if got {
            last = timer::ticks();
            let _ = advanced;
        } else {
            // Rien a lire pour l'instant. On NE DORT PAS via `hlt` tant que le
            // transfert est jeune : `enable_and_hlt` ne rend la main qu'a la
            // prochaine interruption timer (~55 ms), ce qui RETARDE nos ACK
            // d'autant -> le pair (SLIRP) mesure un RTT enorme, garde une fenetre
            // de congestion de 1 segment et envoie ~1 segment/3,7 s (44 Ko en
            // ~130 s). En occupant le CPU (`pause`) on ACKe instantanement : la
            // fenetre grossit et le transfert passe a < 1 s. Apres une periode
            // d'inactivite reelle (le proxy reflechit, page vide...), on repasse
            // en veille pour ne pas chauffer un coeur inutilement.
            let idle = timer::ticks().wrapping_sub(last);
            if idle > deadline { break; }

            // Un segment expire se renvoie, meme au milieu de la reception :
            // c'est le cas d'une requete perdue apres que le pair a commence a
            // repondre a une precedente.
            let ms = maintenant_ms();
            match emission.expire(ms) {
                Expiration::Retransmettre(index) => {
                    if let Some(perdu) = emission.segment(index) {
                        let (pseq, pdata, pctrl) =
                            (perdu.seq, perdu.charge().to_vec(), perdu.controle);
                        let l = if pctrl {
                            build_syn(&mut seg, &dst, sport, port, pseq)
                        } else {
                            build(&mut seg, &dst, sport, port, pseq, my_ack,
                                  PSH | ACK, WINDOW, &pdata)
                        };
                        net::send_ip(dst, 6, &seg[..l]);
                        emission.note_retransmission(index, ms);
                    }
                }
                Expiration::Abandon(_) => break,
                Expiration::Rien => {}
            }

            // BOUCHAUD_C9_BUSY_POLL_BORNE_PAR_LE_RTO_V1
            //
            // L'attente active durait huit secondes, en dur, parce qu'on ne
            // savait pas combien de temps attendre. Le RTO le dit maintenant.
            //
            // Elle n'est pas supprimee, et c'est deliberе : le pair (SLIRP)
            // fait grossir sa fenetre de congestion sur le RTT qu'il mesure sur
            // NOS ACK. Dormir jusqu'au prochain tick timer (~55 ms) les retarde
            // d'autant, il garde cwnd=1 et espace ses envois de plusieurs
            // secondes -- un transfert de 44 Ko passait alors a plus de deux
            // minutes. Ce qui change, c'est qu'elle est BORNEE PAR UNE MESURE et
            // COMPTEE : `[NET-TCP] busy_poll_tours=` face a `sommeils=` dit ce qui
            // reste a gagner, la ou une constante ne disait rien.
            let fenetre_active_ticks = {
                let rto_ticks = emission.rto_ms().saturating_mul(
                    timer::TICKS_PER_SECOND.max(1) as u64) / 1000;
                rto_ticks.max(1).min(8 * timer::TICKS_PER_SECOND.max(1) as u64)
            };
            if (idle as u64) < fenetre_active_ticks {
                busy_poll_tours += 1;
                for _ in 0..1000 {
                    unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
                }
            } else {
                attentes_dormies += 1;
                x86_64::instructions::interrupts::enable_and_hlt();
            }
        }
    }

    // Fermeture propre.
    let f = build(&mut seg, &dst, sport, port, my_seq, my_ack, FIN | ACK, WINDOW, &[]);
    net::send_ip(dst, 6, &seg[..f]);
    let _ = &mut my_seq;
    let _ = requete_index;
    retransmission::note_connexion(&emission, busy_poll_tours, attentes_dormies);
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
    pub rst_seen: bool,
    pub fin_seen: bool,
}

impl TcpConn {
    /// Ouvre une connexion (poignee SYN/SYN-ACK/ACK).
    pub fn connect(dst: Ipv4Addr, port: u16) -> Option<TcpConn> {
        let sport = 0xC000u16 | (cpu::rdtsc() as u16 & 0x0FFF);
        let isn = cpu::rdtsc() as u32;
        let mut seg = [0u8; 64];
        let mut rb = [0u8; 2048];

        let mut their_seq = 0u32;
        let mut ok = false;
        let mut refuse = false;

        // Le SYN est retransmis, comme dans tout TCP. Le premier paquet vers un
        // hote encore inconnu part souvent alors que son adresse materielle
        // n'est pas resolue : il se perd, et n'envoyer qu'un seul SYN faisait
        // echouer toute premiere connexion. Le navigateur y echappait sans le
        // savoir, parce qu'il commencait toujours par une requete DNS — laquelle
        // resolvait l'ARP au passage.
        // BOUCHAUD_C13_RTT_DE_POIGNEE_V1
        //
        // L'intervalle SYN -> SYN-ACK est un aller-retour complet sur le
        // chemin reseau : c'est la definition du RTT, et c'est la seule mesure
        // que ce chemin -- celui qu'empruntent les sockets ring 3 du
        // navigateur -- puisse produire aujourd'hui.
        //
        // L'horloge est la MONOTONE, jamais le tick PIT. Sous TCG, IRQ0 est
        // retardee quand un vCPU sature le traducteur ; un RTT mesure sur le
        // tick sous-estimerait la latence exactement quand la machine peine.
        let mut envoi_ms = 0u64;
        let mut syn_retransmis = 0u64;

        for tentative in 0..5u32 {
            let l = build(&mut seg, &dst, sport, port, isn, 0, SYN, WINDOW, &[]);
            envoi_ms = crate::kernel::timer::monotonic_ms();
            if tentative != 0 { syn_retransmis += 1; }
            net::send_ip(dst, 6, &seg[..l]);

            // Attente croissante d'une tentative a l'autre, comme le veut la
            // retransmission exponentielle.
            let patience = 1_500_000u32.saturating_mul(tentative + 1);
            for _ in 0..patience {
                if let Some((_, n)) = net::poll_ip(6, Some(dst), &mut rb) {
                    if let Some(h) = parse(&rb[..n]) {
                        if h.dport == sport {
                            // Un RST est une vraie fin de non-recevoir : inutile
                            // d'insister, personne n'ecoute sur ce port.
                            if h.flags & RST != 0 { refuse = true; break; }
                            if h.flags & SYN != 0 && h.flags & ACK != 0 {
                                their_seq = h.seq;
                                ok = true;
                                break;
                            }
                        }
                    }
                }
            }
            if ok || refuse { break; }
        }

        // Algorithme de Karn : apres une retransmission, on ne sait pas AUQUEL
        // des SYN le SYN-ACK repond. L'echantillon est ecarte -- mais la perte,
        // elle, est comptee. C'est meme la mesure la plus parlante des deux :
        // un SYN retransmis est une perte constatee, pas une estimation.
        let echantillon = if ok && syn_retransmis == 0 {
            Some(crate::kernel::timer::monotonic_ms().saturating_sub(envoi_ms))
        } else {
            None
        };
        retransmission::note_poignee(echantillon, syn_retransmis);

        if !ok { return None; }

        let seq = isn.wrapping_add(1);
        let ack = their_seq.wrapping_add(1);
        let a = build(&mut seg, &dst, sport, port, seq, ack, ACK, WINDOW, &[]);
        net::send_ip(dst, 6, &seg[..a]);

        Some(TcpConn { dst, sport, dport: port, seq, ack, rx: Vec::new(), ooo: Vec::new(), peer_fin: false, closed: false, rst_seen: false, fin_seen: false })
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
            net::send_ip_immediat(self.dst, 6, &seg[..l]);
            self.seq = self.seq.wrapping_add(chunk.len() as u32);
            off = end;
            // Petite fenetre de drainage des ACK/segments entrants.
            self.pump(200_000);
        }
        true
    }

    /// Envoie des donnees sans drainer les paquets entrants juste apres.
    /// Utile pour le Finished TLS 1.3 : on veut enchainer le premier record
    /// applicatif sans laisser un gros trou temporel cote client.
    pub fn send_no_pump(&mut self, data: &[u8]) -> bool {
        if self.closed { return false; }
        let mut seg = [0u8; 1600];
        let mss = 1400usize;
        let mut off = 0;
        while off < data.len() {
            let end = (off + mss).min(data.len());
            let chunk = &data[off..end];
            let l = build(&mut seg, &self.dst, self.sport, self.dport, self.seq, self.ack, PSH | ACK, WINDOW, chunk);
            net::send_ip_immediat(self.dst, 6, &seg[..l]);
            self.seq = self.seq.wrapping_add(chunk.len() as u32);
            off = end;
        }
        true
    }

    /// Sonde debug : draine les segments entrants sans consommer `rx`.
    /// Utilise par la trace TLS pour observer l'etat TCP juste apres le Finished client.
    pub fn poll_debug(&mut self, budget: u32) {
        self.pump(budget);
    }

    /// Fait avancer la connexion : lit les segments disponibles, acquitte, et
    /// range les donnees dans `rx`.
    ///
    /// Publique parce que la couche socket en a besoin : c'est elle qui decide
    /// combien de temps un `recv` accepte d'attendre.
    ///
    /// `budget` borne le nombre de segments traites en un passage — une rafale
    /// ne doit pas retenir l'appelant dans le noyau indefiniment. Ce n'est pas
    /// une attente : `poll_ip` est **non bloquant**, et quand il rend `None`
    /// l'anneau est vide a cet instant. Le rappeler ne peut rien faire arriver.
    ///
    /// La boucle epuisait pourtant son budget entier dans ce cas. Avec un
    /// budget de 20 000 appele depuis `poll` a chaque tick — mille fois par
    /// seconde — cela faisait vingt millions de lectures d'un anneau vide par
    /// seconde et par connexion ouverte. C'est le meme defaut que M13 a corrige
    /// pour UDP, reste ici : une attente comptee en tours de boucle n'est pas
    /// une attente, c'est du processeur brule.
    pub fn pump(&mut self, budget: u32) {
        let mut rb = [0u8; 2048];
        let mut seg = [0u8; 64];
        // Segments du bon hote mais d'une autre connexion. Ils sont remis a
        // disposition **apres** la boucle, jamais pendant : les rendre tout de
        // suite les ferait reprendre au tour suivant par ce meme appel, qui les
        // redeposerait, indefiniment. Un differe borne evite ce va-et-vient.
        let mut differes: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();

        for _ in 0..budget {
            // Anneau vide : insister ne fait rien arriver.
            let Some((_src, n)) = net::poll_ip(6, Some(self.dst), &mut rb) else {
                break;
            };
            let Some(h) = parse(&rb[..n]) else { continue };
            if h.dport != self.sport {
                differes.push(rb[..n].to_vec());
                continue;
            }
            if h.flags & RST != 0 {
                self.closed = true;
                self.peer_fin = true;
                self.rst_seen = true;
                break;
            }
            let plen = n - h.data_off;
            if plen > 0 {
                self.accept_segment(h.seq, &rb[h.data_off..n]);
                // ACK cumulatif. Si le segment etait hors sequence, on re-ACK
                // volontairement le prochain octet attendu.
                let a = build(&mut seg, &self.dst, self.sport, self.dport, self.seq, self.ack, ACK, WINDOW, &[]);
                net::send_ip_immediat(self.dst, 6, &seg[..a]);
            }
            if h.flags & FIN != 0 && h.seq.wrapping_add(plen as u32) == self.ack {
                self.ack = self.ack.wrapping_add(1);
                self.peer_fin = true;
                self.fin_seen = true;
                let a = build(&mut seg, &self.dst, self.sport, self.dport, self.seq, self.ack, ACK, WINDOW, &[]);
                net::send_ip_immediat(self.dst, 6, &seg[..a]);
            }
        }

        for segment in differes {
            net::depose_en_attente(6, self.dst, &segment);
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

        // Tampon de reordonnancement. Il doit couvrir une fenetre entiere en
        // vol : a 65535 octets annonces et 1460 par segment, cela fait
        // quarante-cinq segments. Le plafond de seize d'origine suffisait aux
        // rafales d'un echange TLS, mais un transfert soutenu le depassait —
        // et tout ce qui deborde ici doit etre retransmis, ce qui suffit a
        // effondrer le debit.
        if self.ooo.len() >= 64 { return; }
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
        use crate::kernel::timer;
        // Timeout base sur l'horloge PIT (avance via IRQ0, independamment du
        // niveau d'optimisation) : abandonne apres ~3 s SANS progres. La date
        // limite est repoussee a chaque octet recu, donc les gros transferts
        // par morceaux ne sont jamais coupes.
        let deadline = 3 * timer::TICKS_PER_SECOND.max(1);
        let mut last = timer::ticks();
        while self.rx.len() < want && !self.peer_fin && !self.closed {
            let before = self.rx.len();
            self.pump(100_000);
            if self.rx.len() != before {
                last = timer::ticks();
            } else if timer::ticks().wrapping_sub(last) >= deadline {
                break;
            } else {
                // Trois secondes d'attente sur l'horloge, sans jamais rendre le
                // gros verrou, gelaient tous les autres CPU -- la meme panne de
                // vivacite que l'attente ARP. `attente_cedante` suspend la
                // profondeur COMPLETE du verrou externe avant de dormir un
                // tick, et ne restaure que celle-la au reveil.
                //
                // Sur : depuis que `sys_recvfrom` pompe lui-meme, `fill` n'est
                // plus appele que depuis la poignee de main TLS, qui possede sa
                // `TcpConn` en propre et ne tient aucun verrou tournant.
                net::attente_cedante();
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
        net::send_ip_immediat(self.dst, 6, &seg[..f]);
        self.seq = self.seq.wrapping_add(1);
        self.closed = true;
    }
}
