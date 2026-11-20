//! Banc Venus. La fenêtre PCI n'est accessible qu'au noyau. Un ioctl ne
//! choisit ni le blob, ni le décalage, ni les commandes de la soumission.
use super::*;
use alloc::sync::{Arc, Weak};

const CFG_SHARED_MEMORY: u8 = 8;
const SHM_ID_HOST_VISIBLE: u8 = 1;
const RESP_OK_MAP_INFO: u32 = 0x1106;

// Le blob de réponse est distinct des allocations Vulkan VkDeviceMemory.
const MAX_BLOB_REPONSE: u64 = 1024 * 1024;
const BASE_GEMS: u64 = 2 * MAX_BLOB_REPONSE;
const MAX_GEM_VENUS: u64 = 16 * 1024 * 1024;
const SECOND_CONTEXTE: u32 = 0xB109;

pub(super) struct SecondContexte {
    proprietaire: Weak<()>,
    anneaux: u8,
}

#[derive(Clone, Copy)]
pub(super) struct Fenetre {
    adresse: usize,
    octets: u64,
    physique: u64,
    relecture: Option<usize>,
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
        // Le blob de relecture ne recouvre jamais celui des réponses.
        let relecture = if taille >= MAX_BLOB_REPONSE + PAGE as u64 {
            memory::mappe_mmio(base + MAX_BLOB_REPONSE, PAGE as u64).ok().map(|ptr| ptr as usize)
        } else {
            None
        };
        return Some(Fenetre { adresse: v as usize, octets: taille, physique: base, relecture });
    }
    None
}

impl Fenetre {
    fn octets_par_gem(self) -> u64 {
        let possible = self.octets.saturating_sub(BASE_GEMS) / gpu3d::GEMS_VENUS_PRIMAIRE as u64;
        possible.min(MAX_GEM_VENUS) & !(PAGE as u64 - 1)
    }

    fn permet_second(self) -> bool {
        self.octets >= BASE_GEMS + self.octets_par_gem() * gpu3d::NOMBRE_GEMS_VENUS as u64
    }
}

pub(super) enum Canal {
    Nouveau(Fenetre),
    Ouvert(Fenetre, Option<u32>, Weak<()>),
}

#[derive(Clone, Copy)]
pub struct Gem {
    pub handle: u32,
    pub ressource: u32,
    pub octets: u32,
}

// Le bail garde l'ouverture du contexte vivante tant qu'un espace
// d'adressage (ou une retraite TLB) peut atteindre le blob.
pub struct BailGem {
    _session: Arc<()>,
}

struct ObjetGem {
    gem: Gem,
    contexte: u32,
    ferme: bool,
    mappe: bool,
    bail: Weak<BailGem>,
}

pub(super) struct Gems {
    objets: [Option<ObjetGem>; gpu3d::NOMBRE_GEMS_VENUS],
    prochain: u32,
}

impl Gems {
    pub(super) const fn neuf() -> Self {
        Self { objets: [const { None }; gpu3d::NOMBRE_GEMS_VENUS], prochain: 1 }
    }

    fn cherche(&self, contexte: u32, handle: u32) -> Result<usize, ErreurRendu> {
        self.objets.iter().position(|objet| objet.as_ref().is_some_and(|o| !o.ferme && o.contexte == contexte && o.gem.handle == handle))
            .ok_or(ErreurRendu::Inconnue)
    }

    unsafe fn retire(&mut self, file: &mut FileControle, i: usize) -> Result<(), &'static str> {
        if let Some(objet) = &mut self.objets[i] {
            if objet.bail.strong_count() != 0 { return Err("gem-bail-actif"); }
            if objet.mappe {
                let (reponse, n) = gpu3d::mappe_blob_venus(objet.gem.ressource, false, 0);
                if file.commande(&reponse[..n], ENTETE)? != RESP_OK_NODATA {
                    return Err("gem-unmap");
                }
                objet.mappe = false;
            }
            if libere(file, objet.gem.ressource)? != RESP_OK_NODATA {
                return Err("gem-unref");
            }
            self.objets[i] = None;
        }
        Ok(())
    }

    fn baux_actifs(&self, contexte: u32) -> bool {
        self.objets.iter().flatten().any(|objet| objet.contexte == contexte && objet.bail.strong_count() != 0)
    }

    unsafe fn ferme(&mut self, file: &mut FileControle, contexte: u32) -> Result<(), &'static str> {
        for i in 0..self.objets.len() {
            if self.objets[i].as_ref().is_some_and(|objet| objet.contexte == contexte) {
                self.retire(file, i)?;
            }
        }
        Ok(())
    }
}

fn possede_contexte(p: &Peripherique, session: &Arc<()>) -> Result<(Fenetre, u32, u8, bool), ErreurRendu> {
    let cherche = Arc::downgrade(session);
    let fenetre = match &p.hostmem_venus {
        Some(Canal::Ouvert(fenetre, blob, proprietaire)) => {
            if proprietaire.ptr_eq(&cherche) {
                return Ok((*fenetre, gpu3d::CONTEXTE_VENUS, p.anneaux_venus, blob.is_some()));
            }
            *fenetre
        }
        Some(Canal::Nouveau(fenetre)) => *fenetre,
        None => return Err(ErreurRendu::Indisponible),
    };
    if let Some(contexte) = &p.second_venus {
        if contexte.proprietaire.ptr_eq(&cherche) {
            return Ok((fenetre, SECOND_CONTEXTE, contexte.anneaux, false));
        }
    }
    if matches!(p.hostmem_venus, Some(Canal::Ouvert(..))) || p.second_venus.is_some() {
        return Err(ErreurRendu::Occupe);
    }
    Err(ErreurRendu::Invalide)
}

pub fn cree_gem_venus(session: &Arc<()>, id_memoire: u64, octets: u64) -> Result<Gem, ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let (fenetre, contexte, _, _) = possede_contexte(p, session)?;
    if octets < PAGE as u64 || octets % PAGE as u64 != 0 || octets > fenetre.octets_par_gem() {
        return Err(ErreurRendu::Invalide);
    }
    let gems = &mut p.gems_venus;
    for i in 0..gems.objets.len() {
        if gems.objets[i].as_ref().is_some_and(|o| o.ferme && o.bail.strong_count() == 0)
            && unsafe { gems.retire(&mut p.file, i) }.is_err()
        {
            p.hostmem_venus = None;
            return Err(ErreurRendu::Commande);
        }
    }
    // Les deux pools matériels ne se recouvrent pas, même si un client
    // utilise tous ses slots. Le slot du canal principal conserve sa taille.
    let plage = if contexte == SECOND_CONTEXTE {
        gpu3d::GEMS_VENUS_PRIMAIRE..gpu3d::NOMBRE_GEMS_VENUS
    } else {
        0..gpu3d::GEMS_VENUS_PRIMAIRE
    };
    let i = plage.into_iter().find(|&i| gems.objets[i].is_none()).ok_or(ErreurRendu::Plein)?;
    // Le slot matériel est recyclé après UNREF. Le handle visible ne l'est pas.
    let gem = Gem { handle: gems.prochain, ressource: gpu3d::PREMIER_GEM_VENUS + i as u32, octets: octets as u32 };
    gems.prochain = gem.handle.checked_add(1).ok_or(ErreurRendu::Plein)?;
    let mut requete = gpu3d::blob_memoire_venus(gem.ressource, id_memoire, gem.octets);
    requete[16..20].copy_from_slice(&contexte.to_le_bytes());
    match unsafe { p.file.commande(&requete, ENTETE) } {
        Ok(RESP_OK_NODATA) => {
            gems.objets[i] = Some(ObjetGem { gem, contexte, ferme: false, mappe: false, bail: Weak::new() });
            Ok(gem)
        }
        Ok(_) => Err(ErreurRendu::Commande),
        Err(_) => {
            gems.objets[i] = Some(ObjetGem { gem, contexte, ferme: false, mappe: false, bail: Weak::new() });
            p.hostmem_venus = None;
            Err(ErreurRendu::Commande)
        }
    }
}

pub fn info_gem_venus(session: &Arc<()>, handle: u32) -> Result<Gem, ErreurRendu> {
    let garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_ref().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let (_, contexte, _, _) = possede_contexte(p, session)?;
    p.gems_venus.objets[p.gems_venus.cherche(contexte, handle)?].as_ref().map(|o| o.gem).ok_or(ErreurRendu::Inconnue)
}

pub fn ferme_gem_venus(session: &Arc<()>, handle: u32) -> Result<(), ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let (_, contexte, _, _) = possede_contexte(p, session)?;
    let i = p.gems_venus.cherche(contexte, handle)?;
    let objet = p.gems_venus.objets[i].as_mut().ok_or(ErreurRendu::Inconnue)?;
    objet.ferme = true;
    if objet.bail.strong_count() != 0 { return Ok(()); }
    if unsafe { p.gems_venus.retire(&mut p.file, i) }.is_err() {
        p.hostmem_venus = None;
        return Err(ErreurRendu::Commande);
    }
    Ok(())
}

pub fn offset_gem_venus(session: &Arc<()>, handle: u32) -> Result<u64, ErreurRendu> {
    info_gem_venus(session, handle).map(|gem| u64::from(gem.handle) * MAX_GEM_VENUS)
}

pub fn mappe_gem_venus(session: &Arc<()>, offset: u64, taille: u64) -> Result<(u64, Arc<BailGem>), ErreurRendu> {
    if offset % MAX_GEM_VENUS != 0 { return Err(ErreurRendu::Invalide); }
    let handle = u32::try_from(offset / MAX_GEM_VENUS).map_err(|_| ErreurRendu::Invalide)?;
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let (fenetre, contexte, _, _) = possede_contexte(p, session)?;
    let fente = fenetre.octets_par_gem();
    if fente == 0 { return Err(ErreurRendu::Indisponible); }
    let i = p.gems_venus.cherche(contexte, handle)?;
    let objet = p.gems_venus.objets[i].as_mut().ok_or(ErreurRendu::Inconnue)?;
    if taille > u64::from(objet.gem.octets) { return Err(ErreurRendu::Invalide); }
    let offset_bar = BASE_GEMS + i as u64 * fente;
    let physique = fenetre.physique + offset_bar;
    if !objet.mappe {
        // L'alias noyau sert aux copies user_read/user_write. Seules
        // les pages du blob demandé sont installées, jamais toute la BAR.
        memory::mappe_mmio(physique, objet.gem.octets.into()).map_err(|_| ErreurRendu::Plein)?;
        let (q, n) = gpu3d::mappe_blob_venus(objet.gem.ressource, true, offset_bar);
        match unsafe { p.file.commande(&q[..n], 32) } {
            Ok(RESP_OK_MAP_INFO) => objet.mappe = true,
            Ok(_) => return Err(ErreurRendu::Commande),
            Err(_) => {
                p.hostmem_venus = None;
                return Err(ErreurRendu::Commande);
            }
        }
    }
    let bail = objet.bail.upgrade().unwrap_or_else(|| {
        let bail = Arc::new(BailGem { _session: session.clone() });
        objet.bail = Arc::downgrade(&bail);
        bail
    });
    Ok((physique, bail))
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
    recupere_second(p)?;
    if p.second_venus.is_some() { return Err(ErreurRendu::Occupe); }
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
    if !existe { p.anneaux_venus = 1; }
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
    if let Err(etape) = unsafe { p.gems_venus.ferme(&mut p.file, gpu3d::CONTEXTE_VENUS).and_then(|()| ferme_objets(&mut p.file, blob.is_some())) } {
        crate::serial_println!("P10_VENUS_ORPHELIN_ECHEC etape={}", etape);
        return Err(ErreurRendu::Commande);
    }
    p.hostmem_venus = Some(Canal::Nouveau(fenetre));
    crate::serial_println!("P10_VENUS_ORPHELIN_RECUPERE");
    Ok((false, None))
}

fn ferme_second(p: &mut Peripherique) -> Result<(), ErreurRendu> {
    if p.gems_venus.baux_actifs(SECOND_CONTEXTE) { return Err(ErreurRendu::Occupe); }
    let (q, n) = gpu3d::contexte_venus_id(SECOND_CONTEXTE, false);
    if unsafe { p.gems_venus.ferme(&mut p.file, SECOND_CONTEXTE) }.is_err()
        || unsafe { p.file.commande(&q[..n], ENTETE) } != Ok(RESP_OK_NODATA)
    {
        p.hostmem_venus = None;
        return Err(ErreurRendu::Commande);
    }
    p.second_venus = None;
    Ok(())
}

fn recupere_second(p: &mut Peripherique) -> Result<(), ErreurRendu> {
    if p.hostmem_venus.is_none() { return Err(ErreurRendu::Indisponible); }
    if p.second_venus.as_ref().is_some_and(|c| c.proprietaire.strong_count() == 0) {
        ferme_second(p)?;
        crate::serial_println!("P10_VENUS_SECOND_ORPHELIN_RECUPERE");
    }
    Ok(())
}

pub fn initialise_contexte_venus(session: &Arc<()>, anneaux: u8) -> Result<(), ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let proprietaire = Arc::downgrade(session);
    recupere_second(p)?;
    if p.second_venus.as_ref().is_some_and(|c| c.proprietaire.ptr_eq(&proprietaire)) {
        return Err(ErreurRendu::Existe);
    }
    match prepare_canal(p, &proprietaire) {
        Ok((true, _)) => return Err(ErreurRendu::Existe),
        Ok((false, _)) => {}
        Err(ErreurRendu::Occupe) if p.second_venus.is_none()
            && matches!(p.hostmem_venus, Some(Canal::Ouvert(fenetre, None, _)) if fenetre.permet_second()) =>
        {
            let (q, n) = gpu3d::contexte_venus_id(SECOND_CONTEXTE, true);
            return match unsafe { p.file.commande(&q[..n], ENTETE) } {
                Ok(RESP_OK_NODATA) => {
                    p.second_venus = Some(SecondContexte { proprietaire, anneaux });
                    Ok(())
                }
                Ok(_) => Err(ErreurRendu::Commande),
                Err(_) => {
                    p.hostmem_venus = None;
                    Err(ErreurRendu::Commande)
                }
            };
        }
        Err(e) => return Err(e),
    }
    let Some(Canal::Nouveau(fenetre)) = p.hostmem_venus.take() else {
        return Err(ErreurRendu::Indisponible);
    };
    let (q, n) = gpu3d::contexte_venus(true);
    match unsafe { p.file.commande(&q[..n], ENTETE) } {
        Ok(RESP_OK_NODATA) => {
            p.anneaux_venus = anneaux;
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

pub fn host_visible_venus() -> bool {
    verrou().is_some_and(|garde| garde.as_ref().is_some_and(|p| {
        !p.file.cassee && match &p.hostmem_venus {
            Some(Canal::Nouveau(fenetre) | Canal::Ouvert(fenetre, ..)) => fenetre.octets_par_gem() != 0,
            None => false,
        }
    }))
}

pub fn contexte_venus_disponible() -> bool {
    verrou().is_some_and(|garde| garde.as_ref().is_some_and(|p| !p.file.cassee && p.hostmem_venus.is_some()))
}

pub fn ferme_vulkan_banc(session: &Arc<()>) -> Result<(), ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    if p.hostmem_venus.is_none() { return Err(ErreurRendu::Indisponible); }
    if let Some(contexte) = &p.second_venus {
        if contexte.proprietaire.ptr_eq(&Arc::downgrade(session)) {
            return ferme_second(p);
        }
        if matches!(p.hostmem_venus, Some(Canal::Nouveau(_))) {
            return Err(ErreurRendu::Occupe);
        }
    }
    if p.gems_venus.baux_actifs(gpu3d::CONTEXTE_VENUS) { return Err(ErreurRendu::Occupe); }
    if let Some(Canal::Ouvert(_, _, ancien)) = &p.hostmem_venus {
        if !ancien.ptr_eq(&Arc::downgrade(session)) { return Err(ErreurRendu::Occupe); }
    }
    let canal = p.hostmem_venus.take().ok_or(ErreurRendu::Indisponible)?;
    let fenetre = match canal {
        Canal::Nouveau(fenetre) => fenetre,
        Canal::Ouvert(fenetre, blob, _) => match unsafe { p.gems_venus.ferme(&mut p.file, gpu3d::CONTEXTE_VENUS).and_then(|()| ferme_objets(&mut p.file, blob.is_some())) } {
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
        Some(Canal::Ouvert(_, _, proprietaire)) if !proprietaire.ptr_eq(&Arc::downgrade(session)) => {
            Err(ErreurRendu::Occupe)
        }
        Some(Canal::Ouvert(fenetre, Some(_), _)) => Ok(fenetre.adresse),
        _ => Err(ErreurRendu::Invalide),
    }
}

pub fn soumets_venus(session: &Arc<()>, flux: &[u8], anneau: u8) -> Result<(), ErreurRendu> {
    let mut q = alloc::vec::Vec::new();
    q.try_reserve_exact(32 + flux.len()).map_err(|_| ErreurRendu::Plein)?;
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    let (fenetre, contexte, anneaux, blob_prive) = possede_contexte(p, session)?;
    if anneau >= anneaux { return Err(ErreurRendu::Invalide); }
    let entete = gpu3d::entete_soumission_venus(contexte, flux.len(), anneau).ok_or(ErreurRendu::Invalide)?;
    q.extend_from_slice(&entete);
    q.extend_from_slice(flux);

    let memo = blob_prive.then_some(fenetre.adresse as *mut u32);
    unsafe {
        if let Some(memo) = memo {
            for i in 0..OCTETS_REPONSE_VENUS / 4 { write_volatile(memo.add(i), u32::MAX); }
            fence(Ordering::SeqCst);
        }
        if p.file.commande(&q, ENTETE) == Ok(RESP_OK_NODATA)
            && memo.is_none_or(|memo| attente_reponse(250, || read_volatile(memo) != u32::MAX))
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

pub const TAILLE_RELECTURE_VENUS: usize = PAGE;

pub fn relis_memoire_venus(session: &Arc<()>, id_memoire: u64) -> Result<alloc::vec::Vec<u8>, ErreurRendu> {
    if id_memoire == 0 { return Err(ErreurRendu::Invalide); }
    let mut sortie = alloc::vec::Vec::new();
    sortie.try_reserve_exact(PAGE).map_err(|_| ErreurRendu::Plein)?;
    sortie.resize(PAGE, 0);
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| !p.file.cassee).ok_or(ErreurRendu::Indisponible)?;
    aperture_possedee(p, session)?;
    let Some(Canal::Ouvert(Fenetre { relecture: Some(adresse), .. }, _, _)) = &p.hostmem_venus else {
        return Err(ErreurRendu::Indisponible);
    };
    match unsafe { copie_memoire_exportee(&mut p.file, *adresse, id_memoire, &mut sortie) } {
        Ok(()) => Ok(sortie),
        Err(etape) => {
            // La première erreur peut avoir laissé le second blob vivant.
            // Aucune réutilisation des deux fenêtres sans reset.
            p.hostmem_venus = None;
            crate::serial_println!("P10_VENUS_RELECTURE_ECHEC etape={}", etape);
            Err(ErreurRendu::Commande)
        }
    }
}

unsafe fn copie_memoire_exportee(f: &mut FileControle, adresse: usize, id_memoire: u64,
    sortie: &mut [u8]) -> Result<(), &'static str>
{
    if f.commande(&gpu3d::blob_memoire_venus(gpu3d::RESSOURCE_RELECTURE_VENUS, id_memoire, PAGE as u32), ENTETE)? != RESP_OK_NODATA {
        return Err("export-memoire");
    }
    let (q, n) = gpu3d::mappe_memoire_venus(true, MAX_BLOB_REPONSE);
    if f.commande(&q[..n], 32)? != RESP_OK_MAP_INFO {
        let _ = libere(f, gpu3d::RESSOURCE_RELECTURE_VENUS);
        return Err("map-memoire");
    }
    fence(Ordering::SeqCst);
    let mem = adresse as *const u8;
    for (i, octet) in sortie.iter_mut().enumerate() {
        *octet = read_volatile(mem.add(i));
    }
    let (q, n) = gpu3d::mappe_memoire_venus(false, 0);
    if f.commande(&q[..n], ENTETE).map_err(|_| "unmap-memoire-sans-reponse")? != RESP_OK_NODATA { return Err("unmap-memoire"); }
    if libere(f, gpu3d::RESSOURCE_RELECTURE_VENUS)? != RESP_OK_NODATA { return Err("unref-memoire"); }
    Ok(())
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
    if blob && libere(f, gpu3d::RESSOURCE_REPONSE_VENUS) != Ok(RESP_OK_NODATA) {
        return Err("blob-unref");
    }
    let (q, n) = gpu3d::contexte_venus(false);
    if f.commande(&q[..n], ENTETE) != Ok(RESP_OK_NODATA) { return Err("ctx-destroy"); }
    Ok(())
}
