//! Pont entre l'ingress reseau unique et `smoltcp::phy::Device`.
//!
//! # BOUCHAUD_NET_INGRESS_UNIQUE_V1 : ce module ne lit plus la carte
//!
//! La version precedente appelait `e1000::receive` directement. Son propre
//! commentaire de tete posait la regle -- « les deux ne doivent JAMAIS tourner
//! en meme temps (le NIC n'a qu'une file RX/TX) » -- et rien ne la faisait
//! respecter : le veilleur de lien, la resolution ARP, le client DHCP et
//! `pump_udp` drainent l'anneau pendant qu'une requete `smol_tcp::fetch`
//! tourne.
//!
//! Le probleme n'est pas une course memoire. C'est la PROPRIETE DES PAQUETS :
//! une trame retiree de la carte par l'un n'existe plus pour l'autre, et une
//! reponse ARP ne se retransmet pas.
//!
//! Desormais une seule fonction lit physiquement l'anneau --
//! `net::draine_verrouille` -- et repartit. Ce peripherique consomme une file
//! logicielle, comme n'importe quel autre consommateur :
//!
//! ```text
//!   NIC RX --> ingress unique --> ARP maison
//!                             --> DHCP
//!                             --> files IPv4 maison
//!                             --> file smoltcp  <-- ici
//! ```
//!
//! L'emission, elle, reste directe : le sens TX n'a pas de probleme de
//! propriete -- une trame emise n'est retiree a personne.

use crate::drivers::e1000;
use smoltcp::phy::{Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;

// MTU Ethernet standard (1500) + marge pour l'en-tete/FCS deja geree par le driver.
const FRAME_MAX: usize = 1600;

pub struct E1000Device;

pub struct E1000RxToken {
    buf: [u8; FRAME_MAX],
    len: usize,
}

pub struct E1000TxToken;

impl smoltcp::phy::RxToken for E1000RxToken {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.buf[..self.len])
    }
}

impl smoltcp::phy::TxToken for E1000TxToken {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut buf = [0u8; FRAME_MAX];
        let r = f(&mut buf[..len.min(FRAME_MAX)]);
        e1000::send(&buf[..len.min(FRAME_MAX)]);
        r
    }
}

impl Device for E1000Device {
    type RxToken<'a> = E1000RxToken;
    type TxToken<'a> = E1000TxToken;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut buf = [0u8; FRAME_MAX];
        // LA FILE, PAS LA CARTE. Voir l'en-tete de ce module.
        match crate::net::retire_trame_smoltcp(&mut buf) {
            Some(n) => Some((E1000RxToken { buf, len: n }, E1000TxToken)),
            None => None,
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(E1000TxToken)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1500;
        caps.medium = Medium::Ethernet;
        caps
    }
}
