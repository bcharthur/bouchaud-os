//! Fetch TCP via `smoltcp` (experimental) : meme signature que `tcp::fetch`,
//! mais adosse a une vraie pile TCP (RFC 793 correcte : RTT/retransmission/
//! fenetre glissante geres par smoltcp) plutot qu'a notre implementation
//! maison (SYN/ACK par iteration, timeout d'inactivite fixe -- cf. tcp.rs).
//!
//! Construit une `smoltcp::iface::Interface` ephemere pour l'operation (voir
//! smol_device.rs : ne doit jamais tourner en concurrence avec le reste de la
//! pile reseau, qui partage le meme e1000).
//!
//! Etat : verifie a la COMPILATION pour cette cible no_std (cargo check),
//! PAS ENCORE verifie a l'EXECUTION (pas d'acces QEMU dans cet environnement
//! de developpement). Pas encore cable comme chemin par defaut -- voir
//! net::mod::fetch_document, qui continue d'utiliser tcp.rs/TcpConn. A tester
//! au prochain boot avant toute bascule.

use alloc::vec;
use alloc::vec::Vec;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address};

use super::smol_device::E1000Device;
use crate::net::ipv4::Ipv4Addr;

fn now_ms() -> i64 {
    crate::kernel::timer::cycles_to_ms(crate::kernel::timer::cycles_since_boot()) as i64
}

/// Ouvre une connexion TCP vers `dst:port`, envoie `request`, accumule la
/// reponse dans `out`. Timeout global de 15s. Renvoie `true` si au moins un
/// octet de reponse a ete recu.
pub fn fetch(dst: Ipv4Addr, port: u16, request: &[u8], out: &mut Vec<u8>) -> bool {
    let mut device = E1000Device;
    let mac = crate::drivers::e1000::mac();
    let hw = HardwareAddress::Ethernet(EthernetAddress(mac));
    let mut config = Config::new(hw);
    config.random_seed = crate::arch::x86_64::cpu::rdtsc();

    let t0 = Instant::from_millis(now_ms());
    let mut iface = Interface::new(config, &mut device, t0);

    let our = crate::net::our_ip();
    iface.update_ip_addrs(|ips| {
        let _ = ips.push(IpCidr::new(IpAddress::v4(our[0], our[1], our[2], our[3]), 24));
    });
    let gw = crate::net::gateway();
    let _ = iface.routes_mut().add_default_ipv4_route(Ipv4Address::new(gw[0], gw[1], gw[2], gw[3]));

    let rx_buf = tcp::SocketBuffer::new(vec![0u8; 16384]);
    let tx_buf = tcp::SocketBuffer::new(vec![0u8; 4096]);
    let mut socket = tcp::Socket::new(rx_buf, tx_buf);

    let local_port = 0xC000u16 | (crate::arch::x86_64::cpu::rdtsc() as u16 & 0x0FFF);
    let remote = IpEndpoint::new(IpAddress::v4(dst[0], dst[1], dst[2], dst[3]), port);
    if socket.connect(iface.context(), remote, local_port).is_err() {
        return false;
    }

    let mut sockets = SocketSet::new(Vec::new());
    let handle = sockets.add(socket);

    let mut sent = false;
    // Deadline GLOBALE (le service de rendu deporte peut mettre quelques
    // secondes a produire la premiere image) et deadline d'INACTIVITE (repoussee
    // a chaque octet recu) pour ne pas rester bloque si le pair se tait.
    let deadline = now_ms() + 30_000;
    let mut last_rx = now_ms();
    loop {
        let t = Instant::from_millis(now_ms());
        iface.poll(t, &mut device, &mut sockets);
        let sock = sockets.get_mut::<tcp::Socket>(handle);

        if !sent && sock.can_send() {
            let _ = sock.send_slice(request);
            sent = true;
        }
        let mut got = false;
        while sock.can_recv() {
            let mut buf = [0u8; 2048];
            match sock.recv_slice(&mut buf) {
                Ok(n) if n > 0 => { out.extend_from_slice(&buf[..n]); got = true; }
                _ => break,
            }
        }
        if got { last_rx = now_ms(); }
        if sent && !sock.may_recv() && !sock.can_recv() { break; }
        if !sock.is_active() && sent { break; }
        if !sock.is_active() && !sent { return false; }
        let nowm = now_ms();
        if nowm > deadline { break; }
        if nowm.wrapping_sub(last_rx) > 12_000 { break; } // 12 s sans le moindre octet
    }
    !out.is_empty()
}
