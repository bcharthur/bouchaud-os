//! Interface de découverte virtgpu et textures bornées Bouchaud. La file et les ressources VRAM
//! restent privées au pilote. Aucun pointeur utilisateur ne devient une DMA.

use super::{errno, user_read, user_write};
use alloc::sync::Arc;
use crate::drivers::virtio_gpu::{self, ErreurRendu, TransfertRendu, OCTETS_TEXTURE};

const GETPARAM: u32 = 0xc010_6443;
const GET_CAPS: u32 = 0xc018_6449;
// Bouchaud uniquement : _IOR('B', 0xa0, uint32_t[64]).
const BLIT_SONDE: u32 = 0x8100_42a0;

const CREE: u32 = 0x8004_42a1;
const DETRUIT: u32 = 0x4004_42a2;
const CHARGE: u32 = 0x4010_42a3;
const RELIT: u32 = 0x4010_42a4;
const BLIT: u32 = 0x4008_42a5;

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
        CREE => {
            if !super::fault_in_user_range(arg, 4, true) { return -errno::EFAULT; }
            let id = match virtio_gpu::cree_texture_rendu(session) {
                Ok(id) => id,
                Err(e) => return erreur(e),
            };
            if !user_write(arg, &id.to_le_bytes()) {
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
            let id = u32::from_le_bytes(buf[..4].try_into().unwrap());
            if buf[4..8] != [0; 4] { return -errno::EINVAL; }
            let adresse = u64::from_le_bytes(buf[8..].try_into().unwrap());
            if request == CHARGE {
                let Some(octets) = user_read(adresse, OCTETS_TEXTURE) else { return -errno::EFAULT; };
                let sens = TransfertRendu::VersGpu(octets.as_slice().try_into().unwrap());
                return virtio_gpu::transfert_texture_rendu(session, id, sens)
                    .map_or_else(erreur, |()| 0);
            }
            if !super::fault_in_user_range(adresse, OCTETS_TEXTURE, true) { return -errno::EFAULT; }
            let mut octets = [0; OCTETS_TEXTURE];
            let sens = TransfertRendu::DepuisGpu(&mut octets);
            if let Err(e) = virtio_gpu::transfert_texture_rendu(session, id, sens) {
                return erreur(e);
            }
            if user_write(adresse, &octets) { 0 } else { -errno::EFAULT }
        }
        BLIT => {
            let Some(buf) = user_read(arg, 8) else { return -errno::EFAULT; };
            let source = u32::from_le_bytes(buf[..4].try_into().unwrap());
            let cible = u32::from_le_bytes(buf[4..].try_into().unwrap());
            virtio_gpu::blit_texture_rendu(session, source, cible).map_or_else(erreur, |()| 0)
        }
        _ => -errno::ENOTTY,
    }
}
