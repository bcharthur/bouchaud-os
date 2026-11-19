//! Banc Venus. La fenêtre PCI n'est accessible qu'au noyau. Un ioctl ne
//! choisit ni le blob, ni le décalage, ni les commandes de la soumission.
use super::*;

const CFG_SHARED_MEMORY: u8 = 8;
const SHM_ID_HOST_VISIBLE: u8 = 1;
const RESP_OK_MAP_INFO: u32 = 0x1106;

// Le blob de réponse est distinct des allocations Vulkan VkDeviceMemory.
const MAX_BLOB_REPONSE: u64 = 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct Fenetre {
    adresse: usize,
    octets: u64,
}

pub(super) fn mappe_fenetre(dev: &PciDevice) -> Option<Fenetre> {
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
        return Some(Fenetre { adresse: v as usize, octets: taille });
    }
    None
}

#[derive(Clone, Copy)]
pub(super) enum Canal {
    Nouveau(Fenetre),
    Mappe(Fenetre, u32),
}

pub fn version_vulkan_banc(octets: u32) -> Result<u32, ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let canal = p.hostmem_venus.ok_or(ErreurRendu::Indisponible)?;
    let fenetre = match canal {
        Canal::Nouveau(fenetre) | Canal::Mappe(fenetre, _) => fenetre,
    };
    let taille = u64::from(octets);
    if taille < PAGE as u64 || taille % PAGE as u64 != 0
        || taille > fenetre.octets.min(MAX_BLOB_REPONSE)
        || matches!(canal, Canal::Mappe(_, existants) if existants != octets)
    {
        return Err(ErreurRendu::Invalide);
    }

    // Une seule ressource de transport et un seul intervalle hostmem.
    // Une issue incertaine retire le canal jusqu'au reset du périphérique.
    p.hostmem_venus = None;
    if matches!(canal, Canal::Nouveau(_)) {
        let mut restaure = false;
        if let Err(etape) = unsafe { ouvre(&mut p.file, octets, &mut restaure) } {
            if restaure { p.hostmem_venus = Some(canal); }
            crate::serial_println!("P10_VENUS_OUVERTURE_ECHEC etape={} restaure={}", etape, restaure);
            return Err(ErreurRendu::Commande);
        }
    }
    match unsafe { lit_version(&mut p.file, fenetre.adresse) } {
        Ok(version) => {
            p.hostmem_venus = Some(Canal::Mappe(fenetre, octets));
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
    let canal = p.hostmem_venus.take().ok_or(ErreurRendu::Indisponible)?;
    let fenetre = match canal {
        Canal::Nouveau(fenetre) => fenetre,
        Canal::Mappe(fenetre, _) => match unsafe { ferme(&mut p.file) } {
            Ok(()) => fenetre,
            Err(etape) => {
                crate::serial_println!("P10_VENUS_DEMAPPE_ECHEC etape={}", etape);
                return Err(ErreurRendu::Commande);
            }
        },
    };
    p.hostmem_venus = Some(Canal::Nouveau(fenetre));
    Ok(())
}

unsafe fn ouvre(f: &mut FileControle, octets: u32, restaure: &mut bool) -> Result<(), &'static str> {
    let (q, n) = gpu3d::contexte_venus(true);
    let contexte = f.commande(&q[..n], ENTETE)?;
    if contexte != RESP_OK_NODATA {
        *restaure = true;
        return Err("ctx-create-refuse");
    }
    let blob = f.commande(&gpu3d::blob_reponse_venus(octets), ENTETE)?;
    if blob != RESP_OK_NODATA {
        *restaure = ferme_contexte(f, false).is_ok();
        return Err("blob-host3d-refuse");
    }
    let (q, n) = gpu3d::mappe_reponse_venus(true);
    let carte = f.commande(&q[..n], 32)?;
    if carte != RESP_OK_MAP_INFO {
        // Refus confirmé : il n'y a aucun mappage à retirer.
        *restaure = ferme_contexte(f, true).is_ok();
        return Err("map-blob-refuse");
    }
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
    let debut = crate::kernel::timer::monotonic_ns();
    let reponse = f.commande(&q[..n], ENTETE);
    crate::serial_println!("P10_VENUS_UNMAP reponse={:?} us={}", reponse,
        crate::kernel::timer::monotonic_ns().saturating_sub(debut) / 1000);
    if reponse? != RESP_OK_NODATA { return Err("unmap-blob-refuse"); }
    ferme_contexte(f, true)
}

unsafe fn ferme_contexte(f: &mut FileControle, blob: bool) -> Result<(), &'static str> {
    let (q, n) = gpu3d::contexte_venus(false);
    if f.commande(&q[..n], ENTETE) != Ok(RESP_OK_NODATA) { return Err("ctx-destroy"); }
    if blob && libere(f, gpu3d::RESSOURCE_REPONSE_VENUS) != Ok(RESP_OK_NODATA) {
        return Err("blob-unref");
    }
    Ok(())
}
