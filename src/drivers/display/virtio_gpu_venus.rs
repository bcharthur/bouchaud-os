//! Banc Venus. La fenêtre PCI n'est accessible qu'au noyau. Un ioctl ne
//! choisit ni le blob, ni le décalage, ni les commandes de la soumission.
use super::*;
use alloc::sync::{Arc, Weak};

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

pub(super) enum Canal {
    Nouveau(Fenetre),
    Ouvert(Fenetre, Option<u32>, Weak<()>),
}

pub fn version_vulkan_banc(session: &Arc<()>, octets: u32) -> Result<u32, ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let canal = p.hostmem_venus.as_ref().ok_or(ErreurRendu::Indisponible)?;
    let fenetre = match canal {
        Canal::Nouveau(fenetre) | Canal::Ouvert(fenetre, _, _) => *fenetre,
    };
    let taille = u64::from(octets);
    if taille < PAGE as u64 || taille % PAGE as u64 != 0
        || taille > fenetre.octets.min(MAX_BLOB_REPONSE)
    {
        return Err(ErreurRendu::Invalide);
    }
    let proprietaire = Arc::downgrade(session);
    let (existe, precedent) = prepare_canal(p, &proprietaire)?;
    if precedent.is_some_and(|existant| existant != octets) {
        return Err(ErreurRendu::Invalide);
    }

    // Une issue incertaine retire le canal jusqu'au reset du périphérique.
    p.hostmem_venus = None;
    if precedent.is_none() {
        let mut restaure = false;
        if let Err(etape) = unsafe { ouvre(&mut p.file, octets, existe, &mut restaure) } {
            if restaure { p.hostmem_venus = Some(Canal::Nouveau(fenetre)); }
            crate::serial_println!("P10_VENUS_OUVERTURE_ECHEC etape={} restaure={}", etape, restaure);
            return Err(ErreurRendu::Commande);
        }
    }
    match unsafe { lit_version(&mut p.file, fenetre.adresse) } {
        Ok(version) => {
            p.hostmem_venus = Some(Canal::Ouvert(fenetre, Some(octets), proprietaire));
            Ok(version)
        }
        Err(etape) => {
            crate::serial_println!("P10_VENUS_VERSION_ECHEC etape={}", etape);
            Err(ErreurRendu::Commande)
        }
    }
}

/// Revient à Nouveau si le précédent propriétaire est mort et tous ses
/// objets ont été libérés. Aucun handle n'est prêté avant les accusés.
fn prepare_canal(p: &mut Peripherique, proprietaire: &Weak<()>) -> Result<(bool, Option<u32>), ErreurRendu> {
    let canal = p.hostmem_venus.as_ref().ok_or(ErreurRendu::Indisponible)?;
    let Canal::Ouvert(fenetre, blob, ancien) = canal else { return Ok((false, None)); };
    if ancien.ptr_eq(proprietaire) { return Ok((true, *blob)); }
    if ancien.strong_count() != 0 { return Err(ErreurRendu::Occupe); }
    let (fenetre, blob) = (*fenetre, *blob);
    p.hostmem_venus = None;
    if let Err(etape) = unsafe { ferme_objets(&mut p.file, blob.is_some()) } {
        crate::serial_println!("P10_VENUS_ORPHELIN_ECHEC etape={}", etape);
        return Err(ErreurRendu::Commande);
    }
    p.hostmem_venus = Some(Canal::Nouveau(fenetre));
    crate::serial_println!("P10_VENUS_ORPHELIN_RECUPERE");
    Ok((false, None))
}

pub fn initialise_contexte_venus(session: &Arc<()>) -> Result<(), ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let proprietaire = Arc::downgrade(session);
    if prepare_canal(p, &proprietaire)?.0 { return Err(ErreurRendu::Existe); }
    let Some(Canal::Nouveau(fenetre)) = p.hostmem_venus.take() else {
        return Err(ErreurRendu::Indisponible);
    };
    let (q, n) = gpu3d::contexte_venus(true);
    match unsafe { p.file.commande(&q[..n], ENTETE) } {
        Ok(RESP_OK_NODATA) => {
            p.hostmem_venus = Some(Canal::Ouvert(fenetre, None, proprietaire));
            Ok(())
        }
        Ok(_) => {
            p.hostmem_venus = Some(Canal::Nouveau(fenetre));
            Err(ErreurRendu::Commande)
        }
        Err(_) => Err(ErreurRendu::Commande),
    }
}

pub fn contexte_venus_disponible() -> bool {
    verrou().is_some_and(|garde| garde.as_ref().is_some_and(|p| !p.file.cassee && p.hostmem_venus.is_some()))
}

pub fn ferme_vulkan_banc(session: &Arc<()>) -> Result<(), ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    if let Some(Canal::Ouvert(_, _, ancien)) = &p.hostmem_venus {
        if !ancien.ptr_eq(&Arc::downgrade(session)) { return Err(ErreurRendu::Occupe); }
    }
    let canal = p.hostmem_venus.take().ok_or(ErreurRendu::Indisponible)?;
    let fenetre = match canal {
        Canal::Nouveau(fenetre) => fenetre,
        Canal::Ouvert(fenetre, blob, _) => match unsafe { ferme_objets(&mut p.file, blob.is_some()) } {
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

unsafe fn ouvre(f: &mut FileControle, octets: u32, contexte_existe: bool, restaure: &mut bool) -> Result<(), &'static str> {
    if !contexte_existe {
        let (q, n) = gpu3d::contexte_venus(true);
        if f.commande(&q[..n], ENTETE)? != RESP_OK_NODATA {
            *restaure = true;
            return Err("ctx-create-refuse");
        }
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

pub const OCTETS_REPONSE_VENUS: usize = 512;

fn aperture_possedee(p: &Peripherique, session: &Arc<()>) -> Result<usize, ErreurRendu> {
    match p.hostmem_venus.as_ref() {
        Some(Canal::Ouvert(fenetre, Some(_), proprietaire)) => {
            if proprietaire.ptr_eq(&Arc::downgrade(session)) {
                Ok(fenetre.adresse)
            } else {
                Err(ErreurRendu::Occupe)
            }
        }
        _ => Err(ErreurRendu::Invalide),
    }
}

pub fn soumets_venus(session: &Arc<()>, flux: &[u8]) -> Result<(), ErreurRendu> {
    let entete = gpu3d::entete_soumission_venus(flux.len()).ok_or(ErreurRendu::Invalide)?;
    let mut q = alloc::vec::Vec::new();
    q.try_reserve_exact(entete.len() + flux.len()).map_err(|_| ErreurRendu::Plein)?;
    q.extend_from_slice(&entete);
    q.extend_from_slice(flux);
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let mem = aperture_possedee(p, session)? as *mut u32;
    unsafe {
        for i in 0..OCTETS_REPONSE_VENUS / 4 { write_volatile(mem.add(i), u32::MAX); }
        fence(Ordering::SeqCst);
        if p.file.commande(&q, ENTETE) == Ok(RESP_OK_NODATA)
            && attente_reponse(250, || read_volatile(mem) != u32::MAX)
        {
            return Ok(());
        }
    }
    // Impossible de prouver qu'un flux incomplet a cessé d'écrire.
    p.hostmem_venus = None;
    Err(ErreurRendu::Commande)
}

pub fn copie_reponse_venus(session: &Arc<()>) -> Result<[u8; OCTETS_REPONSE_VENUS], ErreurRendu> {
    let garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_ref().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let mem = aperture_possedee(p, session)? as *const u8;
    let mut octets = [0; OCTETS_REPONSE_VENUS];
    fence(Ordering::SeqCst);
    for (i, octet) in octets.iter_mut().enumerate() {
        *octet = unsafe { read_volatile(mem.add(i)) };
    }
    Ok(octets)
}

unsafe fn ferme_objets(f: &mut FileControle, blob: bool) -> Result<(), &'static str> {
    if !blob { return ferme_contexte(f, false); }
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
