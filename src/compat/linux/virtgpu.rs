//! Partie découverte de l'ABI Linux virtgpu. La file et les ressources VRAM
//! restent privées au pilote. Aucun pointeur utilisateur ne devient une DMA.

use super::{errno, user_read, user_write};
use crate::drivers::virtio_gpu;

const GETPARAM: u32 = 0xc010_6443;
const GET_CAPS: u32 = 0xc018_6449;
// Bouchaud uniquement : _IOR('B', 0xa0, uint32_t[64]).
const BLIT_SONDE: u32 = 0x8100_42a0;

pub(super) fn ioctl(request: u32, arg: u64) -> i64 {
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
        _ => -errno::ENOTTY,
    }
}
