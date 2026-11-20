//! Interface de découverte virtgpu et textures bornées Bouchaud. La file et les ressources VRAM
//! restent privées au pilote. Aucun pointeur utilisateur ne devient une DMA.

use super::{errno, user_read, user_write};
use alloc::sync::Arc;
use crate::drivers::virtio_gpu::{self, ErreurRendu, RegionRendu, TransfertRendu, OCTETS_TEXTURE};

const GETPARAM: u32 = 0xc010_6443;
const GET_CAPS: u32 = 0xc018_6449;
// Bouchaud uniquement : _IOR('B', 0xa0, uint32_t[64]).
const BLIT_SONDE: u32 = 0x8100_42a0;

const CREE: u32 = 0x8004_42a1;
const DETRUIT: u32 = 0x4004_42a2;
const CHARGE: u32 = 0x4010_42a3;
const RELIT: u32 = 0x4010_42a4;
const BLIT: u32 = 0x4008_42a5;

const CREE_TEXTURE: u32 = 0xc010_42a6;
const CHARGE_REGION: u32 = 0x4020_42a7;
const RELIT_REGION: u32 = 0x4020_42a8;
const BLIT_REGION: u32 = 0x4020_42a9;

fn mot(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap())
}

fn transfert(session: &Arc<()>, charge: bool, id: u32, region: Option<RegionRendu>,
    pas: usize, adresse: u64) -> i64
{
    let longueur = match region {
        Some(r) => match r.longueur(pas) {
            Some(n) => n,
            None => return -errno::EINVAL,
        },
        None => OCTETS_TEXTURE,
    };
    let (mut octets, sens) = if charge {
        let Some(octets) = user_read(adresse, longueur) else { return -errno::EFAULT; };
        (octets, TransfertRendu::VersGpu)
    } else {
        if !super::fault_in_user_range(adresse, longueur, true) { return -errno::EFAULT; }
        let mut sortie = alloc::vec::Vec::new();
        if sortie.try_reserve_exact(longueur).is_err() { return -errno::ENOMEM; }
        sortie.resize(longueur, 0);
        (sortie, TransfertRendu::DepuisGpu)
    };
    if let Err(e) = virtio_gpu::transfert_texture_rendu(session, id, region, pas, &mut octets, sens) {
        return erreur(e);
    }
    if charge || user_write(adresse, &octets) { 0 } else { -errno::EFAULT }
}

fn erreur(e: ErreurRendu) -> i64 {
    -(match e {
        ErreurRendu::Indisponible => errno::ENODEV,
        ErreurRendu::Plein => errno::ENOMEM,
        #[cfg(feature = "banc-gpu-venus")]
        ErreurRendu::Occupe => errno::EBUSY,
        #[cfg(feature = "banc-gpu-venus")]
        ErreurRendu::Existe => errno::EEXIST,
        ErreurRendu::Inconnue => errno::ENOENT,
        ErreurRendu::Invalide => errno::EINVAL,
        ErreurRendu::Commande => errno::EIO,
    })
}

pub(super) fn ioctl(session: &Arc<()>, request: u32, arg: u64) -> i64 {
    match request {
        GETPARAM => {
            let Some(buf) = user_read(arg, 16) else { return -errno::EFAULT };
            let param = u64::from_le_bytes(buf[..8].try_into().unwrap());
            let adresse = u64::from_le_bytes(buf[8..].try_into().unwrap());
            let Some(masque) = virtio_gpu::masque_capsets_rendu() else { return -errno::ENODEV };
            let valeur = match param {
                1 | 2 => 1u64, // matériel virtuel 3D et interrogation des capsets
                #[cfg(feature = "banc-gpu-venus")]
                6 => virtio_gpu::contexte_venus_disponible() as u64,
                3..=5 | 8 => 0,
                #[cfg(not(feature = "banc-gpu-venus"))]
                6 => 0, // blob, host-visible, cross-device, context-init, nom
                7 => masque, // les seuls capsets lisibles par cette interface
                _ => return -errno::EINVAL,
            };
            if user_write(adresse, &valeur.to_le_bytes()) { 0 } else { -errno::EFAULT }
        }
        GET_CAPS => {
            let Some(buf) = user_read(arg, 24) else { return -errno::EFAULT };
            let id = u32::from_le_bytes(buf[..4].try_into().unwrap());
            let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            let adresse = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            let taille = u32::from_le_bytes(buf[16..20].try_into().unwrap()) as usize;
            if taille == 0 { return -errno::EINVAL; }
            let mut sortie = alloc::vec![0; taille.min(virtio_gpu::CAPSET_OCTETS_MAX)];
            let Some(n) = virtio_gpu::copie_capset(id, version, &mut sortie) else {
                return -errno::EINVAL;
            };
            if user_write(adresse, &sortie[..n]) { 0 } else { -errno::EFAULT }
        }
        BLIT_SONDE => {
            let mut octets = [0u8; 256];
            if !super::fault_in_user_range(arg, octets.len(), true) { return -errno::EFAULT; }
            let pixels = match virtio_gpu::blit_sonde() {
                Ok(pixels) => pixels,
                Err("occupe") => return -errno::EAGAIN,
                Err("absent") => return -errno::ENODEV,
                Err("deja-jouee") => return -errno::EBUSY,
                Err(_) => return -errno::EIO,
            };
            for (sortie, pixel) in octets.chunks_exact_mut(4).zip(pixels) {
                sortie.copy_from_slice(&pixel.to_le_bytes());
            }
            if user_write(arg, &octets) { 0 } else { -errno::EFAULT }
        }
        CREE | CREE_TEXTURE => {
            let (dimensions, sortie) = if request == CREE_TEXTURE {
                let Some(buf) = user_read(arg, 16) else { return -errno::EFAULT; };
                if mot(&buf, 8) != 2 || mot(&buf, 12) != 0 { return -errno::EINVAL; }
                let Some(sortie) = arg.checked_add(12) else { return -errno::EFAULT; };
                (Some((mot(&buf, 0), mot(&buf, 4))), sortie)
            } else {
                (None, arg)
            };
            if !super::fault_in_user_range(sortie, 4, true) { return -errno::EFAULT; }
            let id = match virtio_gpu::cree_texture_rendu(session, dimensions) {
                Ok(id) => id,
                Err(e) => return erreur(e),
            };
            if !user_write(sortie, &id.to_le_bytes()) {
                let _ = virtio_gpu::detruit_texture_rendu(session, id);
                return -errno::EFAULT;
            }
            0
        }
        DETRUIT => {
            let Some(id) = super::user_read_u32(arg) else { return -errno::EFAULT; };
            virtio_gpu::detruit_texture_rendu(session, id).map_or_else(erreur, |()| 0)
        }
        CHARGE | RELIT => {
            let Some(buf) = user_read(arg, 16) else { return -errno::EFAULT; };
            if mot(&buf, 4) != 0 { return -errno::EINVAL; }
            let adresse = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            transfert(session, request == CHARGE, mot(&buf, 0), None, 32, adresse)
        }
        CHARGE_REGION | RELIT_REGION => {
            let Some(buf) = user_read(arg, 32) else { return -errno::EFAULT; };
            let region = RegionRendu {
                x: mot(&buf, 4), y: mot(&buf, 8), largeur: mot(&buf, 12), hauteur: mot(&buf, 16),
            };
            let adresse = u64::from_le_bytes(buf[24..32].try_into().unwrap());
            transfert(session, request == CHARGE_REGION, mot(&buf, 0), Some(region), mot(&buf, 20) as usize, adresse)
        }
        BLIT | BLIT_REGION => {
            let taille = if request == BLIT_REGION { 32 } else { 8 };
            let Some(buf) = user_read(arg, taille) else { return -errno::EFAULT; };
            let zones = if request == BLIT_REGION {
                let (largeur, hauteur) = (mot(&buf, 24), mot(&buf, 28));
                Some((
                    RegionRendu { x: mot(&buf, 8), y: mot(&buf, 12), largeur, hauteur },
                    RegionRendu { x: mot(&buf, 16), y: mot(&buf, 20), largeur, hauteur },
                ))
            } else {
                None
            };
            virtio_gpu::blit_texture_rendu(session, mot(&buf, 0), mot(&buf, 4), zones)
                .map_or_else(erreur, |()| 0)
        }
        // DRM_IOCTL_VIRTGPU_RESOURCE_CREATE_BLOB. HOST3D mappable sans
        // commandes attachées, partage PRIME ou pages invitées.
        #[cfg(feature = "banc-gpu-venus")]
        0xc030_644a => {
            let Some(mut buf) = user_read(arg, 48) else { return -errno::EFAULT; };
            if mot(&buf, 0) != 2 || mot(&buf, 4) != 1 || buf[24..40].iter().any(|b| *b != 0) {
                return -errno::EINVAL;
            }
            if !super::fault_in_user_range(arg, 48, true) { return -errno::EFAULT; }
            let octets = u64::from_le_bytes(buf[16..24].try_into().unwrap());
            let id = u64::from_le_bytes(buf[40..48].try_into().unwrap());
            let gem = match virtio_gpu::cree_gem_venus(session, id, octets) {
                Ok(gem) => gem,
                Err(e) => return erreur(e),
            };
            buf[8..12].copy_from_slice(&gem.handle.to_le_bytes());
            buf[12..16].copy_from_slice(&gem.ressource.to_le_bytes());
            if user_write(arg, &buf) { return 0; }
            let _ = virtio_gpu::ferme_gem_venus(session, gem.handle);
            -errno::EFAULT
        }
        #[cfg(feature = "banc-gpu-venus")]
        0xc010_6445 => {
            let Some(mut buf) = user_read(arg, 16) else { return -errno::EFAULT; };
            let gem = match virtio_gpu::info_gem_venus(session, mot(&buf, 0)) {
                Ok(gem) => gem,
                Err(e) => return erreur(e),
            };
            buf[4..8].copy_from_slice(&gem.ressource.to_le_bytes());
            buf[8..12].copy_from_slice(&gem.octets.to_le_bytes());
            buf[12..16].copy_from_slice(&2u32.to_le_bytes());
            if user_write(arg, &buf) { 0 } else { -errno::EFAULT }
        }
        #[cfg(feature = "banc-gpu-venus")]
        0x4008_6409 => {
            let Some(buf) = user_read(arg, 8) else { return -errno::EFAULT; };
            if mot(&buf, 4) != 0 { return -errno::EINVAL; }
            virtio_gpu::ferme_gem_venus(session, mot(&buf, 0)).map_or_else(erreur, |()| 0)
        }
        // DRM_IOCTL_VIRTGPU_EXECBUFFER, profil de banc synchrone.
        // Les flux qui demandent une réponse utilisent le blob de transport
        // de l'ouverture. Pas de liste GEM ni de syncobj dans ce profil.
        #[cfg(feature = "banc-gpu-venus")]
        0xc040_6442 => {
            let Some(buf) = user_read(arg, 64) else { return -errno::EFAULT; };
            let taille = mot(&buf, 4) as usize;
            if !matches!(mot(&buf, 0), 0 | 4) || taille == 0 || taille > 2016
                || taille % 4 != 0 || mot(&buf, 28) != u32::MAX
                || buf[16..28].iter().chain(&buf[32..]).any(|b| *b != 0)
            {
                return -errno::EINVAL;
            }
            let adresse = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            let Some(flux) = user_read(adresse, taille) else { return -errno::EFAULT; };
            virtio_gpu::soumets_venus(session, &flux).map_or_else(erreur, |()| 0)
        }
        #[cfg(feature = "banc-gpu-venus")]
        0x8040_42ad | 0x8200_42ad => {
            let longueur = if request == 0x8040_42ad { 64 } else { virtio_gpu::OCTETS_REPONSE_VENUS };
            if !super::fault_in_user_range(arg, longueur, true) {
                return -errno::EFAULT;
            }
            match virtio_gpu::copie_reponse_venus(session) {
                Ok(octets) if user_write(arg, &octets[..longueur]) => 0,
                Ok(_) => -errno::EFAULT,
                Err(e) => erreur(e),
            }
        }
        // Bouchaud _IOW('B', 0xae, {u64 id_memoire, u64 destination}).
        #[cfg(feature = "banc-gpu-venus")]
        0x4010_42ae => {
            let Some(buf) = user_read(arg, 16) else { return -errno::EFAULT; };
            let id = u64::from_le_bytes(buf[..8].try_into().unwrap());
            let adresse = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            if !super::fault_in_user_range(adresse, virtio_gpu::TAILLE_RELECTURE_VENUS, true) {
                return -errno::EFAULT;
            }
            match virtio_gpu::relis_memoire_venus(session, id) {
                Ok(octets) if user_write(adresse, &octets) => 0,
                Ok(_) => -errno::EFAULT,
                Err(e) => erreur(e),
            }
        }
        // DRM_IOCTL_VIRTGPU_CONTEXT_INIT, sous-ensemble Venus à un anneau.
        #[cfg(feature = "banc-gpu-venus")]
        0xc010_644b => {
            let Some(buf) = user_read(arg, 16) else { return -errno::EFAULT; };
            let n = mot(&buf, 0) as usize;
            if !(1..=2).contains(&n) || mot(&buf, 4) != 0 { return -errno::EINVAL; }
            let adresse = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            let Some(params) = user_read(adresse, n * 16) else { return -errno::EFAULT; };
            let mut capset = false;
            let mut anneaux = false;
            for p in params.chunks_exact(16) {
                let cle = u64::from_le_bytes(p[..8].try_into().unwrap());
                let valeur = u64::from_le_bytes(p[8..16].try_into().unwrap());
                match (cle, valeur) {
                    (1, 4) if !capset => capset = true,
                    (2, 1) if !anneaux => anneaux = true,
                    _ => return -errno::EINVAL,
                }
            }
            if !capset { return -errno::EINVAL; }
            virtio_gpu::initialise_contexte_venus(session).map_or_else(erreur, |()| 0)
        }
        // Bouchaud expérimental : _IOR('B', 0xaa, uint32_t).
        #[cfg(feature = "banc-gpu-venus")]
        0x8004_42aa => {
            if !super::fault_in_user_range(arg, 4, true) { return -errno::EFAULT; }
            match virtio_gpu::version_vulkan_banc(session, 4096) {
                Ok(version) if user_write(arg, &version.to_le_bytes()) => 0,
                Ok(_) => -errno::EFAULT,
                Err(e) => erreur(e),
            }
        }
        // _IOWR('B', 0xac, {u32 octets, u32 version}).
        #[cfg(feature = "banc-gpu-venus")]
        0xc008_42ac => {
            let Some(buf) = user_read(arg, 8) else { return -errno::EFAULT; };
            if mot(&buf, 4) != 0 { return -errno::EINVAL; }
            let Some(sortie) = arg.checked_add(4) else { return -errno::EFAULT; };
            if !super::fault_in_user_range(sortie, 4, true) { return -errno::EFAULT; }
            match virtio_gpu::version_vulkan_banc(session, mot(&buf, 0)) {
                Ok(version) if user_write(sortie, &version.to_le_bytes()) => 0,
                Ok(_) => -errno::EFAULT,
                Err(e) => erreur(e),
            }
        }
        #[cfg(feature = "banc-gpu-venus")]
        0x42ab => virtio_gpu::ferme_vulkan_banc(session).map_or_else(erreur, |()| 0),
        _ => -errno::ENOTTY,
    }
}
