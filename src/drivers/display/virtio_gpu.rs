//! Transport virtio-PCI moderne, et un premier aller-retour avec virtio-gpu.
//!
//! BOUCHAUD_VIRTIO_PCI_V1 -- premier composant de P10 (docs/ladybird/
//! P10_GPU_AUDIT.md). Aucun peripherique virtio n'existait pour Bouchaud :
//! ni GPU, ni reseau, ni disque. Ce module pose ce dont tous dependent :
//!
//!   - les capacites vendeur virtio de l'espace de configuration PCI
//!     (COMMON, NOTIFY, ISR, DEVICE), chacune dans un BAR memoire ;
//!   - la negociation : reset, ACKNOWLEDGE, DRIVER, fonctionnalites
//!     (VIRTIO_F_VERSION_1 et RIEN d'autre), FEATURES_OK relu, DRIVER_OK ;
//!   - une file « split » (descripteurs, anneau disponible, anneau utilise)
//!     en memoire DMA, et sa sonnette ;
//!   - un aller-retour SCRUTE (sans interruption, borne en temps) :
//!     VIRTIO_GPU_CMD_GET_DISPLAY_INFO, qui rend la geometrie des ecrans.
//!
//! Ce n'est PAS un pilote d'affichage : le bureau reste sur BGA/GOP, et rien
//! n'est accelere. Ne s'execute que si QEMU expose `-device virtio-gpu-pci`
//! (1af4:1050) ; sans lui, une ligne et rien d'autre.

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use crate::arch::x86_64::pci::{self, Bar, PciDevice};
use crate::kernel::memory;

const VENDEUR_VIRTIO: u16 = 0x1AF4;
const PERIPHERIQUE_GPU: u16 = 0x1040 + 16;
const CAP_VENDEUR: u8 = 0x09;

const CFG_COMMUNE: u8 = 1;
const CFG_NOTIFICATION: u8 = 2;
const CFG_ISR: u8 = 3;
const CFG_PERIPHERIQUE: u8 = 4;

// Statut du peripherique.
const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const FEATURES_OK: u8 = 8;
const ECHEC: u8 = 128;

/// VIRTIO_F_VERSION_1 : bit 32 des fonctionnalites.
const VERSION_1: u64 = 1 << 32;

// Configuration commune (virtio 1.x, 4.1.4.3).
const DEVICE_FEATURE_SELECT: usize = 0x00;
const DEVICE_FEATURE: usize = 0x04;
const DRIVER_FEATURE_SELECT: usize = 0x08;
const DRIVER_FEATURE: usize = 0x0C;
const NUM_QUEUES: usize = 0x12;
const DEVICE_STATUS: usize = 0x14;
const QUEUE_SELECT: usize = 0x16;
const QUEUE_SIZE: usize = 0x18;
const QUEUE_ENABLE: usize = 0x1C;
const QUEUE_NOTIFY_OFF: usize = 0x1E;
const QUEUE_DESC: usize = 0x20;
const QUEUE_DRIVER: usize = 0x28;
const QUEUE_DEVICE: usize = 0x30;

// Descripteurs.
const DESC_SUIVANT: u16 = 1;
const DESC_ECRIT_PAR_LE_PERIPHERIQUE: u16 = 2;

// virtio-gpu.
const CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
const MAX_ECRANS: usize = 16;
const ENTETE: usize = 24;

const FILE_MAX: u16 = 64;
const PAGE: usize = 4096;

struct Region {
    base: usize,
}

impl Region {
    unsafe fn l8(&self, d: usize) -> u8 { read_volatile((self.base + d) as *const u8) }
    unsafe fn l16(&self, d: usize) -> u16 { read_volatile((self.base + d) as *const u16) }
    unsafe fn l32(&self, d: usize) -> u32 { read_volatile((self.base + d) as *const u32) }
    unsafe fn e8(&self, d: usize, v: u8) { write_volatile((self.base + d) as *mut u8, v) }
    unsafe fn e16(&self, d: usize, v: u16) { write_volatile((self.base + d) as *mut u16, v) }
    unsafe fn e32(&self, d: usize, v: u32) { write_volatile((self.base + d) as *mut u32, v) }
    unsafe fn e64(&self, d: usize, v: u64) {
        self.e32(d, v as u32);
        self.e32(d + 4, (v >> 32) as u32);
    }
}

struct Capacites {
    commune: Option<usize>,
    notification: Option<(usize, u32)>,
    isr: bool,
    peripherique: bool,
}

/// L'adresse virtuelle d'un emplacement (bar, decalage) d'une capacite.
fn adresse(dev: &PciDevice, bar: u8, decalage: u32) -> Option<usize> {
    let physique = match pci::bar_decode(dev, bar) {
        Bar::Memoire32 { adresse, .. } => u64::from(adresse),
        Bar::Memoire64 { adresse, .. } => adresse,
        _ => return None,
    };
    if physique == 0 {
        return None;
    }
    Some(memory::phys_to_virt(physique + u64::from(decalage)) as usize)
}

fn capacites(dev: &PciDevice) -> Capacites {
    let mut c = Capacites { commune: None, notification: None, isr: false, peripherique: false };
    let mut liste = [pci::Capacite { identifiant: 0, decalage: 0 }; 16];
    let n = pci::capacites_de(dev, &mut liste);
    for cap in &liste[..n] {
        if cap.identifiant != CAP_VENDEUR {
            continue;
        }
        let o = cap.decalage;
        let mot0 = pci::config_lit32(dev, o);
        let genre = (mot0 >> 24) as u8;
        let bar = (pci::config_lit32(dev, o.saturating_add(4)) & 0xFF) as u8;
        let decalage = pci::config_lit32(dev, o.saturating_add(8));
        match genre {
            CFG_COMMUNE if c.commune.is_none() => c.commune = adresse(dev, bar, decalage),
            CFG_NOTIFICATION if c.notification.is_none() => {
                let multiplicateur = pci::config_lit32(dev, o.saturating_add(16));
                c.notification = adresse(dev, bar, decalage).map(|a| (a, multiplicateur));
            }
            CFG_ISR => c.isr = true,
            CFG_PERIPHERIQUE => c.peripherique = true,
            _ => {}
        }
    }
    c
}

/// Sonde : un virtio-gpu moderne present ? Si oui, transport + GET_DISPLAY_INFO.
pub fn sonde() {
    let mut trouve: Option<PciDevice> = None;
    pci::parcours(&mut |d| {
        if d.vendor == VENDEUR_VIRTIO && d.device == PERIPHERIQUE_GPU {
            trouve = Some(*d);
            return false;
        }
        true
    });
    let Some(dev) = trouve else {
        crate::serial_println!("VIRTIO_GPU absent (aucun 1af4:1050)");
        return;
    };
    match initialise(&dev) {
        Ok(()) => {}
        Err(etape) => crate::serial_println!("VIRTIO_GPU_ECHEC etape={}", etape),
    }
}

fn initialise(dev: &PciDevice) -> Result<(), &'static str> {
    let c = capacites(dev);
    crate::serial_println!(
        "VIRTIO_PCI peripherique={:04x}:{:04x} bus={:02x}:{:02x}.{} commune={} notification={} isr={} config={}",
        dev.vendor, dev.device, dev.bus, dev.slot, dev.func,
        c.commune.is_some() as u8, c.notification.is_some() as u8, c.isr as u8, c.peripherique as u8,
    );
    let commune = Region { base: c.commune.ok_or("capacite-commune-absente")? };
    let (notification, multiplicateur) = c.notification.ok_or("capacite-notification-absente")?;
    pci::enable_bus_master(dev);

    unsafe {
        // Reset, puis attendre que le statut retombe a 0.
        commune.e8(DEVICE_STATUS, 0);
        if !crate::kernel::timer::attente_bornee(1000, || commune.l8(DEVICE_STATUS) == 0) {
            return Err("reset");
        }
        commune.e8(DEVICE_STATUS, ACKNOWLEDGE);
        commune.e8(DEVICE_STATUS, ACKNOWLEDGE | DRIVER);

        // Fonctionnalites : VERSION_1 seulement. Tout le reste est refuse,
        // y compris ce que le peripherique propose (VIRGL, EDID, BLOB...) :
        // un pilote n'accepte que ce qu'il sait servir.
        commune.e32(DEVICE_FEATURE_SELECT, 0);
        let bas = commune.l32(DEVICE_FEATURE) as u64;
        commune.e32(DEVICE_FEATURE_SELECT, 1);
        let haut = commune.l32(DEVICE_FEATURE) as u64;
        let proposees = bas | (haut << 32);
        if proposees & VERSION_1 == 0 {
            commune.e8(DEVICE_STATUS, ECHEC);
            return Err("version-1-non-proposee");
        }
        let retenues = VERSION_1;
        commune.e32(DRIVER_FEATURE_SELECT, 0);
        commune.e32(DRIVER_FEATURE, retenues as u32);
        commune.e32(DRIVER_FEATURE_SELECT, 1);
        commune.e32(DRIVER_FEATURE, (retenues >> 32) as u32);
        commune.e8(DEVICE_STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK);
        if commune.l8(DEVICE_STATUS) & FEATURES_OK == 0 {
            commune.e8(DEVICE_STATUS, ECHEC);
            return Err("features-ok-refuse");
        }

        // La file 0 : controlq.
        let files = commune.l16(NUM_QUEUES);
        commune.e16(QUEUE_SELECT, 0);
        let taille_max = commune.l16(QUEUE_SIZE);
        if taille_max == 0 {
            commune.e8(DEVICE_STATUS, ECHEC);
            return Err("controlq-absente");
        }
        let taille = taille_max.min(FILE_MAX);
        commune.e16(QUEUE_SIZE, taille);
        let (desc_p, desc_v) = memory::alloc_dma(PAGE).ok_or("dma-descripteurs")?;
        let (dispo_p, dispo_v) = memory::alloc_dma(PAGE).ok_or("dma-disponible")?;
        let (util_p, util_v) = memory::alloc_dma(PAGE).ok_or("dma-utilise")?;
        let (tamp_p, tamp_v) = memory::alloc_dma(PAGE).ok_or("dma-tampons")?;
        for v in [desc_v, dispo_v, util_v, tamp_v] {
            core::ptr::write_bytes(v, 0, PAGE);
        }
        commune.e64(QUEUE_DESC, desc_p);
        commune.e64(QUEUE_DRIVER, dispo_p);
        commune.e64(QUEUE_DEVICE, util_p);
        let decalage_sonnette = commune.l16(QUEUE_NOTIFY_OFF) as usize * multiplicateur as usize;
        commune.e16(QUEUE_ENABLE, 1);
        commune.e8(DEVICE_STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK);
        crate::serial_println!(
            "VIRTIO_GPU_INIT fonctionnalites_proposees={:#018x} retenues={:#018x} files={} controlq={}/{}",
            proposees, retenues, files, taille, taille_max
        );

        // GET_DISPLAY_INFO : requete (lue par le peripherique) a 0, reponse
        // (ecrite par lui) a 512, chainees.
        let requete = tamp_v;
        let reponse = tamp_v.add(512);
        write_volatile(requete as *mut u32, CMD_GET_DISPLAY_INFO);
        let longueur_reponse = ENTETE + MAX_ECRANS * 24;
        let desc = desc_v as *mut u8;
        let pose = |i: usize, adresse: u64, longueur: u32, drapeaux: u16, suivant: u16| {
            let d = desc.add(i * 16);
            write_volatile(d as *mut u64, adresse);
            write_volatile(d.add(8) as *mut u32, longueur);
            write_volatile(d.add(12) as *mut u16, drapeaux);
            write_volatile(d.add(14) as *mut u16, suivant);
        };
        pose(0, tamp_p, ENTETE as u32, DESC_SUIVANT, 1);
        pose(1, tamp_p + 512, longueur_reponse as u32, DESC_ECRIT_PAR_LE_PERIPHERIQUE, 0);
        // Anneau disponible : flags u16, idx u16, ring[taille] u16.
        let dispo = dispo_v as *mut u16;
        write_volatile(dispo.add(2), 0); // ring[0] = tete de chaine 0
        fence(Ordering::SeqCst);
        write_volatile(dispo.add(1), 1); // idx = 1
        fence(Ordering::SeqCst);
        let debut = crate::kernel::timer::monotonic_ns();
        write_volatile((notification + decalage_sonnette) as *mut u16, 0);
        // Anneau utilise : flags u16, idx u16, puis {id u32, len u32}.
        let util = util_v as *const u16;
        if !crate::kernel::timer::attente_bornee(1000, || read_volatile(util.add(1)) != 0) {
            return Err("display-info-sans-reponse");
        }
        fence(Ordering::SeqCst);
        let delai_us = (crate::kernel::timer::monotonic_ns() - debut) / 1000;
        let genre = read_volatile(reponse as *const u32);
        if genre != RESP_OK_DISPLAY_INFO {
            crate::serial_println!("VIRTIO_GPU_DISPLAY_INFO reponse={:#06x} attendue={:#06x}", genre, RESP_OK_DISPLAY_INFO);
            return Err("display-info-refuse");
        }
        let mut actifs = 0;
        let (mut largeur0, mut hauteur0) = (0u32, 0u32);
        for e in 0..MAX_ECRANS {
            let m = reponse.add(ENTETE + e * 24);
            let largeur = read_volatile(m.add(8) as *const u32);
            let hauteur = read_volatile(m.add(12) as *const u32);
            let actif = read_volatile(m.add(16) as *const u32);
            if actif != 0 {
                if actifs == 0 {
                    largeur0 = largeur;
                    hauteur0 = hauteur;
                }
                actifs += 1;
            }
        }
        crate::serial_println!(
            "VIRTIO_GPU_DISPLAY_INFO ecrans_actifs={} ecran0={}x{} delai_us={}",
            actifs, largeur0, hauteur0, delai_us
        );
        if actifs == 0 {
            return Err("aucun-ecran-actif");
        }
        crate::serial_println!("BOUCHAUD_VIRTIO_GPU_OK ecran0={}x{}", largeur0, hauteur0);
        // Le peripherique reste initialise et inutilise : le bureau est sur
        // BGA. Pas de remise a zero -- personne d'autre ne le pilote.
        let _ = commune.l8(DEVICE_STATUS);
        let _ = (desc_p, util_p);
    }
    Ok(())
}
