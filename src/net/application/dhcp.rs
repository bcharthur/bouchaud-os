//! Client DHCP (DORA : Discover/Offer/Request/Ack) sur UDP.
//!
//! Obtient automatiquement l'adresse IP, la passerelle et le DNS aupres du
//! serveur DHCP du reseau (SLIRP sous QEMU), puis applique la configuration.

use crate::arch::x86_64::cpu;
use crate::drivers::e1000;
use crate::net::ipv4::Ipv4Addr;
use crate::net::{self, ethernet, ipv4, udp};

pub mod options;
use options::{build_msg, parse_reply, Lease, LONGUEUR_DOMAINE};

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
    recv_avant(xid, want_type, ATTENTE_COMMANDE_MS)
}

/// Delai laisse a un serveur DHCP par la commande `dhcp`, tapee par quelqu'un
/// qui attend une reponse et preferera patienter plutot que d'echouer trop tot.
pub const ATTENTE_COMMANDE_MS: u64 = 4_000;

/// Delai laisse par l'initialisation du demarrage.
///
/// Court, et volontairement. Un serveur DHCP repond en quelques millisecondes ;
/// un serveur qui met plusieurs secondes est un serveur qui n'est pas la. La
/// premiere version attendait deux fois huit millions de tours de boucle sans
/// horloge — environ quinze secondes — et les payait **a chaque demarrage** sur
/// un reseau qui n'a pas de serveur DHCP. Sous QEMU/SLIRP, ou l'adressage est
/// fixe et connu d'avance, cela retardait la banniere de quinze secondes pour
/// ne rien apprendre.
pub const ATTENTE_DEMARRAGE_MS: u64 = 700;

/// Attend une reponse, au plus `budget_ms` millisecondes.
///
/// Le temps se lit sur l'horloge du noyau et non sur un compteur de tours : un
/// nombre de tours ne mesure rien, puisqu'il vaut quinze secondes sous
/// emulation et une fraction de seconde sur une vraie machine. C'est la meme
/// boucle qui rendait le demarrage lent ici et rapide ailleurs, sans que le
/// code ne dise laquelle des deux etait voulue.
fn recv_avant(xid: u32, want_type: u8, budget_ms: u64) -> Option<Lease> {
    let mut buf = [0u8; 2048];
    let debut = crate::kernel::timer::monotonic_ms();
    let limite = debut.saturating_add(budget_ms);
    let mut fallback_spins = 0usize;

    while crate::kernel::timer::monotonic_ms() < limite
        && fallback_spins < 10_000_000
    {
        fallback_spins = fallback_spins.saturating_add(1);
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

/// Ce qu'une negociation reussie a rapporte.
#[derive(Clone, Copy)]
pub struct Bail {
    pub ip: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns: Ipv4Addr,
}

/// Negocie un bail et applique la configuration. **Sans rien afficher.**
///
/// Separe de `run` parce que les deux appelants n'ont pas le meme besoin : la
/// commande `dhcp` raconte chaque etape a l'utilisateur qui l'a tapee, tandis
/// que l'initialisation du demarrage ne doit produire qu'une ligne de journal.
/// Tant qu'il n'y avait qu'un appelant, la question ne se posait pas ; le
/// demarrage automatique en a fait un second.
pub fn negocie() -> Option<Bail> {
    negocie_avant(ATTENTE_DEMARRAGE_MS)
}

/// `negocie`, avec un budget d'attente explicite par etape.
pub fn negocie_avant(budget_ms: u64) -> Option<Bail> {
    if !e1000::is_ready() && !e1000::init() {
        return None;
    }
    let mac = e1000::mac();
    let xid = cpu::rdtsc() as u32;
    let mut msg = [0u8; 400];

    let l = build_msg(&mut msg, xid, mac, 1, None, None);
    send(mac, &msg[..l]);
    let offer = recv_avant(xid, 2, budget_ms)?;

    let l = build_msg(&mut msg, xid, mac, 3, Some(offer.your_ip), Some(offer.server_id));
    send(mac, &msg[..l]);
    let ack = recv_avant(xid, 5, budget_ms)?;

    // Valeurs de repli : un serveur qui n'annonce ni routeur ni resolveur
    // laisse la configuration compilee en place plutot que de poser 0.0.0.0,
    // qui donnerait une interface configuree et injoignable.
    let gateway = if ack.router == [0, 0, 0, 0] { net::gateway() } else { ack.router };
    let dns = if ack.dns == [0, 0, 0, 0] { net::dns_server() } else { ack.dns };
    net::set_config(ack.your_ip, gateway, dns);
    net::pose_identite_reseau(&ack.domaine[..ack.domaine_len], ack.masque);
    Some(Bail { ip: ack.your_ip, gateway, dns })
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
    net::pose_identite_reseau(&ack.domaine[..ack.domaine_len], ack.masque);

    crate::print!("DHCP: bail obtenu  inet "); ipv4::print_addr(&ack.your_ip);
    crate::print!("  gw "); ipv4::print_addr(&gw);
    crate::print!("  dns "); ipv4::print_addr(&dns);
    crate::println!("");
}
