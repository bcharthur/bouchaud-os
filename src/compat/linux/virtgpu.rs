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
            let Some(id) = virtio_gpu::capset_render_id() else { return -errno::ENODEV };
            let valeur = match param {
                1 | 2 => 1u64, // matériel virtuel 3D et interrogation des capsets
                3..=6 | 8 => 0, // blob, host-visible, cross-device, context-init, nom
                7 => 1u64 << id, // les seuls capsets lisibles par cette interface
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
        _ => -errno::ENOTTY,
    }
}
