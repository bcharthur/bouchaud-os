//! P10: upload DMA d'une surface de fenetre et blit VIRGL vers le scanout.
//! Module protocolaire sans allocation, testable par `rustc --test` sous Windows.
//! La mise en file, le DMA et l'isolation sont faits par virtio_gpu.rs.

pub const RESOURCE_ID: u32 = 0xB120;
/// Maximum de textures indépendantes pour une trame Atelier P10.
/// Les identifiants restent privés au contexte 3D du pilote.
pub const MAX_TEXTURES: usize = 4;

pub fn resource_id(slot: usize) -> Option<u32> {
    // then_some() evalue son argument meme lorsque la condition est fausse.
    // Un slot hostile (usize::MAX) ne doit jamais deborder en debug.
    if slot >= MAX_TEXTURES { return None; }
    Some(RESOURCE_ID + slot as u32)
}

fn ressource_valide(id: u32) -> bool {
    id >= RESOURCE_ID && id < RESOURCE_ID + MAX_TEXTURES as u32
}
pub const TARGET_TEXTURE_2D: u32 = 2;
pub const FORMAT_B8G8R8X8: u32 = 2;
pub const BIND_SAMPLER_VIEW: u32 = 1 << 3;
pub const CMD_RESOURCE_CREATE_3D: u32 = 0x0204;
pub const CMD_CTX_ATTACH_RESOURCE: u32 = 0x0202;
pub const CMD_TRANSFER_TO_HOST_3D: u32 = 0x0205;
pub const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
pub const CONTEXT_ID: u32 = 0xB104;

fn put32(b: &mut [u8], offset: usize, value: u32) {
    b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(b: &mut [u8], offset: usize, value: u64) {
    b[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// QEMU include/standard-headers/linux/virtio_gpu.h: 72 bytes.
pub fn create_source(w: u32, h: u32) -> Option<[u8; 72]> {
    create_source_id(RESOURCE_ID, w, h)
}

/// Une ressource 3D par calque, pour pouvoir grouper les BLIT dans
/// une seule commande SUBMIT_3D au lieu de soumettre chaque fenetre.
pub fn create_source_id(id: u32, w: u32, h: u32) -> Option<[u8; 72]> {
    if !ressource_valide(id) || w == 0 || h == 0 || w > 4096 || h > 2160 {
        return None;
    }
    let mut q = [0u8; 72];
    put32(&mut q, 0, CMD_RESOURCE_CREATE_3D);
    put32(&mut q, 24, id);
    put32(&mut q, 28, TARGET_TEXTURE_2D);
    put32(&mut q, 32, FORMAT_B8G8R8X8);
    put32(&mut q, 36, BIND_SAMPLER_VIEW);
    put32(&mut q, 40, w);
    put32(&mut q, 44, h);
    put32(&mut q, 48, 1);
    put32(&mut q, 52, 1);
    Some(q)
}

pub fn attach_context(resource: u32) -> [u8; 32] {
    let mut q = [0u8; 32];
    put32(&mut q, 0, CMD_CTX_ATTACH_RESOURCE);
    put32(&mut q, 16, CONTEXT_ID);
    put32(&mut q, 24, resource);
    q
}

pub fn attach_backing(physique: u64, octets: usize) -> Option<[u8; 48]> {
    attach_backing_id(RESOURCE_ID, physique, octets)
}

pub fn attach_backing_id(id: u32, physique: u64, octets: usize) -> Option<[u8; 48]> {
    let n = u32::try_from(octets).ok()?;
    if !ressource_valide(id) || n == 0 || physique == 0 { return None; }
    let mut q = [0u8; 48];
    put32(&mut q, 0, CMD_RESOURCE_ATTACH_BACKING);
    put32(&mut q, 24, id);
    put32(&mut q, 28, 1);
    put64(&mut q, 32, physique);
    put32(&mut q, 40, n);
    Some(q)
}

/// Code QEMU: header 24 + box 24 + offset 8 + res id 4 + level 4
/// + stride 4 + layer_stride 4 = 72.
pub fn transfer_to_gpu(w: u32, h: u32, stride: u32) -> Option<[u8; 72]> {
    transfer_to_gpu_id(RESOURCE_ID, w, h, stride)
}

pub fn transfer_to_gpu_id(id: u32, w: u32, h: u32, stride: u32) -> Option<[u8; 72]> {
    if !ressource_valide(id) || w == 0 || h == 0 || w > 4096 || h > 2160
        || stride < w.checked_mul(4)? {
        return None;
    }
    let layer_stride = stride.checked_mul(h)?;
    let mut q = [0u8; 72];
    put32(&mut q, 0, CMD_TRANSFER_TO_HOST_3D);
    // box x,y,z stays zero
    put32(&mut q, 36, w);
    put32(&mut q, 40, h);
    put32(&mut q, 44, 1);
    // offset u64 at 48: zero
    put32(&mut q, 56, id);
    // level 60: zero
    put32(&mut q, 64, stride);
    put32(&mut q, 68, layer_stride);
    Some(q)
}

// BOUCHAUD_P10_ORIENTATION_ATOMIC_20261109
// Une image ring3 est top-down, mais le chemin texture 3D -> scanout 2D
// observe sous QEMU restitue les lignes inversees. Inverser UNE fois les
// lignes du DMA source (sans modifier les coordonnees Gallium).
#[inline]
pub fn ligne_source_gpu(ligne: usize, hauteur: usize) -> usize {
    debug_assert!(ligne < hauteur);
    hauteur - 1 - ligne
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pool_de_quatre_textures_distinctes() {
        for slot in 0..MAX_TEXTURES {
            let id = resource_id(slot).unwrap();
            assert_eq!(get(&create_source_id(id, 1280, 720).unwrap(), 24), id);
            assert_eq!(get(&attach_backing_id(id, 0x1000, 4096).unwrap(), 24), id);
            assert_eq!(get(&transfer_to_gpu_id(id, 100, 100, 512).unwrap(), 56), id);
        }
        assert!(resource_id(MAX_TEXTURES).is_none());
        assert!(resource_id(usize::MAX).is_none());
        assert!(create_source_id(RESOURCE_ID - 1, 10, 10).is_none());
        assert!(attach_backing_id(RESOURCE_ID + 4, 0x1000, 4096).is_none());
        assert!(transfer_to_gpu_id(RESOURCE_ID + 4, 10, 10, 40).is_none());
    }
    #[test]
    fn inversion_des_lignes_gpu_sans_perte() {
        let source = [11u32, 12, 21, 22, 31, 32];
        let mut gpu = [0u32; 6];
        for yy in 0..3 {
            let src = ligne_source_gpu(yy, 3) * 2;
            gpu[yy * 2..yy * 2 + 2].copy_from_slice(&source[src..src + 2]);
        }
        assert_eq!(gpu, [31, 32, 21, 22, 11, 12]);
        assert_eq!(ligne_source_gpu(0, 1), 0);
    }
    fn get(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o+4].try_into().unwrap()) }
    #[test] fn creation_3d_reelle() {
        let q = create_source(1280,720).unwrap();
        assert_eq!(get(&q, 0), 0x0204);
        assert_eq!(get(&q, 36), 8);
        assert_eq!(get(&q, 40), 1280);
        assert_eq!(get(&q, 44), 720);
    }
    #[test] fn transfert_3d_reel() {
        let q = transfer_to_gpu(1100,604,5120).unwrap();
        assert_eq!(get(&q, 0), 0x0205);
        assert_eq!(get(&q, 36), 1100);
        assert_eq!(get(&q, 40), 604);
        assert_eq!(get(&q, 56), RESOURCE_ID);
        assert_eq!(get(&q, 64), 5120);
        assert_eq!(get(&q, 68), 5120*604);
    }
    #[test] fn attachement_domaine() {
        let q = attach_context(1);
        assert_eq!(get(&q, 0), 0x0202);
        assert_eq!(get(&q, 16), CONTEXT_ID);
        assert_eq!(get(&q, 24), 1);
    }
    #[test] fn refus_dimensions_hostiles() {
        assert!(create_source(0,720).is_none());
        assert!(create_source(9000,720).is_none());
        assert!(transfer_to_gpu(1280,720,32).is_none());
        assert!(transfer_to_gpu(u32::MAX,720,5120).is_none());
        assert!(attach_backing(0,100).is_none());
    }
}
