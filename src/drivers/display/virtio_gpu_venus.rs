//! Banc Venus. La fenêtre PCI n'est accessible qu'au noyau. Un ioctl ne
//! choisit ni le blob, ni le décalage, ni les commandes de la soumission.
use super::*;

const CFG_SHARED_MEMORY: u8 = 8;
const SHM_ID_HOST_VISIBLE: u8 = 1;
const RESP_OK_MAP_INFO: u32 = 0x1106;

pub(super) fn mappe_fenetre(dev: &PciDevice) -> Option<usize> {
    let mut liste = [pci::Capacite { identifiant: 0, decalage: 0 }; 16];
    let n = pci::capacites_de(dev, &mut liste);
    for cap in &liste[..n] {
        let o = cap.decalage;
        if cap.identifiant != CAP_VENDEUR || o > 232 { continue; }
        let entete = pci::config_lit32(dev, o);
        if (entete >> 24) as u8 != CFG_SHARED_MEMORY || ((entete >> 16) as u8) < 24 {
            continue;
        }
        let identifiant = pci::config_lit32(dev, o + 4);
        if (identifiant >> 8) as u8 != SHM_ID_HOST_VISIBLE { continue; }
        let bar = match pci::bar_decode(dev, identifiant as u8) {
            Bar::Memoire32 { adresse, .. } => u64::from(adresse),
            Bar::Memoire64 { adresse, .. } => adresse,
            _ => return None,
        };
        let lit64 = |bas, haut| u64::from(pci::config_lit32(dev, o + bas))
            | (u64::from(pci::config_lit32(dev, o + haut)) << 32);
        let offset = lit64(8, 16);
        let taille = lit64(12, 20);
        let base = bar.checked_add(offset)?;
        base.checked_add(taille)?;
        if bar == 0 || base % PAGE as u64 != 0 || taille < PAGE as u64 {
            return None;
        }
        let v = memory::mappe_mmio(base, PAGE as u64).ok()?;
        crate::serial_println!("P10_VENUS_HOSTMEM_OK octets={} page={}", taille, PAGE);
        return Some(v as usize);
    }
    None
}

pub(super) enum Canal {
    Nouveau(usize),
    Mappe(usize),
}

pub fn version_vulkan_banc() -> Result<u32, ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    // Une seule page de transport par périphérique, sans accès utilisateur.
    // Les requêtes partagent la fenêtre sous le verrou de la controlq.
    // Si une commande échoue, elle ne sera jamais réutilisée dans ce démarrage.
    let canal = p.hostmem_venus.take().ok_or(ErreurRendu::Indisponible)?;
    let resultat = unsafe {
        let aperture = match canal {
            Canal::Nouveau(aperture) => ouvre(&mut p.file).map(|()| aperture),
            Canal::Mappe(aperture) => Ok(aperture),
        };
        aperture.and_then(|v| lit_version(&mut p.file, v).map(|version| (v, version)))
    };
    match resultat {
        Ok((aperture, version)) => {
            p.hostmem_venus = Some(Canal::Mappe(aperture));
            Ok(version)
        }
        Err(etape) => {
            crate::serial_println!("P10_VENUS_VERSION_ECHEC etape={}", etape);
            Err(ErreurRendu::Commande)
        }
    }
}

pub fn ferme_vulkan_banc() -> Result<(), ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let Some(canal) = p.hostmem_venus.take() else { return Err(ErreurRendu::Indisponible); };
    let aperture = match canal {
        Canal::Nouveau(aperture) => aperture,
        Canal::Mappe(aperture) => match unsafe { ferme(&mut p.file) } {
            Ok(()) => aperture,
            Err(etape) => {
                crate::serial_println!("P10_VENUS_DEMAPPE_ECHEC etape={}", etape);
                return Err(ErreurRendu::Commande);
            }
        },
    };
    p.hostmem_venus = Some(Canal::Nouveau(aperture));
    Ok(())
}

unsafe fn ouvre(f: &mut FileControle) -> Result<(), &'static str> {
    let (q, n) = gpu3d::contexte_venus(true);
    if f.commande(&q[..n], ENTETE) != Ok(RESP_OK_NODATA) { return Err("ctx-create"); }
    if f.commande(&gpu3d::blob_reponse_venus(PAGE as u32), ENTETE) != Ok(RESP_OK_NODATA) {
        // La ressource n'a pas été mappée. Essayer de fermer le contexte
        // seulement si la file peut toujours recevoir une commande.
        if !f.cassee {
            let (q, n) = gpu3d::contexte_venus(false);
            let _ = f.commande(&q[..n], ENTETE);
        }
        return Err("blob-host3d");
    }
    let (q, n) = gpu3d::mappe_reponse_venus(true);
    if f.commande(&q[..n], 32) != Ok(RESP_OK_MAP_INFO) { return Err("map-blob"); }
    if f.derniere_reponse_len < 32 { return Err("map-info-tronquee"); }
    Ok(())
}

unsafe fn lit_version(f: &mut FileControle, aperture: usize) -> Result<u32, &'static str> {
    let mut reponse = [0u8; 20];
    let mem = aperture as *mut u8;
    for i in 0..reponse.len() {
        write_volatile(mem.add(i), 0xff);
    }
    fence(Ordering::SeqCst);
    if f.commande(&gpu3d::soumet_version_venus(), ENTETE) != Ok(RESP_OK_NODATA) {
        return Err("submit-venus");
    }
    // Certaines versions de QEMU clôturent le transport avant la réponse
    // du render server. Vérifier la publication et le contenu du flux.
    if !attente_reponse(250, || read_volatile(mem.add(16).cast::<u32>()) != u32::MAX) {
        return Err("reponse-vulkan-absente");
    }
    fence(Ordering::SeqCst);
    for (i, b) in reponse.iter_mut().enumerate() {
        *b = read_volatile(mem.add(i));
    }
    gpu3d::lit_version_venus(&reponse).ok_or("reponse-vulkan")
}

unsafe fn ferme(f: &mut FileControle) -> Result<(), &'static str> {
    let (q, n) = gpu3d::mappe_reponse_venus(false);
    if f.commande(&q[..n], ENTETE) != Ok(RESP_OK_NODATA) { return Err("unmap-blob"); }
    let (q, n) = gpu3d::contexte_venus(false);
    if f.commande(&q[..n], ENTETE) != Ok(RESP_OK_NODATA) { return Err("ctx-destroy"); }
    if libere(f, gpu3d::RESSOURCE_REPONSE_VENUS) != Ok(RESP_OK_NODATA) {
        return Err("blob-unref");
    }
    Ok(())
}
