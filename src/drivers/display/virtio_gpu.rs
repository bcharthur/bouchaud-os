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
use core::sync::atomic::{fence, AtomicBool, AtomicU16, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use crate::arch::x86_64::pci::{self, Bar, PciDevice};
use crate::drivers::virtio_gpu_dma::{self as dma, Memoire};
use crate::drivers::virtio_gpu_file as file;
use crate::drivers::virtio_gpu_irq::{self as irq, Reveil, Strategie};
use crate::drivers::virtio_gpu_tampons as tampons;
use crate::drivers::virtio_gpu_trame as trame;
use crate::kernel::memory;

const VENDEUR_VIRTIO: u16 = 0x1AF4;
const PERIPHERIQUE_GPU: u16 = 0x1040 + 16;
const CAP_VENDEUR: u8 = 0x09;
const CAP_MSIX: u8 = 0x11;

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
const MSIX_CONFIG: usize = 0x10;
const QUEUE_MSIX_VECTOR: usize = 0x1A;
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
    /// BOUCHAUD_VIRTIO_GPU_IRQ_V1 : decalage de la capacite MSI-X.
    msix: Option<u8>,
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
    let mut c = Capacites { commune: None, notification: None, isr: false, peripherique: false, msix: None };
    let mut liste = [pci::Capacite { identifiant: 0, decalage: 0 }; 16];
    let n = pci::capacites_de(dev, &mut liste);
    for cap in &liste[..n] {
        if cap.identifiant == CAP_MSIX && c.msix.is_none() {
            c.msix = Some(cap.decalage);
            continue;
        }
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
        // BOUCHAUD_VIRTIO_GPU_IRQ_V1 : le vecteur MSI-X de la controlq, avant
        // de l'activer. Un refus laisse la scrutation bornee, et le dit.
        let irq_armee = match c.msix {
            Some(cap) => match arme_msix(dev, cap, &commune) {
                Ok((apic, entrees)) => {
                    UTIL_IDX.store(util_v as usize + 2, Ordering::Release);
                    crate::serial_println!(
                        "VIRTIO_GPU_IRQ msix vecteur={:#04x} apic={} entrees={} controlq=0 (achevement par interruption)",
                        irq::VECTEUR, apic, entrees
                    );
                    true
                }
                Err(raison) => {
                    crate::serial_println!("VIRTIO_GPU_IRQ indisponible raison={} (scrutation bornee)", raison);
                    false
                }
            },
            None => {
                crate::serial_println!("VIRTIO_GPU_IRQ indisponible raison=pas-de-msix (scrutation bornee)");
                false
            }
        };
        commune.e16(QUEUE_ENABLE, 1);
        commune.e8(DEVICE_STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK);
        IRQ_ARMEE.store(irq_armee, Ordering::Release);
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
            cassee: false,
            barriere: 0,
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
        match verrou() {
            Some(mut garde) => {
                *garde = Some(Peripherique {
                    file,
                    ecran: (largeur0, hauteur0),
                    autotest_ok,
                    scanout: None,
                    mire,
                });
            }
            // Impossible au demarrage (personne d'autre ne le tient) ; si
            // cela arrivait, le peripherique est oublie -- jamais un blocage.
            None => crate::serial_println!("VIRTIO_GPU_ECHEC raison=verrou-occupe-a-la-sonde"),
        }
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
    /// Une commande est restee sans reponse : la file n'est plus utilisable.
    ///
    /// Le peripherique peut encore lire la requete ou ecrire la reponse de
    /// cette commande-la. Reutiliser les descripteurs 0 et 1 et le tampon
    /// melerait sa reponse tardive a la commande suivante, dont l'attente
    /// (`dernier_util + 1`) serait d'ailleurs deja satisfaite. On ne
    /// renvoie donc plus rien sur cette file.
    cassee: bool,
    /// BOUCHAUD_VIRTIO_GPU_BARRIERE_V1 : le dernier `fence_id` envoye. Chaque
    /// commande en porte un nouveau ; sa reponse doit le rendre.
    barriere: u64,
}

/// La requete occupe le debut du tampon, la reponse commence a REPONSE.
const REPONSE: usize = 2048;
const REQUETE_MAX: usize = REPONSE;

impl FileControle {
    /// Envoie une requete et attend sa reponse (scrute, borne a 1 s). Rend le
    /// genre de la reponse (`RESP_OK_*` ou `RESP_ERR_*`).
    ///
    /// BOUCHAUD_VIRTIO_GPU_BARRIERE_V1 : UNE seule commande en vol, toujours
    /// avec `VIRTIO_GPU_FLAG_FENCE` -- sa reponse prouve que le traitement est
    /// fini (la specification permet sinon de la rendre AVANT). La reponse
    /// est jugee par `virtio_gpu_file::juge` ; une reponse qui n'est pas celle
    /// de la commande en vol casse la file (`REPONSE_INVALIDE`, issue tenue
    /// pour inconnue).
    unsafe fn commande(&mut self, requete: &[u8], longueur_reponse: usize) -> Result<u32, &'static str> {
        if self.cassee {
            return Err("file-sans-reponse");
        }
        if requete.len() > REQUETE_MAX || longueur_reponse > PAGE - REPONSE {
            return Err("commande-trop-longue");
        }
        if requete.len() < file::ENTETE as usize || longueur_reponse < file::ENTETE as usize {
            return Err("commande-trop-courte");
        }
        let tampon = self.tampon_v as *mut u8;
        core::ptr::copy_nonoverlapping(requete.as_ptr(), tampon, requete.len());
        self.barriere = self.barriere.wrapping_add(1);
        file::pose_barriere(core::slice::from_raw_parts_mut(tampon, requete.len()), self.barriere);
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
        // BANC DE PANNE (option `banc-virtio-gpu-sans-reponse`, jamais dans
        // une image livree) : la requete est PARTIE, on fait comme si sa
        // reponse n'arrivait jamais.
        #[cfg(feature = "banc-virtio-gpu-sans-reponse")]
        if lit32(requete, 0) == CMD_RESOURCE_ATTACH_BACKING && lit32(requete, 24) == RESSOURCE_BUREAU {
            crate::serial_println!("VIRTIO_GPU_INJECTION attache_bureau_sans_reponse (requete envoyee, attente declaree echue)");
            self.cassee = true;
            return Err(dma::SANS_REPONSE);
        }
        // Anneau utilise : flags u16, idx u16, puis {id u32, len u32}.
        // L'attente se termine sur TOUT progres de `idx` : une avance de plus
        // d'une entree est une violation, pas une reponse.
        let util = self.util as *const u16;
        let dernier = self.dernier_util;
        if !attente_achevement(1000, || read_volatile(util.add(1)) != dernier) {
            self.cassee = true;
            return Err(dma::SANS_REPONSE);
        }
        fence(Ordering::SeqCst);
        let avance = read_volatile(util.add(1)).wrapping_sub(dernier);
        let element = (self.util + 4 + 8 * (dernier % self.taille) as usize) as *const u32;
        let utilise = file::Utilise { id: read_volatile(element), len: read_volatile(element.add(1)) };
        let reponse = tampon.add(REPONSE);
        let genre = read_volatile(reponse as *const u32);
        let drapeaux = read_volatile(reponse.add(file::DECALAGE_DRAPEAUX) as *const u32);
        let barriere = core::ptr::read_unaligned(reponse.add(file::DECALAGE_BARRIERE) as *const u64);
        #[allow(unused_mut)]
        let mut attendue = self.barriere;
        // BANC DE PANNE (option `banc-virtio-gpu-reponse-etrangere`, jamais
        // dans une image livree) : le premier transfert du bureau est juge
        // contre une barriere qui n'est pas la sienne -- la reponse d'une
        // autre commande.
        #[cfg(feature = "banc-virtio-gpu-reponse-etrangere")]
        if lit_genre(requete) == CMD_TRANSFER_TO_HOST_2D
            && lit32(requete, 48) == RESSOURCE_BUREAU
            && !INJECTION_ETRANGERE_FAITE.swap(true, Ordering::Relaxed)
        {
            crate::serial_println!("VIRTIO_GPU_INJECTION reponse_etrangere (transfert du bureau juge contre une autre barriere)");
            attendue = attendue.wrapping_add(1);
        }
        if let Err(v) = file::juge(avance, 0, utilise, longueur_reponse as u32, drapeaux, barriere, attendue) {
            // La file ne sert plus : rien de ce qu'elle rendrait ensuite ne
            // pourrait etre rattache a une commande.
            self.cassee = true;
            crate::serial_println!(
                "VIRTIO_GPU_FILE violation={} commande={:#x} avance={} id={} len={} drapeaux={:#x} barriere={} attendue={}",
                v.nom(), lit_genre(requete), avance, utilise.id, utilise.len, drapeaux, barriere, attendue
            );
            return Err(dma::REPONSE_INVALIDE);
        }
        self.dernier_util = dernier.wrapping_add(1);
        Ok(genre)
    }

    fn reponse(&self) -> *const u8 {
        (self.tampon_v + REPONSE) as *const u8
    }
}

/// Le genre d'une requete (pour le journal).
fn lit_genre(requete: &[u8]) -> u32 {
    u32::from_le_bytes([requete[0], requete[1], requete[2], requete[3]])
}

#[cfg(feature = "banc-virtio-gpu-reponse-etrangere")]
static INJECTION_ETRANGERE_FAITE: AtomicBool = AtomicBool::new(false);

#[cfg(any(feature = "banc-virtio-gpu-sans-reponse", feature = "banc-virtio-gpu-reponse-etrangere"))]
fn lit32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
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
    // BOUCHAUD_VIRTIO_GPU_DMA_ETAT_V1 : qui peut encore toucher cette memoire.
    let mut memoire = Memoire::NonAttachee;
    match autotest_2d_etapes(f, phys, virt, octets, &mut memoire) {
        Ok(()) => Ok(MireTenue { physique: phys, octets, memoire }),
        Err(e) => {
            rend_si_sur(f, RESSOURCE_ESSAI, phys, octets, &mut memoire, "mire");
            Err(e)
        }
    }
}

/// Note l'issue d'une commande qui n'attache ni ne detache la memoire suivie.
fn suivie(m: &mut Memoire, rendu: Result<u32, &'static str>) -> Result<u32, &'static str> {
    *m = dma::apres_autre(*m, dma::issue(rendu));
    rendu
}

/// Apres un echec : detacher si c'est possible, liberer la ressource, et ne
/// rendre la memoire que si le peripherique ne peut plus la tenir. Sinon elle
/// est conservee -- une fuite bornee plutot qu'une corruption -- et le journal
/// dit pourquoi.
unsafe fn rend_si_sur(f: &mut FileControle, ressource: u32, phys: u64, octets: usize, m: &mut Memoire, quoi: &str) {
    if *m == Memoire::Attachee {
        *m = dma::apres_detache(*m, dma::issue(detache(f, ressource)));
    }
    let _ = suivie(m, libere(f, ressource));
    if dma::peut_rendre(*m) {
        memory::free_dma(phys, octets);
    } else {
        crate::serial_println!(
            "VIRTIO_GPU_DMA memoire_conservee quoi={} octets={} etat={:?} (le peripherique peut encore la tenir)",
            quoi, octets, *m
        );
    }
}

unsafe fn autotest_2d_etapes(
    f: &mut FileControle,
    phys: u64,
    virt: *mut u8,
    octets: usize,
    m: &mut Memoire,
) -> Result<(), &'static str> {
    let cote = MIRE_COTE;
    let px = virt as *mut u32;
    for y in 0..cote {
        for x in 0..cote {
            let q = ((y >= cote / 2) as usize) * 2 + (x >= cote / 2) as usize;
            write_volatile(px.add((y * cote + x) as usize), MIRE[q]);
        }
    }
    let tout = Rect { x: 0, y: 0, largeur: cote, hauteur: cote };
    exige("creation", suivie(m, cree_2d(f, RESSOURCE_ESSAI, FORMAT_B8G8R8X8, cote, cote)), RESP_OK_NODATA)?;
    let rendu = attache(f, RESSOURCE_ESSAI, phys, octets as u32);
    *m = dma::apres_attache(*m, dma::issue(rendu));
    exige("attache", rendu, RESP_OK_NODATA)?;
    exige("scanout", suivie(m, scanout(f, 0, RESSOURCE_ESSAI, tout)), RESP_OK_NODATA)?;
    exige("transfert", suivie(m, transfert(f, RESSOURCE_ESSAI, tout, 0)), RESP_OK_NODATA)?;
    exige("flush", suivie(m, vide(f, RESSOURCE_ESSAI, tout)), RESP_OK_NODATA)?;
    crate::serial_println!("VIRTIO_GPU_2D positif creation+attache+scanout+transfert+flush ok ressource={} {}x{}", RESSOURCE_ESSAI, cote, cote);

    // Les refus : chacun avec son code.
    let mut negatifs = 0;
    exige("negatif-format", suivie(m, cree_2d(f, 0x0BAD, 0, 8, 8)), RESP_ERR_INVALID_PARAMETER)?;
    negatifs += 1;
    exige("negatif-ressource-zero", suivie(m, cree_2d(f, 0, FORMAT_B8G8R8X8, 8, 8)), RESP_ERR_INVALID_RESOURCE_ID)?;
    negatifs += 1;
    exige("negatif-doublon", suivie(m, cree_2d(f, RESSOURCE_ESSAI, FORMAT_B8G8R8X8, 8, 8)), RESP_ERR_INVALID_RESOURCE_ID)?;
    negatifs += 1;
    exige("negatif-transfert-inconnue", suivie(m, transfert(f, RESSOURCE_INCONNUE, tout, 0)), RESP_ERR_INVALID_RESOURCE_ID)?;
    negatifs += 1;
    exige("negatif-flush-inconnue", suivie(m, vide(f, RESSOURCE_INCONNUE, tout)), RESP_ERR_INVALID_RESOURCE_ID)?;
    negatifs += 1;
    exige("negatif-ecran", suivie(m, scanout(f, MAX_ECRANS as u32, RESSOURCE_ESSAI, tout)), RESP_ERR_INVALID_SCANOUT_ID)?;
    negatifs += 1;
    let debordant = Rect { x: cote / 2, y: 0, largeur: cote, hauteur: cote };
    exige("negatif-rect-hors-limites", suivie(m, transfert(f, RESSOURCE_ESSAI, debordant, 0)), RESP_ERR_INVALID_PARAMETER)?;
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
    Ok(())
}

/// La ressource d'essai laissee a l'ecran par l'autotest.
struct MireTenue {
    physique: u64,
    octets: usize,
    /// `Attachee` quand l'autotest a reussi.
    memoire: Memoire,
}

/// Rend la ressource d'essai (une fois que le bureau a son propre scanout).
/// La memoire n'est rendue que detachee AVEC reponse (voir `virtio_gpu_dma`).
unsafe fn libere_mire(f: &mut FileControle, mut m: MireTenue) {
    rend_si_sur(f, RESSOURCE_ESSAI, m.physique, m.octets, &mut m.memoire, "mire");
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
    virt: *mut u32,
    octets: usize,
}

// La memoire du scanout n'est touchee que sous le verrou du peripherique et
// par le bureau qui l'a recue : le pointeur ne circule pas ailleurs.
unsafe impl Send for ScanoutBureau {}

// BOUCHAUD_VIRTIO_GPU_VERROU_DORMANT_V1
//
// Le peripherique etait sous un `SpinLock`, tenu pendant des commandes
// SYNCHRONES : deux par presentation (182 us sous KVM, 230 a 860 us sous
// TCG), jusqu'a une seconde si le peripherique ne repond pas. Un `SpinLock`
// tenu marque le coeur NON PREEMPTIBLE : aucune tache prete ne passait sur ce
// coeur pendant ce temps, et l'attente elle-meme tournait a vide.
//
// Desormais un `SleepMutex` (le detenteur reste preemptible) et une attente
// de reponse qui CEDE le processeur entre deux lectures (`attente_reponse`).
// Hors de toute tache (sonde au demarrage) ou interruptions masquees, on ne
// bloque jamais : `try_lock` seulement (voir `verrou`).
static GPU: crate::kernel::sync::SleepMutex<Option<Peripherique>> = crate::kernel::sync::SleepMutex::new(None);

/// Le verrou du peripherique, sans jamais s'endormir la ou c'est interdit.
fn verrou() -> Option<crate::kernel::sync::SleepMutexGuard<'static, Option<Peripherique>>> {
    if contexte_cedable() {
        Some(GPU.lock())
    } else {
        GPU.try_lock()
    }
}

/// Peut-on dormir ou ceder le processeur ici ? Une tache courante, les
/// interruptions ouvertes, et AUCUN verrou tournant tenu par l'appelant (ni
/// `preempt_count`, ni lecture du registre, ni shootdown en vol) : voir
/// `preempt::cession_volontaire_sure`.
fn contexte_cedable() -> bool {
    crate::kernel::scheduler::preempt::cession_volontaire_sure()
}

/// Presentations sautees faute de pouvoir prendre le verrou sans bloquer.
static PRESENTS_SAUTES: AtomicU64 = AtomicU64::new(0);
/// Fois ou l'attente d'une reponse a cede le processeur.
static ATTENTES_CEDEES: AtomicU64 = AtomicU64::new(0);
/// Attentes de reponse faites dans un contexte ou ceder est interdit (elles
/// tournent jusqu'a la reponse ou la borne).
static ATTENTES_NON_CEDABLES: AtomicU64 = AtomicU64::new(0);
/// ... dont celles faites DANS une tache (un appelant tient un verrou
/// tournant ou a masque les interruptions). Attendu : zero.
static NON_CEDABLES_EN_TACHE: AtomicU64 = AtomicU64::new(0);

/// Rotation avant de ceder : sous KVM la reponse arrive en quelques dizaines
/// de microsecondes ; ceder plus tot couterait un passage dans l'ordonnanceur
/// a chaque commande.
const ROTATION_AVANT_CEDER_NS: u64 = 50_000;

/// Attend `predicat`, au plus `limite_ms`. Tourne d'abord (reponse rapide),
/// puis CEDE le processeur entre deux lectures si le contexte le permet. Le
/// plafond en cycles TSC garantit la fin meme si l'horloge monotone n'avance
/// pas encore (sonde au demarrage).
fn attente_reponse(limite_ms: u64, mut predicat: impl FnMut() -> bool) -> bool {
    let cedable = contexte_cedable();
    if !cedable {
        ATTENTES_NON_CEDABLES.fetch_add(1, Ordering::Relaxed);
        // Hors tache (sonde et autotest au demarrage), c'est attendu. DANS une
        // tache, c'est qu'un appelant tient un verrou tournant ou a masque les
        // interruptions : on le dit (8 fois au plus), avec ce qui l'interdit.
        if crate::kernel::task::in_user_task() {
            let n = NON_CEDABLES_EN_TACHE.fetch_add(1, Ordering::Relaxed);
            if n < 8 {
                if let Some((c, irq)) = crate::kernel::scheduler::preempt::contexte_cession() {
                    crate::serial_println!(
                        "VIRTIO_GPU_ATTENTE non_cedable_en_tache n={} if={} irq={} preempt={} spin={} lockdep={} registre={} shootdown={} sortie={}",
                        n, c.interruptions_ouvertes as u8, irq, c.preempt_count, c.verrous_simples,
                        c.profondeur_lockdep, c.lectures_registre, c.shootdown_en_vol as u8, c.sortie_en_cours as u8
                    );
                }
            }
        }
    }
    let debut = crate::kernel::timer::monotonic_ns();
    let limite_ns = limite_ms.saturating_mul(1_000_000);
    let debut_tsc = crate::arch::x86_64::cpu::rdtsc();
    // ~4 GHz au plus : un plafond large, jamais atteint avant l'horloge.
    let plafond_cycles = limite_ms.saturating_mul(4_000_000).max(1_000_000);
    loop {
        if predicat() {
            return true;
        }
        let ecoule = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
        if ecoule > limite_ns || crate::arch::x86_64::cpu::rdtsc().wrapping_sub(debut_tsc) > plafond_cycles {
            return false;
        }
        if cedable && ecoule > ROTATION_AVANT_CEDER_NS {
            ATTENTES_CEDEES.fetch_add(1, Ordering::Relaxed);
            crate::kernel::task::yield_now();
        } else {
            core::hint::spin_loop();
        }
    }
}

// ---------------------------------------------------------------------------
// BOUCHAUD_VIRTIO_GPU_IRQ_V1 -- achevement par interruption (P10+.1)
// ---------------------------------------------------------------------------
//
// Voir `virtio_gpu_irq.rs` pour le modele. Ici : l'armement MSI-X, le
// gestionnaire (appele par `idt/virtio_gpu.rs`) et l'attente dormante.

/// Le peripherique a accepte le vecteur de la controlq.
static IRQ_ARMEE: AtomicBool = AtomicBool::new(false);
/// La politique : dormir sur l'interruption quand c'est possible (commande
/// `gpu-irq on|off`, pour l'A/B). ACTIVEE par defaut depuis l'A/B KVM
/// CONVAINCANT de f23a1247 (run 38697040371, 6/6 tours AB/BA : latences au
/// niveau de la scrutation, coeur inactif 201 pour mille en plein ecran
/// contre 0, aucune interruption perdue sur 3 840 presentations ; pannes
/// MSI-X masque / parasites / aveugle vertes sous TCG et KVM). La CI rejoue
/// l'A/B a chaque campagne : un KVM NON_CONVAINCANT rend le defaut a la
/// scrutation (BOUCHAUD_VIRTIO_GPU_IRQ_V1, P10+.1).
static IRQ_VOULUE: AtomicBool = AtomicBool::new(true);
/// L'adresse virtuelle de `used.idx` (0 avant l'armement).
static UTIL_IDX: AtomicUsize = AtomicUsize::new(0);
/// L'entree 0 de la table MSI-X (banc de panne : la masquer).
#[cfg_attr(not(feature = "banc-virtio-gpu-irq-pannes"), allow(dead_code))]
static ENTREE_MSIX: AtomicUsize = AtomicUsize::new(0);
/// Le dernier `used.idx` vu par le gestionnaire.
static IRQ_VU: AtomicU16 = AtomicU16::new(0);
/// La file des achevements : le gestionnaire la reveille, l'attendant y dort.
static ACHEVEMENTS: crate::kernel::sync::WaitQueue = crate::kernel::sync::WaitQueue::new();

static IRQ_TOTAL: AtomicU64 = AtomicU64::new(0);
static IRQ_SANS_PROGRES: AtomicU64 = AtomicU64::new(0);
static ATTENTES_DORMANTES: AtomicU64 = AtomicU64::new(0);
static ACHEVES_EN_ROTATION: AtomicU64 = AtomicU64::new(0);
static SOMMEILS: AtomicU64 = AtomicU64::new(0);
static REVEILS_SANS_PROGRES: AtomicU64 = AtomicU64::new(0);
static VUS_PAR_RELECTURE: AtomicU64 = AtomicU64::new(0);
static ATTENTES_DORMANTES_ECHUES: AtomicU64 = AtomicU64::new(0);

/// Rotation avant de dormir : sous KVM la reponse arrive en quelques
/// dizaines de microsecondes, un sommeil couterait plus qu'il n'epargne.
const ROTATION_IRQ_NS: u64 = 20_000;
/// Le filet d'une interruption perdue : la reponse est relue au plus tard a
/// ce rythme, et chaque achevement vu ainsi est compte.
const RELECTURE_NS: u64 = 2_000_000;

/// Programme l'entree 0 de la table MSI-X vers le LAPIC de ce coeur, associe
/// la controlq a cette entree, et active MSI-X. Rend (apic, entrees).
///
/// L'entree est masquee pendant sa programmation et la fonction masquee
/// pendant l'association ; un refus du peripherique (`queue_msix_vector`
/// relu different) desactive MSI-X et laisse la scrutation.
unsafe fn arme_msix(dev: &PciDevice, cap: u8, commune: &Region) -> Result<(u32, u16), &'static str> {
    let mot0 = pci::config_lit32(dev, cap);
    let controle = (mot0 >> 16) as u16;
    let entrees = (controle & 0x7FF) + 1;
    let (bir, decalage) = irq::emplacement_table(pci::config_lit32(dev, cap.saturating_add(4)));
    let table = adresse(dev, bir, decalage).ok_or("msix-table-hors-bar-memoire")?;
    let apic = crate::arch::x86_64::smp::hardware_apic_id() as u32;
    let (adresse_bas, adresse_haut, donnee) = irq::message(apic, irq::VECTEUR).ok_or("msix-apic-hors-format-xapic")?;
    let entree = table as *mut u32;
    write_volatile(entree.add(3), 1);
    write_volatile(entree, adresse_bas);
    write_volatile(entree.add(1), adresse_haut);
    write_volatile(entree.add(2), donnee);
    let ecrit_controle = |c: u16| pci::config_ecrit32(dev, cap, (mot0 & 0xFFFF) | (u32::from(c) << 16));
    ecrit_controle(controle | irq::MSIX_ACTIVE | irq::MSIX_MASQUE_FONCTION);
    commune.e16(MSIX_CONFIG, irq::AUCUN_VECTEUR);
    commune.e16(QUEUE_SELECT, 0);
    commune.e16(QUEUE_MSIX_VECTOR, 0);
    if commune.l16(QUEUE_MSIX_VECTOR) != 0 {
        commune.e16(QUEUE_MSIX_VECTOR, irq::AUCUN_VECTEUR);
        ecrit_controle(controle & !irq::MSIX_ACTIVE);
        return Err("vecteur-refuse-par-le-peripherique");
    }
    write_volatile(entree.add(3), 0);
    ecrit_controle((controle | irq::MSIX_ACTIVE) & !irq::MSIX_MASQUE_FONCTION);
    ENTREE_MSIX.store(entree as usize, Ordering::Release);
    Ok((apic, entrees))
}

/// Le gestionnaire de `irq::VECTEUR` (contexte d'interruption) : compter,
/// reperer une interruption sans progres, reveiller. Aucun verrou, aucune
/// commande, aucune impression.
pub fn interruption() {
    IRQ_TOTAL.fetch_add(1, Ordering::Relaxed);
    let p = UTIL_IDX.load(Ordering::Acquire);
    if p != 0 {
        let idx = unsafe { read_volatile(p as *const u16) };
        let vu = IRQ_VU.swap(idx, Ordering::Relaxed);
        if !irq::progres(vu, idx) {
            IRQ_SANS_PROGRES.fetch_add(1, Ordering::Relaxed);
        }
    }
    ACHEVEMENTS.wake_all();
}

/// Le monde de l'attente dormante : l'horloge monotone et la file des
/// achevements. Le ticket est pris AVANT la derniere relecture.
struct MondeNoyau;

impl irq::Dormeur for MondeNoyau {
    fn maintenant(&mut self) -> u64 {
        crate::kernel::timer::monotonic_ns()
    }
    fn dors(&mut self, jusqu_a: u64, pret: &mut dyn FnMut() -> bool) -> Reveil {
        let ticket = ACHEVEMENTS.ticket();
        if pret() {
            return Reveil::DejaPret;
        }
        // BANC DE PANNE : une interruption parasite APRES le ticket, alors que
        // la reponse n'est pas la -- elle reveille, et ne doit rien achever.
        #[cfg(feature = "banc-virtio-gpu-irq-pannes")]
        if pannes::parasite_due() {
            pannes::parasite();
        }
        // Une borne qui a croise l'interruption n'est pas une interruption
        // perdue : le ticket dit si un reveil a ete signale.
        if ACHEVEMENTS.wait_until_noyau(ticket, jusqu_a) || ACHEVEMENTS.signale_depuis(ticket) {
            Reveil::Interruption
        } else {
            Reveil::Borne
        }
    }
}

/// Attend une reponse : sommeil sur l'interruption si tout le permet
/// (`irq::strategie`), sinon la scrutation bornee d'avant.
fn attente_achevement(limite_ms: u64, mut predicat: impl FnMut() -> bool) -> bool {
    let strategie = irq::strategie(
        contexte_cedable(),
        IRQ_ARMEE.load(Ordering::Acquire),
        IRQ_VOULUE.load(Ordering::Relaxed),
    );
    if strategie == Strategie::Scrute {
        return attente_reponse(limite_ms, predicat);
    }
    let rang = ATTENTES_DORMANTES.fetch_add(1, Ordering::Relaxed) + 1;
    let echeance = crate::kernel::timer::monotonic_ns().saturating_add(limite_ms.saturating_mul(1_000_000));
    #[cfg(feature = "banc-virtio-gpu-irq-pannes")]
    let aveugle = pannes::avant_attente(rang);
    #[cfg(not(feature = "banc-virtio-gpu-irq-pannes"))]
    let aveugle = { let _ = rang; false };
    // BANC DE PANNE : une reponse jamais vue (la vraie arrive, l'attente ne
    // la lit pas) -- l'echeance doit finir l'attente.
    let mut lit = || !aveugle && predicat();
    let b = irq::attend(&mut lit, &mut MondeNoyau, echeance, ROTATION_IRQ_NS, RELECTURE_NS);
    if b.vu_en_rotation {
        ACHEVES_EN_ROTATION.fetch_add(1, Ordering::Relaxed);
    }
    SOMMEILS.fetch_add(u64::from(b.sommeils), Ordering::Relaxed);
    REVEILS_SANS_PROGRES.fetch_add(u64::from(b.reveils_sans_progres), Ordering::Relaxed);
    if b.vu_par_relecture {
        VUS_PAR_RELECTURE.fetch_add(1, Ordering::Relaxed);
    }
    if !b.reussi {
        ATTENTES_DORMANTES_ECHUES.fetch_add(1, Ordering::Relaxed);
    }
    b.reussi
}

/// BANC DE PANNE (option `banc-virtio-gpu-irq-pannes`, jamais dans une image
/// livree) : les trois pannes de l'attente par interruption, par rang
/// d'attente dormante.
#[cfg(feature = "banc-virtio-gpu-irq-pannes")]
mod pannes {
    use super::*;

    /// Attentes [MASQUE_DEBUT, MASQUE_FIN) : entree MSI-X masquee.
    const MASQUE_DEBUT: u64 = 4;
    const MASQUE_FIN: u64 = 14;
    /// Attentes [PARASITES_DEBUT, PARASITES_FIN) : une parasite par sommeil.
    const PARASITES_DEBUT: u64 = 16;
    const PARASITES_FIN: u64 = 22;
    /// L'attente dont la reponse n'est jamais vue.
    const AVEUGLE: u64 = 26;

    static RANG: AtomicU64 = AtomicU64::new(0);
    static PARASITES: AtomicU64 = AtomicU64::new(0);

    pub(super) fn parasite() {
        PARASITES.fetch_add(1, Ordering::Relaxed);
        crate::arch::x86_64::smp::ipi_vers_soi(irq::VECTEUR);
    }

    fn masque(oui: bool) {
        let e = ENTREE_MSIX.load(Ordering::Acquire);
        if e != 0 {
            unsafe { write_volatile((e as *mut u32).add(3), oui as u32) };
        }
    }

    /// Rend vrai si cette attente doit etre aveugle.
    pub(super) fn avant_attente(rang: u64) -> bool {
        RANG.store(rang, Ordering::Relaxed);
        if rang == MASQUE_DEBUT {
            masque(true);
            crate::serial_println!("VIRTIO_GPU_INJECTION irq_masquee attentes={}..{}", MASQUE_DEBUT, MASQUE_FIN);
        } else if rang == MASQUE_FIN {
            masque(false);
            crate::serial_println!("VIRTIO_GPU_INJECTION irq_demasquee");
        } else if rang == PARASITES_DEBUT {
            crate::serial_println!("VIRTIO_GPU_INJECTION irq_parasites attentes={}..{}", PARASITES_DEBUT, PARASITES_FIN);
        } else if rang == PARASITES_FIN {
            crate::serial_println!(
                "VIRTIO_GPU_INJECTION irq_parasites_fin envoyees={}", PARASITES.load(Ordering::Relaxed)
            );
        } else if rang == AVEUGLE {
            crate::serial_println!("VIRTIO_GPU_INJECTION reponse_jamais_vue attente={} (echeance attendue)", AVEUGLE);
        }
        // Une parasite certaine par attente de la fenetre (hors de tout
        // sommeil), en plus de celle envoyee pendant un sommeil (`dors`).
        if (PARASITES_DEBUT..PARASITES_FIN).contains(&rang) {
            parasite();
        }
        rang == AVEUGLE
    }

    pub(super) fn parasite_due() -> bool {
        (PARASITES_DEBUT..PARASITES_FIN).contains(&RANG.load(Ordering::Relaxed))
    }
}

/// La politique d'attente (commande `gpu-irq`). Rend l'etat precedent.
pub fn veut_irq(oui: bool) -> bool {
    IRQ_VOULUE.swap(oui, Ordering::Relaxed)
}

/// Ce que l'achevement par interruption a fait.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompteursIrq {
    pub armee: bool,
    pub voulue: bool,
    pub irq: u64,
    pub irq_sans_progres: u64,
    pub attentes_dormantes: u64,
    pub acheves_en_rotation: u64,
    pub sommeils: u64,
    pub reveils_sans_progres: u64,
    pub vus_par_relecture: u64,
    pub echues: u64,
    /// Scrutation : cessions et attentes non cedables (voir `compteurs_verrou`).
    pub cessions: u64,
    pub non_cedables: u64,
}

pub fn compteurs_irq() -> CompteursIrq {
    CompteursIrq {
        armee: IRQ_ARMEE.load(Ordering::Acquire),
        voulue: IRQ_VOULUE.load(Ordering::Relaxed),
        irq: IRQ_TOTAL.load(Ordering::Relaxed),
        irq_sans_progres: IRQ_SANS_PROGRES.load(Ordering::Relaxed),
        attentes_dormantes: ATTENTES_DORMANTES.load(Ordering::Relaxed),
        acheves_en_rotation: ACHEVES_EN_ROTATION.load(Ordering::Relaxed),
        sommeils: SOMMEILS.load(Ordering::Relaxed),
        reveils_sans_progres: REVEILS_SANS_PROGRES.load(Ordering::Relaxed),
        vus_par_relecture: VUS_PAR_RELECTURE.load(Ordering::Relaxed),
        echues: ATTENTES_DORMANTES_ECHUES.load(Ordering::Relaxed),
        cessions: ATTENTES_CEDEES.load(Ordering::Relaxed),
        non_cedables: ATTENTES_NON_CEDABLES.load(Ordering::Relaxed),
    }
}

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
    verrou().is_some_and(|g| g.as_ref().is_some_and(|p| p.autotest_ok))
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
    let Some(mut garde) = verrou() else {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison=verrou-indisponible-sans-bloquer");
        return None;
    };
    let Some(p) = garde.as_mut() else {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison=peripherique-absent");
        return None;
    };
    if !p.autotest_ok {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison=autotest-2d-en-echec");
        return None;
    }
    let (l, h) = (largeur as u32, hauteur as u32);
    // BOUCHAUD_VIRTIO_GPU_SCANOUT_CYCLE_V1 : un second `enter()` (bureau
    // quitte puis relance, deux bancs dans le meme demarrage) recreait la
    // ressource 1 sans avoir rendu la precedente. Le peripherique refusait
    // (`RESP_ERR_INVALID_RESOURCE_ID`), et le chemin d'echec faisait UNREF de
    // la ressource 1 -- l'ANCIENNE, encore a l'ecran : sa memoire DMA fuyait
    // et le scanout s'eteignait ; un demarrage sur deux restait sans ecran.
    // Meme geometrie : la ressource est reprise. Autre geometrie : l'ancienne
    // est rendue d'abord (scanout eteint, DETACH, UNREF, memoire selon son
    // etat).
    if let Some(s) = p.scanout {
        let tout = Rect { x: 0, y: 0, largeur: l, hauteur: h };
        if s.largeur == l && s.hauteur == h {
            let repris = exige("bureau-repris", unsafe { scanout(&mut p.file, 0, RESSOURCE_BUREAU, tout) }, RESP_OK_NODATA);
            if let Err(e) = repris {
                crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison={}", e);
                return None;
            }
            ACTIF.store(true, Ordering::Release);
            crate::serial_println!(
                "BOUCHAUD_VIRTIO_GPU_SCANOUT_REPRIS {}x{} ressource={} memoire_phys={:#x} (meme ressource, aucune recreation)",
                l, h, RESSOURCE_BUREAU, s.physique
            );
            return Some((s.virt, s.physique));
        }
        unsafe { rend_scanout(p, s) };
    }
    let octets = largeur * hauteur * 4;
    let Some((phys, virt)) = memory::alloc_dma(octets) else {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison=dma octets={}", octets);
        return None;
    };
    let tout = Rect { x: 0, y: 0, largeur: l, hauteur: h };
    // BOUCHAUD_VIRTIO_GPU_DMA_ETAT_V1 : un `ATTACH_BACKING` SANS REPONSE
    // laissait « jamais attachee » et la memoire etait rendue alors que le
    // peripherique pouvait l'avoir retenue. L'etat est suivi commande par
    // commande ; sur l'echec, `rend_si_sur` ne la rend que si plus rien ne
    // peut la tenir.
    let mut memoire = Memoire::NonAttachee;
    let etapes = unsafe {
        let f = &mut p.file;
        exige("bureau-creation", suivie(&mut memoire, cree_2d(f, RESSOURCE_BUREAU, FORMAT_B8G8R8X8, l, h)), RESP_OK_NODATA)
            .and_then(|_| {
                let rendu = attache(f, RESSOURCE_BUREAU, phys, octets as u32);
                memoire = dma::apres_attache(memoire, dma::issue(rendu));
                exige("bureau-attache", rendu, RESP_OK_NODATA)
            })
            .and_then(|_| exige("bureau-scanout", suivie(&mut memoire, scanout(f, 0, RESSOURCE_BUREAU, tout)), RESP_OK_NODATA))
    };
    if let Err(e) = etapes {
        crate::serial_println!("VIRTIO_GPU_SCANOUT refuse raison={}", e);
        unsafe { rend_si_sur(&mut p.file, RESSOURCE_BUREAU, phys, octets, &mut memoire, "bureau") };
        return None;
    }
    p.scanout = Some(ScanoutBureau { largeur: l, hauteur: h, physique: phys, virt: virt as *mut u32, octets });
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

/// Rend le scanout du bureau : ecran eteint (SET_SCANOUT ressource 0),
/// DETACH, UNREF ; la memoire n'est rendue que si le peripherique ne peut
/// plus la tenir (`virtio_gpu_dma`).
unsafe fn rend_scanout(p: &mut Peripherique, s: ScanoutBureau) {
    ACTIF.store(false, Ordering::Release);
    let eteint = scanout(&mut p.file, 0, 0, Rect { x: 0, y: 0, largeur: 0, hauteur: 0 });
    let mut memoire = Memoire::Attachee;
    memoire = dma::apres_autre(memoire, dma::issue(eteint));
    rend_si_sur(&mut p.file, RESSOURCE_BUREAU, s.physique, s.octets, &mut memoire, "bureau-precedent");
    p.scanout = None;
    crate::serial_println!(
        "VIRTIO_GPU_SCANOUT_RENDU {}x{} ressource={} memoire={:?}",
        s.largeur, s.hauteur, RESSOURCE_BUREAU, memoire
    );
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
    let Some(mut garde) = verrou() else {
        // Contexte ou l'on ne peut pas attendre, verrou tenu ailleurs : la
        // presentation est sautee et COMPTEE, pas tenue pour un echec.
        PRESENTS_SAUTES.fetch_add(1, Ordering::Relaxed);
        return true;
    };
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
    // BOUCHAUD_VIRTIO_GPU_TAMPONS_V1 (P10+.2b) : pendant le transfert, l'hote
    // lit la memoire du scanout -- le bureau qui y compose directement ne doit
    // pas y ecrire (`tampon_ecrivable`). Sans reponse : plus jamais ecrite.
    let Some(en_vol) = tampons::soumet(etat_tampon()) else {
        PRESENTS_ECHEC.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    pose_etat_tampon(en_vol);
    // BOUCHAUD_VIRTIO_GPU_BARRIERE_V1 : le flush ne part qu'apres la reponse
    // -- barriere comprise, donc traitement FINI -- du transfert. Rien dans
    // la specification ne garantit qu'une file de controle soit traitee dans
    // l'ordre : les deux commandes sous une sonnette (BOUCHAUD_VIRTIO_GPU_LOT_V1)
    // pouvaient afficher le rectangle avant de l'avoir recu.
    let rendu = unsafe { transfert(&mut p.file, RESSOURCE_BUREAU, r, decalage) };
    let fin = if dma::issue(rendu) == dma::Issue::SansReponse { tampons::Fin::SansReponse } else { tampons::Fin::Achevee };
    pose_etat_tampon(tampons::acheve(en_vol, fin));
    let ok = rendu == Ok(RESP_OK_NODATA) && unsafe { vide(&mut p.file, RESSOURCE_BUREAU, r) } == Ok(RESP_OK_NODATA);
    if ok {
        PRESENTS_OK.fetch_add(1, Ordering::Relaxed);
        PIXELS_TRANSFERES.fetch_add(u64::from(r.largeur) * u64::from(r.hauteur), Ordering::Relaxed);
        ECHECS_CONSECUTIFS.store(0, Ordering::Relaxed);
    } else {
        PRESENTS_ECHEC.fetch_add(1, Ordering::Relaxed);
        let n = ECHECS_CONSECUTIFS.fetch_add(1, Ordering::Relaxed) + 1;
        // Une commande sans reponse casse la file : inutile d'attendre encore
        // sept fois une seconde, verrou tenu, pour le constater.
        if p.file.cassee {
            ACTIF.store(false, Ordering::Release);
            crate::serial_println!("VIRTIO_GPU_SCANOUT_ABANDON raison=file-sans-reponse echecs_consecutifs={}", n);
        } else if n >= ECHECS_AVANT_ABANDON {
            ACTIF.store(false, Ordering::Release);
            crate::serial_println!("VIRTIO_GPU_SCANOUT_ABANDON raison=refus echecs_consecutifs={}", n);
        }
    }
    NS_PRESENT.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(debut), Ordering::Relaxed);
    let _ = s.physique;
    let _ = s.octets;
    ok
}

static ECHECS_CONSECUTIFS: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_VIRTIO_GPU_TRAME_V1 (P10+.3b) -- une trame, une soumission.
static TRAMES_SOUMISES: AtomicU64 = AtomicU64::new(0);
static COMMANDES_TRAMES: AtomicU64 = AtomicU64::new(0);

/// Presente une trame entiere : les transferts du plan
/// (`virtio_gpu_trame::planifie`), chacun apres la reponse du precedent, puis
/// UN flush sur l'union de la trame, apres la reponse du dernier transfert.
/// Meme tenue de la memoire (`Transfert` autour de chaque transfert), meme
/// jugement d'echec et d'abandon que `presente`.
pub fn presente_trame(plan: &trame::Plan) -> bool {
    if !actif() {
        return false;
    }
    if plan.nb == 0 {
        return true;
    }
    let debut = crate::kernel::timer::monotonic_ns();
    let Some(mut garde) = verrou() else {
        PRESENTS_SAUTES.fetch_add(1, Ordering::Relaxed);
        return true;
    };
    let Some(p) = garde.as_mut() else { return false };
    let Some(s) = p.scanout else { return false };
    let rogne = |r: tampons::Rect| -> Option<Rect> {
        let x1 = (r.x + r.largeur).min(s.largeur);
        let y1 = (r.y + r.hauteur).min(s.hauteur);
        (r.x < x1 && r.y < y1).then_some(Rect { x: r.x, y: r.y, largeur: x1 - r.x, hauteur: y1 - r.y })
    };
    let mut ok = true;
    let mut pixels = 0u64;
    let mut commandes = 0u64;
    for t in plan.transferts() {
        let Some(r) = rogne(*t) else { continue };
        let decalage = ((r.y as usize * s.largeur as usize + r.x as usize) * 4) as u64;
        let Some(en_vol) = tampons::soumet(etat_tampon()) else {
            ok = false;
            break;
        };
        pose_etat_tampon(en_vol);
        let rendu = unsafe { transfert(&mut p.file, RESSOURCE_BUREAU, r, decalage) };
        let fin = if dma::issue(rendu) == dma::Issue::SansReponse { tampons::Fin::SansReponse } else { tampons::Fin::Achevee };
        pose_etat_tampon(tampons::acheve(en_vol, fin));
        commandes += 1;
        if rendu != Ok(RESP_OK_NODATA) {
            ok = false;
            break;
        }
        pixels += u64::from(r.largeur) * u64::from(r.hauteur);
    }
    // Le flush, une fois, apres la reponse du DERNIER transfert.
    if ok {
        if let Some(f) = rogne(plan.flush) {
            ok = unsafe { vide(&mut p.file, RESSOURCE_BUREAU, f) } == Ok(RESP_OK_NODATA);
            commandes += 1;
        }
    }
    TRAMES_SOUMISES.fetch_add(1, Ordering::Relaxed);
    COMMANDES_TRAMES.fetch_add(commandes, Ordering::Relaxed);
    if ok {
        PRESENTS_OK.fetch_add(1, Ordering::Relaxed);
        PIXELS_TRANSFERES.fetch_add(pixels, Ordering::Relaxed);
        ECHECS_CONSECUTIFS.store(0, Ordering::Relaxed);
    } else {
        PRESENTS_ECHEC.fetch_add(1, Ordering::Relaxed);
        let n = ECHECS_CONSECUTIFS.fetch_add(1, Ordering::Relaxed) + 1;
        if p.file.cassee {
            ACTIF.store(false, Ordering::Release);
            crate::serial_println!("VIRTIO_GPU_SCANOUT_ABANDON raison=file-sans-reponse echecs_consecutifs={}", n);
        } else if n >= ECHECS_AVANT_ABANDON {
            ACTIF.store(false, Ordering::Release);
            crate::serial_println!("VIRTIO_GPU_SCANOUT_ABANDON raison=refus echecs_consecutifs={}", n);
        }
    }
    NS_PRESENT.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(debut), Ordering::Relaxed);
    ok
}

/// `(trames soumises d'un bloc, commandes envoyees pour elles)`.
pub fn compteurs_trame() -> (u64, u64) {
    (TRAMES_SOUMISES.load(Ordering::Relaxed), COMMANDES_TRAMES.load(Ordering::Relaxed))
}

// BOUCHAUD_VIRTIO_GPU_TAMPONS_V1 (P10+.2b) : l'etat de la memoire du scanout
// (`virtio_gpu_tampons::Etat`), ecrit sous le verrou du peripherique par
// `presente`, lu sans verrou par le bureau avant d'y ecrire.
static ETAT_TAMPON: AtomicU8 = AtomicU8::new(0);

fn etat_tampon() -> tampons::Etat {
    match ETAT_TAMPON.load(Ordering::Acquire) {
        0 => tampons::Etat::Composition,
        1 => tampons::Etat::Transfert,
        _ => tampons::Etat::Inconnu,
    }
}

fn pose_etat_tampon(e: tampons::Etat) {
    let v = match e {
        tampons::Etat::Composition => 0,
        tampons::Etat::Transfert => 1,
        tampons::Etat::Inconnu => 2,
    };
    ETAT_TAMPON.store(v, Ordering::Release);
}

/// Le bureau peut-il ecrire la memoire du scanout (aucun transfert en vol,
/// aucun transfert reste sans reponse) ?
pub fn tampon_ecrivable() -> bool {
    tampons::peut_ecrire(etat_tampon())
}

/// L'etat de la memoire du scanout, pour les releves.
pub fn etat_tampon_nom() -> &'static str {
    match etat_tampon() {
        tampons::Etat::Composition => "composition",
        tampons::Etat::Transfert => "transfert",
        tampons::Etat::Inconnu => "inconnu",
    }
}

/// `(presentations sautees faute de verrou, attentes ayant cede le
/// processeur, attentes faites la ou ceder est interdit)`.
pub fn compteurs_verrou() -> (u64, u64, u64) {
    (
        PRESENTS_SAUTES.load(Ordering::Relaxed),
        ATTENTES_CEDEES.load(Ordering::Relaxed),
        ATTENTES_NON_CEDABLES.load(Ordering::Relaxed),
    )
}

/// `(presentations_ok, echecs, pixels_transferes, ns_total)`.
pub fn compteurs() -> (u64, u64, u64, u64) {
    (
        PRESENTS_OK.load(Ordering::Relaxed),
        PRESENTS_ECHEC.load(Ordering::Relaxed),
        PIXELS_TRANSFERES.load(Ordering::Relaxed),
        NS_PRESENT.load(Ordering::Relaxed),
    )
}
