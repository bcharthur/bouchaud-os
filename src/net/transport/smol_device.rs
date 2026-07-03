//! Pont entre le driver e1000 (trames brutes) et `smoltcp::phy::Device`.
//!
//! Experimental : ce module permet a une pile `smoltcp` de piloter la meme
//! carte reseau que notre pile maison (link/internet/transport), sans la
//! remplacer. Les deux ne doivent JAMAIS tourner en meme temps (le NIC n'a
//! qu'une file RX/TX) : un appelant qui construit une `smoltcp::iface::Interface`
//! avec `E1000Device` doit le faire pour une operation ponctuelle (ex. une
//! requete TCP), pas la garder ouverte en permanence.

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
        match e1000::receive(&mut buf) {
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
