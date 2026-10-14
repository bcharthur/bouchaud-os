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
//! BOUCHAUD_VIRTIO_GPU_2D_V1 -- le scanout 2D, au-dessus de ce transport :
//!
//!   - un autotest au demarrage : RESOURCE_CREATE_2D, ATTACH_BACKING (memoire
//!     DMA), SET_SCANOUT, TRANSFER_TO_HOST_2D, RESOURCE_FLUSH, puis sept
//!     commandes invalides, chacune refusee avec SON code d'erreur. La mire
//!     64x64 reste a l'ecran jusqu'a ce que le bureau le prenne ;
//!   - le scanout du bureau, quand BGA manque (politique_scanout : GOP, puis
//!     BGA, puis virtio-gpu 2D si l'autotest est vert, sinon aucun) : chaque
//!     present du bureau devient transfert + flush du rectangle presente.
//!
//! Ce n'est PAS une acceleration : le rendu reste fait par le processeur,
//! aucune commande 3D, aucun VIRGL, aucun contexte. Avec BGA, le bureau reste
//! sur BGA. Ne s'execute que si QEMU expose `-device virtio-gpu-pci`
//! (1af4:1050) ; sans lui, une ligne et rien d'autre.

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, AtomicBool, AtomicU64, Ordering};

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
const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const CMD_RESOURCE_UNREF: u32 = 0x0102;
const CMD_SET_SCANOUT: u32 = 0x0103;
const CMD_RESOURCE_FLUSH: u32 = 0x0104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;
const RESP_OK_NODATA: u32 = 0x1100;
const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
const RESP_ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
const RESP_ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
const RESP_ERR_INVALID_PARAMETER: u32 = 0x1205;
/// VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM : en memoire B, G, R, X -- exactement le
/// XRGB8888 petit-boutiste du tampon arriere du bureau.
const FORMAT_B8G8R8X8: u32 = 2;
/// La ressource du bureau, et celle de l'autotest (detruite apres usage).
const RESSOURCE_BUREAU: u32 = 1;
const RESSOURCE_ESSAI: u32 = 0x7E57;
/// Une ressource qu'on ne cree jamais (tests negatifs).
const RESSOURCE_INCONNUE: u32 = 0x00BA_D1D0;
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

        // La controlq est prete : a partir d'ici, chaque commande passe par
        // `FileControle::commande` (une chaine requete -> reponse, scrutee et
        // bornee), sous le verrou du peripherique.
        let mut file = FileControle {
            sonnette: notification + decalage_sonnette,
            taille,
            desc: desc_v as usize,
            dispo: dispo_v as usize,
            util: util_v as usize,
            tampon_p: tamp_p,
            tampon_v: tamp_v as usize,
            prochain_dispo: 0,
            dernier_util: 0,
        };
        let _ = (desc_p, dispo_p, util_p);

        // GET_DISPLAY_INFO.
        let debut = crate::kernel::timer::monotonic_ns();
        let mut requete = [0u8; ENTETE];
        pose32(&mut requete, 0, CMD_GET_DISPLAY_INFO);
        let longueur_reponse = ENTETE + MAX_ECRANS * 24;
        let genre = file.commande(&requete, longueur_reponse).map_err(|_| "display-info-sans-reponse")?;
        let delai_us = (crate::kernel::timer::monotonic_ns() - debut) / 1000;
        if genre != RESP_OK_DISPLAY_INFO {
            crate::serial_println!("VIRTIO_GPU_DISPLAY_INFO reponse={:#06x} attendue={:#06x}", genre, RESP_OK_DISPLAY_INFO);
            return Err("display-info-refuse");
        }
        let reponse = file.reponse();
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
        let _ = commune.l8(DEVICE_STATUS);

        // BOUCHAUD_VIRTIO_GPU_2D_V1 : l'autotest 2D -- une ressource d'essai,
        // sa memoire, le scanout, un transfert, une presentation, puis les
        // refus attendus. Il tourne que BGA soit la ou non : il n'ecrit que
        // sur l'ecran virtio-gpu, jamais sur BGA.
        let autotest = autotest_2d(&mut file);
        let autotest_ok = autotest.is_ok();
        let mire = match autotest {
            Ok(m) => {
                crate::serial_println!("BOUCHAUD_VIRTIO_GPU_2D_AUTOTEST_OK");
                Some(m)
            }
            Err(e) => {
                crate::serial_println!("VIRTIO_GPU_2D_AUTOTEST_ECHEC etape={}", e);
                None
            }
        };
        *GPU.lock() = Some(Peripherique {
            file,
            ecran: (largeur0, hauteur0),
            autotest_ok,
            scanout: None,
            mire,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// BOUCHAUD_VIRTIO_GPU_2D_V1 -- scanout 2D (P10, deuxieme composant)
// ---------------------------------------------------------------------------
//
// Ce que ce code fait : une ressource 2D (B8G8R8X8) dont la memoire d'appui
// est un bloc DMA contigu du noyau ; SET_SCANOUT la montre sur l'ecran 0 ;
// apres chaque copie du bureau dans ce bloc, TRANSFER_TO_HOST_2D puis
// RESOURCE_FLUSH sur le rectangle touche. C'est un REMPLACANT DE BGA pour
// QEMU, pas une acceleration : aucun rendu n'est fait par le GPU, aucun
// contexte 3D, aucune fonctionnalite VIRGL n'est negociee (VERSION_1 seule).
//
// Ce qu'il ne fait pas : 3D, blob, EDID, curseur materiel, plusieurs ecrans,
// interruptions (tout est scrute et borne). virtio-gpu (QEMU) n'est pas le GPU
// de la Trigkey.

/// La file de controle, une commande a la fois.
struct FileControle {
    sonnette: usize,
    taille: u16,
    desc: usize,
    dispo: usize,
    util: usize,
    tampon_p: u64,
    tampon_v: usize,
    /// Prochain indice de l'anneau disponible (libre, modulo 2^16).
    prochain_dispo: u16,
    /// Dernier indice de l'anneau utilise deja consomme.
    dernier_util: u16,
}

/// La requete occupe le debut du tampon, la reponse commence a REPONSE.
const REPONSE: usize = 2048;
const REQUETE_MAX: usize = REPONSE;

impl FileControle {
    /// Envoie une requete et attend sa reponse (scrute, borne a 1 s). Rend le
    /// genre de la reponse (`RESP_OK_*` ou `RESP_ERR_*`).
    unsafe fn commande(&mut self, requete: &[u8], longueur_reponse: usize) -> Result<u32, &'static str> {
        if requete.len() > REQUETE_MAX || longueur_reponse > PAGE - REPONSE {
            return Err("commande-trop-longue");
        }
        let tampon = self.tampon_v as *mut u8;
        core::ptr::copy_nonoverlapping(requete.as_ptr(), tampon, requete.len());
        core::ptr::write_bytes(tampon.add(REPONSE), 0, longueur_reponse);
        let desc = self.desc as *mut u8;
        let pose = |i: usize, adresse: u64, longueur: u32, drapeaux: u16, suivant: u16| {
            let d = desc.add(i * 16);
            write_volatile(d as *mut u64, adresse);
            write_volatile(d.add(8) as *mut u32, longueur);
            write_volatile(d.add(12) as *mut u16, drapeaux);
            write_volatile(d.add(14) as *mut u16, suivant);
        };
        pose(0, self.tampon_p, requete.len() as u32, DESC_SUIVANT, 1);
        pose(1, self.tampon_p + REPONSE as u64, longueur_reponse as u32, DESC_ECRIT_PAR_LE_PERIPHERIQUE, 0);
        // Anneau disponible : flags u16, idx u16, ring[taille] u16.
        let dispo = self.dispo as *mut u16;
        let case = (self.prochain_dispo % self.taille) as usize;
        write_volatile(dispo.add(2 + case), 0); // tete de chaine : descripteur 0
        fence(Ordering::SeqCst);
        self.prochain_dispo = self.prochain_dispo.wrapping_add(1);
        write_volatile(dispo.add(1), self.prochain_dispo);
        fence(Ordering::SeqCst);
        write_volatile(self.sonnette as *mut u16, 0);
        // Anneau utilise : flags u16, idx u16, puis {id u32, len u32}.
        let util = self.util as *const u16;
        let attendu = self.dernier_util.wrapping_add(1);
        if !crate::kernel::timer::attente_bornee(1000, || read_volatile(util.add(1)) == attendu) {
            return Err("commande-sans-reponse");
        }
        fence(Ordering::SeqCst);
        self.dernier_util = attendu;
        Ok(read_volatile(tampon.add(REPONSE) as *const u32))
    }

    fn reponse(&self) -> *const u8 {
        (self.tampon_v + REPONSE) as *const u8
    }
}

fn pose32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn pose64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// Un rectangle virtio-gpu : x, y, largeur, hauteur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub largeur: u32,
    pub hauteur: u32,
}

fn pose_rect(b: &mut [u8], o: usize, r: Rect) {
    pose32(b, o, r.x);
    pose32(b, o + 4, r.y);
    pose32(b, o + 8, r.largeur);
    pose32(b, o + 12, r.hauteur);
}

unsafe fn cree_2d(f: &mut FileControle, ressource: u32, format: u32, l: u32, h: u32) -> Result<u32, &'static str> {
    let mut q = [0u8; 40];
    pose32(&mut q, 0, CMD_RESOURCE_CREATE_2D);
    pose32(&mut q, 24, ressource);
    pose32(&mut q, 28, format);
    pose32(&mut q, 32, l);
    pose32(&mut q, 36, h);
    f.commande(&q, ENTETE)
}

unsafe fn attache(f: &mut FileControle, ressource: u32, physique: u64, octets: u32) -> Result<u32, &'static str> {
    let mut q = [0u8; 48];
    pose32(&mut q, 0, CMD_RESOURCE_ATTACH_BACKING);
    pose32(&mut q, 24, ressource);
    pose32(&mut q, 28, 1);
    pose64(&mut q, 32, physique);
    pose32(&mut q, 40, octets);
    f.commande(&q, ENTETE)
}

unsafe fn detache(f: &mut FileControle, ressource: u32) -> Result<u32, &'static str> {
    let mut q = [0u8; 32];
    pose32(&mut q, 0, CMD_RESOURCE_DETACH_BACKING);
    pose32(&mut q, 24, ressource);
    f.commande(&q, ENTETE)
}

unsafe fn scanout(f: &mut FileControle, ecran: u32, ressource: u32, r: Rect) -> Result<u32, &'static str> {
    let mut q = [0u8; 48];
    pose32(&mut q, 0, CMD_SET_SCANOUT);
    pose_rect(&mut q, 24, r);
    pose32(&mut q, 40, ecran);
    pose32(&mut q, 44, ressource);
    f.commande(&q, ENTETE)
}

unsafe fn transfert(f: &mut FileControle, ressource: u32, r: Rect, decalage: u64) -> Result<u32, &'static str> {
    let mut q = [0u8; 56];
    pose32(&mut q, 0, CMD_TRANSFER_TO_HOST_2D);
    pose_rect(&mut q, 24, r);
    pose64(&mut q, 40, decalage);
    pose32(&mut q, 48, ressource);
    f.commande(&q, ENTETE)
}

unsafe fn vide(f: &mut FileControle, ressource: u32, r: Rect) -> Result<u32, &'static str> {
    let mut q = [0u8; 48];
    pose32(&mut q, 0, CMD_RESOURCE_FLUSH);
    pose_rect(&mut q, 24, r);
    pose32(&mut q, 40, ressource);
    f.commande(&q, ENTETE)
}

unsafe fn libere(f: &mut FileControle, ressource: u32) -> Result<u32, &'static str> {
    let mut q = [0u8; 32];
    pose32(&mut q, 0, CMD_RESOURCE_UNREF);
    pose32(&mut q, 24, ressource);
    f.commande(&q, ENTETE)
}

/// Exige une reponse precise ; nomme l'etape sinon.
fn exige(etape: &'static str, obtenu: Result<u32, &'static str>, attendu: u32) -> Result<(), &'static str> {
    match obtenu {
        Ok(g) if g == attendu => Ok(()),
        Ok(g) => {
            crate::serial_println!("VIRTIO_GPU_2D etape={} reponse={:#06x} attendue={:#06x}", etape, g, attendu);
            Err(etape)
        }
        Err(e) => {
            crate::serial_println!("VIRTIO_GPU_2D etape={} erreur={}", etape, e);
            Err(etape)
        }
    }
}

/// Les couleurs de la mire d'autotest : quatre quadrants purs.
pub const MIRE: [u32; 4] = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0x00FF_FFFF];
const MIRE_COTE: u32 = 64;

/// L'autotest 2D, sur une ressource d'essai detruite a la fin.
///
/// Positif : creer, attacher la memoire, montrer sur l'ecran 0, transferer,
/// presenter, puis detacher et detruire. Negatif : chaque refus attendu doit
/// revenir avec SON code -- un peripherique qui accepterait tout ne prouverait
/// rien, et un pilote qui confondrait les erreurs non plus.
unsafe fn autotest_2d(f: &mut FileControle) -> Result<MireTenue, &'static str> {
    let cote = MIRE_COTE;
    let octets = (cote * cote * 4) as usize;
    let (phys, virt) = memory::alloc_dma(octets).ok_or("dma-mire")?;
    let px = virt as *mut u32;
    for y in 0..cote {
        for x in 0..cote {
            let q = ((y >= cote / 2) as usize) * 2 + (x >= cote / 2) as usize;
            write_volatile(px.add((y * cote + x) as usize), MIRE[q]);
        }
    }
    let tout = Rect { x: 0, y: 0, largeur: cote, hauteur: cote };
    exige("creation", cree_2d(f, RESSOURCE_ESSAI, FORMAT_B8G8R8X8, cote, cote), RESP_OK_NODATA)?;
    exige("attache", attache(f, RESSOURCE_ESSAI, phys, octets as u32), RESP_OK_NODATA)?;
    exige("scanout", scanout(f, 0, RESSOURCE_ESSAI, tout), RESP_OK_NODATA)?;
    exige("transfert", transfert(f, RESSOURCE_ESSAI, tout, 0), RESP_OK_NODATA)?;
    exige("flush", vide(f, RESSOURCE_ESSAI, tout), RESP_OK_NODATA)?;
    crate::serial_println!("VIRTIO_GPU_2D positif creation+attache+scanout+transfert+flush ok ressource={} {}x{}", RESSOURCE_ESSAI, cote, cote);

    // Les refus : chacun avec son code.
    let mut negatifs = 0;
    exige("negatif-format", cree_2d(f, 0x0BAD, 0, 8, 8), RESP_ERR_INVALID_PARAMETER)?;
    negatifs += 1;
    exige("negatif-ressource-zero", cree_2d(f, 0, FORMAT_B8G8R8X8, 8, 8), RESP_ERR_INVALID_RESOURCE_ID)?;
    negatifs += 1;
    exige("negatif-doublon", cree_2d(f, RESSOURCE_ESSAI, FORMAT_B8G8R8X8, 8, 8), RESP_ERR_INVALID_RESOURCE_ID)?;
    negatifs += 1;
    exige("negatif-transfert-inconnue", transfert(f, RESSOURCE_INCONNUE, tout, 0), RESP_ERR_INVALID_RESOURCE_ID)?;
    negatifs += 1;
    exige("negatif-flush-inconnue", vide(f, RESSOURCE_INCONNUE, tout), RESP_ERR_INVALID_RESOURCE_ID)?;
    negatifs += 1;
    exige("negatif-ecran", scanout(f, MAX_ECRANS as u32, RESSOURCE_ESSAI, tout), RESP_ERR_INVALID_SCANOUT_ID)?;
    negatifs += 1;
    let debordant = Rect { x: cote / 2, y: 0, largeur: cote, hauteur: cote };
    exige("negatif-rect-hors-limites", transfert(f, RESSOURCE_ESSAI, debordant, 0), RESP_ERR_INVALID_PARAMETER)?;
    negatifs += 1;
    crate::serial_println!("VIRTIO_GPU_2D negatifs refuses_avec_le_bon_code={}", negatifs);

    // La mire RESTE a l'ecran : c'est elle que le banc capture (screendump
    // de la tete virtio-gpu) et compare pixel a pixel. Retirer le scanout ici
    // laissait QEMU afficher « display inactive » -- une capture qui ne
    // prouvait rien. Le bureau, s'il prend virtio-gpu, la libere en montant
    // sa propre ressource.
    crate::serial_println!(
        "VIRTIO_GPU_2D mire_affichee ressource={} {}x{} quadrants={:#08x},{:#08x},{:#08x},{:#08x}",
        RESSOURCE_ESSAI, cote, cote, MIRE[0], MIRE[1], MIRE[2], MIRE[3]
    );
    Ok(MireTenue { physique: phys, octets })
}

/// La ressource d'essai laissee a l'ecran par l'autotest.
struct MireTenue {
    physique: u64,
    octets: usize,
}

/// Rend la ressource d'essai (une fois que le bureau a son propre scanout).
unsafe fn libere_mire(f: &mut FileControle, m: MireTenue) {
    let detachee = detache(f, RESSOURCE_ESSAI);
    let liberee = libere(f, RESSOURCE_ESSAI);
    if detachee == Ok(RESP_OK_NODATA) && liberee == Ok(RESP_OK_NODATA) {
        memory::free_dma(m.physique, m.octets);
    } else {
        // Le peripherique peut encore lire cette memoire : ne pas la rendre.
        crate::serial_println!("VIRTIO_GPU_2D mire_non_liberee detache={:?} libere={:?}", detachee, liberee);
    }
}

/// Le peripherique, une fois la controlq montee.
struct Peripherique {
    file: FileControle,
    ecran: (u32, u32),
    autotest_ok: bool,
    /// Le scanout du bureau, s'il est actif : (largeur, hauteur, memoire).
    scanout: Option<ScanoutBureau>,
    /// La mire de l'autotest, a l'ecran tant que le bureau ne l'a pas pris.
    mire: Option<MireTenue>,
}

#[derive(Clone, Copy)]
struct ScanoutBureau {
    largeur: u32,
    hauteur: u32,
    physique: u64,
    octets: usize,
}

static GPU: crate::kernel::sync::SpinLock<Option<Peripherique>> = crate::kernel::sync::SpinLock::new(None);

/// Presentations reussies, echouees, et pixels transferes.
static PRESENTS_OK: AtomicU64 = AtomicU64::new(0);
static PRESENTS_ECHEC: AtomicU64 = AtomicU64::new(0);
static PIXELS_TRANSFERES: AtomicU64 = AtomicU64::new(0);
static NS_PRESENT: AtomicU64 = AtomicU64::new(0);
static ACTIF: AtomicBool = AtomicBool::new(false);
/// Au-dela, le scanout virtio est abandonne : le bureau ne peut pas presenter
/// sur un peripherique qui ne repond plus.
const ECHECS_AVANT_ABANDON: u64 = 8;

/// Le peripherique est-il pret a servir de scanout (controlq montee ET
/// autotest 2D reussi) ?
pub fn pret() -> bool {
    GPU.lock().as_ref().is_some_and(|p| p.autotest_ok)
}

/// Le scanout virtio-gpu du bureau est-il actif ?
pub fn actif() -> bool {
    ACTIF.load(Ordering::Acquire)
}

/// Monte le scanout du bureau : une ressource `largeur x hauteur`, sa memoire
/// (un bloc DMA contigu, rendu a l'appelant comme framebuffer lineaire), et
/// SET_SCANOUT sur l'ecran 0. `None` -- et une ligne qui dit pourquoi -- si
/// le peripherique est absent, n'a pas passe l'autotest, ou refuse.
pub fn active_scanout(largeur: usize, hauteur: usize) -> Option<(*mut u32, u64)> {
    let mut garde = GPU.lock();
    let Some(p) = garde.as_mut() else {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison=peripherique-absent");
        return None;
    };
    if !p.autotest_ok {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison=autotest-2d-en-echec");
        return None;
    }
    let (l, h) = (largeur as u32, hauteur as u32);
    let octets = largeur * hauteur * 4;
    let Some((phys, virt)) = memory::alloc_dma(octets) else {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison=dma octets={}", octets);
        return None;
    };
    let tout = Rect { x: 0, y: 0, largeur: l, hauteur: h };
    let etapes = unsafe {
        exige("bureau-creation", cree_2d(&mut p.file, RESSOURCE_BUREAU, FORMAT_B8G8R8X8, l, h), RESP_OK_NODATA)
            .and_then(|_| exige("bureau-attache", attache(&mut p.file, RESSOURCE_BUREAU, phys, octets as u32), RESP_OK_NODATA))
            .and_then(|_| exige("bureau-scanout", scanout(&mut p.file, 0, RESSOURCE_BUREAU, tout), RESP_OK_NODATA))
    };
    if let Err(e) = etapes {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison={}", e);
        unsafe {
            let _ = libere(&mut p.file, RESSOURCE_BUREAU);
        }
        memory::free_dma(phys, octets);
        return None;
    }
    p.scanout = Some(ScanoutBureau { largeur: l, hauteur: h, physique: phys, octets });
    if let Some(m) = p.mire.take() {
        unsafe { libere_mire(&mut p.file, m) };
    }
    ACTIF.store(true, Ordering::Release);
    crate::serial_println!(
        "BOUCHAUD_VIRTIO_GPU_SCANOUT_ACTIF {}x{} ressource={} memoire_phys={:#x} ecran_peripherique={}x{} (scanout 2D, aucune acceleration)",
        l, h, RESSOURCE_BUREAU, phys, p.ecran.0, p.ecran.1
    );
    Some((virt as *mut u32, phys))
}

/// Apres une copie du bureau dans la memoire du scanout : transfert du
/// rectangle vers la ressource de l'hote, puis presentation. Rend `false` si
/// le peripherique a refuse ou n'a pas repondu ; apres ECHECS_AVANT_ABANDON
/// echecs consecutifs le scanout est abandonne (`actif()` faux).
pub fn presente(x: usize, y: usize, largeur: usize, hauteur: usize) -> bool {
    if !actif() {
        return false;
    }
    let debut = crate::kernel::timer::monotonic_ns();
    let mut garde = GPU.lock();
    let Some(p) = garde.as_mut() else { return false };
    let Some(s) = p.scanout else { return false };
    // Rogne au scanout : un rectangle hors limites serait refuse par l'hote
    // (le test negatif le prouve), et une presentation perdue.
    let x1 = (x + largeur).min(s.largeur as usize);
    let y1 = (y + hauteur).min(s.hauteur as usize);
    if x >= x1 || y >= y1 {
        return true;
    }
    let r = Rect { x: x as u32, y: y as u32, largeur: (x1 - x) as u32, hauteur: (y1 - y) as u32 };
    let decalage = ((y * s.largeur as usize + x) * 4) as u64;
    let ok = unsafe {
        transfert(&mut p.file, RESSOURCE_BUREAU, r, decalage) == Ok(RESP_OK_NODATA)
            && vide(&mut p.file, RESSOURCE_BUREAU, r) == Ok(RESP_OK_NODATA)
    };
    if ok {
        PRESENTS_OK.fetch_add(1, Ordering::Relaxed);
        PIXELS_TRANSFERES.fetch_add(u64::from(r.largeur) * u64::from(r.hauteur), Ordering::Relaxed);
        ECHECS_CONSECUTIFS.store(0, Ordering::Relaxed);
    } else {
        PRESENTS_ECHEC.fetch_add(1, Ordering::Relaxed);
        let n = ECHECS_CONSECUTIFS.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= ECHECS_AVANT_ABANDON {
            ACTIF.store(false, Ordering::Release);
            crate::serial_println!("VIRTIO_GPU_SCANOUT_ABANDON echecs_consecutifs={}", n);
        }
    }
    NS_PRESENT.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(debut), Ordering::Relaxed);
    let _ = s.physique;
    let _ = s.octets;
    ok
}

static ECHECS_CONSECUTIFS: AtomicU64 = AtomicU64::new(0);

/// `(presentations_ok, echecs, pixels_transferes, ns_total)`.
pub fn compteurs() -> (u64, u64, u64, u64) {
    (
        PRESENTS_OK.load(Ordering::Relaxed),
        PRESENTS_ECHEC.load(Ordering::Relaxed),
        PIXELS_TRANSFERES.load(Ordering::Relaxed),
        NS_PRESENT.load(Ordering::Relaxed),
    )
}
