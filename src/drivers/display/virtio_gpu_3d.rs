//! P10+.4 (experimental, opt-in): verified virtio-gpu 3D control protocol.
//!
//! Does not draw pixels: this is host-side 3D context + render-target
//! resource management and a trusted-kernel-only command submission ABI.
//! No DRM uAPI, Mesa, Skia GPU or WebGL. The 2D default remains unchanged.
//!
//! Layouts match include/uapi/linux/virtio_gpu.h (little-endian).

pub const F_VIRGL: u64 = 1;
pub const F_VERSION_1: u64 = 1 << 32;
#[cfg(any(feature = "banc-gpu-venus", test))]
pub const F_VENUS: u64 = (1 << 3) | (1 << 4); // RESOURCE_BLOB et CONTEXT_INIT
#[cfg(any(feature = "banc-gpu-venus", test))]
pub const CAPSET_VENUS: u32 = 4;
#[cfg(any(feature = "banc-gpu-venus", test))]
pub const CONTEXTE_VENUS: u32 = 0xB105;
#[cfg(any(feature = "banc-gpu-venus", test))]
pub const RESSOURCE_BLOB_BANC: u32 = 0xB106;
pub const CMD_GET_CAPSET_INFO: u32 = 0x0108;
pub const CMD_GET_CAPSET: u32 = 0x0109;
pub const CMD_CTX_CREATE: u32 = 0x0200;
pub const CMD_CTX_DESTROY: u32 = 0x0201;
pub const CMD_CTX_ATTACH_RESOURCE: u32 = 0x0202;
pub const CMD_CTX_DETACH_RESOURCE: u32 = 0x0203;
pub const CMD_RESOURCE_CREATE_3D: u32 = 0x0204;
pub const CMD_SUBMIT_3D: u32 = 0x0207;
pub const RESP_OK_CAPSET_INFO: u32 = 0x1102;
pub const RESP_OK_CAPSET: u32 = 0x1103;
pub const RESP_OK_NODATA: u32 = 0x1100;
pub const CAPSET_VIRGL: u32 = 1;
pub const CAPSET_VIRGL2: u32 = 2;
pub const MAX_CAPSETS_TO_SCAN: u32 = 16;
pub const CONTEXT_ID: u32 = 0xB104;
pub const RESOURCE_ID: u32 = 0xB103;
pub const RESOURCE_DIM: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapsetInfo {
    pub id: u32,
    pub version: u32,
    pub size: u32,
}

#[inline]
fn put32(buf: &mut [u8], offset: usize, x: u32) {
    buf[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
}

#[inline]
fn get32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
}

/// Never negotiate VIRGL unless explicitly opted in AND offered by the host.
pub fn negotiated_features(offered: u64, opt_in: bool) -> u64 {
    F_VERSION_1 | if opt_in && offered & F_VIRGL != 0 { F_VIRGL } else { 0 }
}

// Réserver le profil aux périphériques proposant les deux fonctionnalités et VIRGL.
#[cfg(any(feature = "banc-gpu-venus", test))]
pub fn fonctionnalites_venus(offertes: u64) -> u64 {
    let requises = F_VERSION_1 | F_VIRGL | F_VENUS;
    if offertes & requises == requises { F_VENUS } else { 0 }
}

#[cfg(any(feature = "banc-gpu-venus", test))]
pub fn contexte_venus(cree: bool) -> ([u8; 96], usize) {
    contexte_venus_id(CONTEXTE_VENUS, cree)
}

#[cfg(any(feature = "banc-gpu-venus", test))]
pub fn contexte_venus_id(contexte: u32, cree: bool) -> ([u8; 96], usize) {
    let mut q = [0u8; 96];
    put32(&mut q, 0, if cree { CMD_CTX_CREATE } else { CMD_CTX_DESTROY });
    put32(&mut q, 16, contexte);
    if !cree { return (q, 24); }
    let nom = b"bouchaud-venus";
    put32(&mut q, 24, nom.len() as u32);
    put32(&mut q, 28, CAPSET_VENUS);
    q[32..32 + nom.len()].copy_from_slice(nom);
    (q, 96)
}

// Un blob GUEST sans mappage hôte, adossé à une page du noyau.
#[cfg(any(feature = "banc-gpu-venus", test))]
pub fn blob_invite(physique: u64, octets: u32) -> [u8; 72] {
    let mut q = [0u8; 72];
    put32(&mut q, 0, 0x010c); // RESOURCE_CREATE_BLOB
    put32(&mut q, 24, RESSOURCE_BLOB_BANC);
    put32(&mut q, 28, 1); // BLOB_MEM_GUEST
    put32(&mut q, 36, 1); // nr_entries
    q[48..56].copy_from_slice(&(octets as u64).to_le_bytes());
    q[56..64].copy_from_slice(&physique.to_le_bytes());
    put32(&mut q, 64, octets);
    q
}

// Blob ID zéro : SHM du render server Venus (virglrenderer 1.1.0,
// venus/vkr_context.c), réservé au flux de réponse du banc.
#[cfg(any(feature = "banc-gpu-venus", test))]
pub const RESSOURCE_REPONSE_VENUS: u32 = 0xB107;

#[cfg(any(feature = "banc-gpu-venus", test))]
pub fn blob_reponse_venus(octets: u32) -> [u8; 56] {
    let mut q = [0; 56];
    put32(&mut q, 0, 0x010c); // RESOURCE_CREATE_BLOB
    put32(&mut q, 16, CONTEXTE_VENUS);
    put32(&mut q, 24, RESSOURCE_REPONSE_VENUS);
    put32(&mut q, 28, 2); // HOST3D
    put32(&mut q, 32, 1); // USE_MAPPABLE
    put32(&mut q, 48, octets);
    q
}

#[cfg(any(feature = "banc-gpu-venus", test))]
pub fn mappe_reponse_venus(mappe: bool) -> ([u8; 40], usize) {
    let mut q = [0; 40];
    put32(&mut q, 0, if mappe { 0x0208 } else { 0x0209 });
    put32(&mut q, 24, RESSOURCE_REPONSE_VENUS);
    // Offset zéro, un seul blob mappé à la fois.
    (q, if mappe { 40 } else { 32 })
}

#[cfg(any(feature = "banc-gpu-venus", test))]
const VENUS_ENUMERATE_INSTANCE_VERSION: u32 = 137;

#[cfg(feature = "banc-gpu-venus")]
pub const RESSOURCE_RELECTURE_VENUS: u32 = 0xB108;

#[cfg(feature = "banc-gpu-venus")]
pub fn blob_memoire_venus(ressource: u32, id_memoire: u64, octets: u32) -> [u8; 56] {
    let mut q = blob_reponse_venus(octets);
    put32(&mut q, 24, ressource);
    q[40..48].copy_from_slice(&id_memoire.to_le_bytes());
    q
}

#[cfg(feature = "banc-gpu-venus")]
pub fn mappe_memoire_venus(mappe: bool, offset: u64) -> ([u8; 40], usize) {
    mappe_blob_venus(RESSOURCE_RELECTURE_VENUS, mappe, offset)
}

#[cfg(feature = "banc-gpu-venus")]
pub const PREMIER_GEM_VENUS: u32 = 0xb110;
#[cfg(feature = "banc-gpu-venus")]
pub const GEMS_VENUS_PRIMAIRE: usize = 8;
#[cfg(feature = "banc-gpu-venus")]
pub const NOMBRE_GEMS_VENUS: usize = GEMS_VENUS_PRIMAIRE + 7;

#[cfg(feature = "banc-gpu-venus")]
pub fn mappe_blob_venus(id: u32, mappe: bool, offset: u64) -> ([u8; 40], usize) {
    let (mut q, n) = mappe_reponse_venus(mappe);
    put32(&mut q, 24, id);
    if mappe { q[32..40].copy_from_slice(&offset.to_le_bytes()); }
    (q, n)
}

/// QEMU 11.1/11.2 déplace l'UNMAP suspendu vers la file de clôtures
/// avant d'en créer une. Sans FENCE il reste dans cmdq jusqu'au rappel RCU.
/// Les ressources admissibles sont HOST3D, sans pages DMA prêtées par l'invité.
pub fn unmap_host3d_qemu(requete: &[u8]) -> bool {
    #[cfg(feature = "banc-gpu-venus")]
    {
        let (reponse, n_reponse) = mappe_reponse_venus(false);
        let (memoire, n_memoire) = mappe_memoire_venus(false, 0);
        if requete == &reponse[..n_reponse] || requete == &memoire[..n_memoire] {
            return true;
        }
        let Some(valeur) = requete.get(24..28) else { return false; };
        let id = u32::from_le_bytes(valeur.try_into().unwrap());
        let (gem, n_gem) = mappe_blob_venus(id, false, 0);
        (PREMIER_GEM_VENUS..PREMIER_GEM_VENUS + NOMBRE_GEMS_VENUS as u32).contains(&id)
            && requete == &gem[..n_gem]
    }
    #[cfg(not(feature = "banc-gpu-venus"))]
    {
        let _ = requete;
        false
    }
}

#[cfg(feature = "banc-gpu-venus")]
pub fn entete_soumission_venus(contexte: u32, octets: usize, anneau: u8) -> Option<[u8; 32]> {
    let mut q = submit_header(octets)?;
    put32(&mut q, 4, 2); // INFO_RING_IDX
    put32(&mut q, 16, contexte);
    q[20] = anneau;
    Some(q)
}

#[cfg(any(feature = "banc-gpu-venus", test))]
pub fn soumet_version_venus() -> [u8; 84] {
    let mut q = [0; 84];
    put32(&mut q, 0, CMD_SUBMIT_3D);
    put32(&mut q, 4, 2); // INFO_RING_IDX, anneau de contrôle 0
    put32(&mut q, 16, CONTEXTE_VENUS);
    put32(&mut q, 24, 52); // taille du flux Venus
    // venus-protocol (virglrenderer 1.1.0) : commande + flags,
    // présence d'un pointeur (u64), puis champs de la structure.
    put32(&mut q, 32, 178); // vkSetReplyCommandStreamMESA
    put32(&mut q, 40, 1);
    put32(&mut q, 48, RESSOURCE_REPONSE_VENUS);
    put32(&mut q, 60, 20); // size_t du flux réponse, offset zéro
    put32(&mut q, 68, VENUS_ENUMERATE_INSTANCE_VERSION);
    put32(&mut q, 72, 1); // VK_COMMAND_GENERATE_REPLY_BIT_EXT
    put32(&mut q, 76, 1); // pApiVersion présent
    q
}

#[cfg(any(feature = "banc-gpu-venus", test))]
pub fn lit_version_venus(reponse: &[u8; 20]) -> Option<u32> {
    if get32(reponse, 0) != VENUS_ENUMERATE_INSTANCE_VERSION || get32(reponse, 4) != 0
        || get32(reponse, 8) != 1 || get32(reponse, 12) != 0
    {
        return None;
    }
    let version = get32(reponse, 16);
    (version >> 22 == 1).then_some(version)
}

/// 32-byte virtio_gpu_get_capset_info (24-byte common header + 8-byte body).
pub fn get_capset_info(index: u32) -> [u8; 32] {
    let mut q = [0u8; 32];
    put32(&mut q, 0, CMD_GET_CAPSET_INFO);
    put32(&mut q, 24, index);
    q
}

/// Exact response length is validated by the caller from virtq used.len.
pub fn parse_capset_info(r: &[u8]) -> Option<CapsetInfo> {
    if r.len() < 40 || get32(r, 0) != RESP_OK_CAPSET_INFO { return None; }
    let info = CapsetInfo { id: get32(r, 24), version: get32(r, 28), size: get32(r, 32) };
    if info.id == 0 || info.size == 0
        || (matches!(info.id, CAPSET_VIRGL | CAPSET_VIRGL2) && info.version == 0)
    { return None; }
    Some(info)
}

/// La charge utile tient dans le descripteur de reponse, en plus de l'entete.
pub fn get_capset(info: CapsetInfo, capacite: usize) -> Option<([u8; 32], usize)> {
    let taille = 24usize.checked_add(info.size as usize)?;
    if info.size == 0 || taille > capacite { return None; }
    let mut q = [0u8; 32];
    put32(&mut q, 0, CMD_GET_CAPSET);
    put32(&mut q, 24, info.id);
    put32(&mut q, 28, info.version);
    Some((q, taille))
}

pub fn capset_payload(reponse: &[u8], taille: usize) -> Option<&[u8]> {
    if reponse.len().checked_sub(24)? != taille || get32(reponse, 0) != RESP_OK_CAPSET {
        return None;
    }
    Some(&reponse[24..])
}

pub fn supported(capset: CapsetInfo) -> bool {
    (capset.id == CAPSET_VIRGL || capset.id == CAPSET_VIRGL2)
        && capset.version != 0 && capset.size != 0
}

/// 96-byte virtio_gpu_ctx_create; context_init=0 for legacy VIRGL.
/// F_CONTEXT_INIT is intentionally not negotiated in this milestone.
pub fn ctx_create() -> [u8; 96] {
    let mut q = [0u8; 96];
    put32(&mut q, 0, CMD_CTX_CREATE);
    put32(&mut q, 16, CONTEXT_ID);
    const NAME: &[u8] = b"bouchaud-p10-virgl";
    put32(&mut q, 24, NAME.len() as u32);
    q[32..32 + NAME.len()].copy_from_slice(NAME);
    q
}

pub fn ctx_destroy() -> [u8; 24] {
    let mut q = [0u8; 24];
    put32(&mut q, 0, CMD_CTX_DESTROY);
    put32(&mut q, 16, CONTEXT_ID);
    q
}

/// 72-byte virtio_gpu_resource_create_3d; resource lives ONLY on GPU host,
/// without guest backing or a display scanout, to preserve 2D invariants.
/// PIPE_TEXTURE_2D=2, B8G8R8X8_UNORM=2, PIPE_BIND_RENDER_TARGET=2.
pub fn resource_create() -> [u8; 72] {
    let mut q = [0u8; 72];
    put32(&mut q, 0, CMD_RESOURCE_CREATE_3D);
    put32(&mut q, 24, RESOURCE_ID);
    put32(&mut q, 28, 2); // PIPE_TEXTURE_2D
    put32(&mut q, 32, 2); // B8G8R8X8_UNORM
    put32(&mut q, 36, 2); // PIPE_BIND_RENDER_TARGET
    put32(&mut q, 40, RESOURCE_DIM);
    put32(&mut q, 44, RESOURCE_DIM);
    put32(&mut q, 48, 1); // depth
    put32(&mut q, 52, 1); // array_size
    q
}

pub fn resource_attach() -> [u8; 32] { context_resource(CMD_CTX_ATTACH_RESOURCE) }
pub fn resource_detach() -> [u8; 32] { context_resource(CMD_CTX_DETACH_RESOURCE) }

fn context_resource(op: u32) -> [u8; 32] {
    let mut q = [0u8; 32];
    put32(&mut q, 0, op);
    put32(&mut q, 16, CONTEXT_ID);
    put32(&mut q, 24, RESOURCE_ID);
    q
}

pub fn resource_unref() -> [u8; 32] {
    let mut q = [0u8; 32];
    put32(&mut q, 0, 0x0102);
    put32(&mut q, 24, RESOURCE_ID);
    q
}

/// Start an encoded SUBMIT_3D command. Caller appends trusted virgl words.
/// Reject empty/unaligned commands; the GPU protocol is not exposed to ring3.
pub fn submit_header(body_len: usize) -> Option<[u8; 32]> {
    if body_len == 0 || body_len % 4 != 0 || body_len > 2016 { return None; }
    let mut q = [0u8; 32];
    put32(&mut q, 0, CMD_SUBMIT_3D);
    put32(&mut q, 16, CONTEXT_ID);
    put32(&mut q, 24, body_len as u32);
    Some(q)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capset_zero_version_and_length() {
        let info = CapsetInfo { id: 4, version: 0, size: 8 };
        let (q, taille) = get_capset(info, 32).unwrap();
        assert_eq!((get32(&q, 0), get32(&q, 24), get32(&q, 28)), (0x0109, 4, 0));
        assert_eq!(taille, 32);
        assert!(get_capset(info, 31).is_none());
        let mut r = [0u8; 32];
        put32(&mut r, 0, RESP_OK_CAPSET);
        r[24] = 42;
        assert_eq!(capset_payload(&r, 8).unwrap()[0], 42);
        assert!(capset_payload(&r[..31], 8).is_none());
        put32(&mut r, 0, RESP_OK_NODATA);
        assert!(capset_payload(&r, 8).is_none());
    }

    #[test]
    fn negotiation_never_enables_3d_by_default() {
        assert_eq!(negotiated_features(F_VERSION_1 | F_VIRGL, false), F_VERSION_1);
        assert_eq!(negotiated_features(F_VERSION_1, true), F_VERSION_1);
        assert_eq!(negotiated_features(F_VERSION_1 | F_VIRGL, true), F_VERSION_1 | F_VIRGL);
    }
    #[test]
    fn venus_exige_les_deux_bits_et_utilise_un_contexte_separe() {
        let offre = F_VERSION_1 | F_VIRGL | F_VENUS;
        assert_eq!(fonctionnalites_venus(offre), F_VENUS);
        for bit in [F_VERSION_1, F_VIRGL, 1 << 3, 1 << 4] {
            assert_eq!(fonctionnalites_venus(offre & !bit), 0);
        }
        let (creation, len) = contexte_venus(true);
        assert_eq!(len, 96);
        assert_eq!(get32(&creation, 28), CAPSET_VENUS);
        let (destruction, len) = contexte_venus(false);
        assert_eq!(len, 24);
        assert_eq!(get32(&destruction, 16), get32(&creation, 16));
        assert_ne!(get32(&creation, 16), CONTEXT_ID);
    }
    #[test]
    fn blob_invite_a_une_entree_et_aucun_drapeau_de_mappage() {
        let blob = blob_invite(0x100000, 4096);
        assert_eq!(get32(&blob, 0), 0x010c);
        assert_eq!(get32(&blob, 24), RESSOURCE_BLOB_BANC);
        assert_eq!(get32(&blob, 28), 1);
        assert_eq!(get32(&blob, 32), 0);
        assert_eq!(get32(&blob, 36), 1);
        assert_eq!(get32(&blob, 48), 4096);
        assert_eq!(get32(&blob, 56), 0x100000);
        assert_eq!(get32(&blob, 64), 4096);
    }
    #[test]
    fn soumission_venus_et_reponse_strictes() {
        let q = soumet_version_venus();
        assert_eq!(get32(&q, 16), CONTEXTE_VENUS);
        assert_eq!(get32(&q, 24) as usize, q.len() - 32);
        assert_eq!(get32(&q, 60), 20);
        assert_eq!(get32(&q, 68), VENUS_ENUMERATE_INSTANCE_VERSION);
        let mut r = [0; 20];
        put32(&mut r, 0, VENUS_ENUMERATE_INSTANCE_VERSION);
        put32(&mut r, 8, 1);
        put32(&mut r, 16, 1 << 22);
        assert_eq!(lit_version_venus(&r), Some(1 << 22));
        for offset in [0, 4, 8, 12, 16] {
            let mut invalide = r;
            put32(&mut invalide, offset, 0xffffffff);
            assert_eq!(lit_version_venus(&invalide), None);
        }
    }
    #[test]
    fn capset_info_strict() {
        let q = get_capset_info(3);
        assert_eq!(get32(&q, 0), CMD_GET_CAPSET_INFO);
        assert_eq!(get32(&q, 24), 3);
        let mut r = [0u8; 40];
        put32(&mut r, 0, RESP_OK_CAPSET_INFO);
        put32(&mut r, 24, CAPSET_VIRGL2);
        put32(&mut r, 28, 5);
        put32(&mut r, 32, 4096);
        assert!(supported(parse_capset_info(&r).unwrap()));
        assert!(parse_capset_info(&r[..39]).is_none());
        put32(&mut r, 28, 0);
        assert!(parse_capset_info(&r).is_none());
    }
    #[test]
    fn context_and_texture_layout() {
        let ctx = ctx_create();
        assert_eq!(get32(&ctx, 0), CMD_CTX_CREATE);
        assert_eq!(get32(&ctx, 16), CONTEXT_ID);
        assert_eq!(get32(&ctx, 24), 18);
        let image = resource_create();
        assert_eq!(get32(&image, 0), CMD_RESOURCE_CREATE_3D);
        assert_eq!(get32(&image, 24), RESOURCE_ID);
        assert_eq!(get32(&image, 40), RESOURCE_DIM);
        assert_eq!(get32(&resource_attach(), 24), RESOURCE_ID);
        assert_eq!(get32(&resource_detach(), 24), RESOURCE_ID);
    }
    #[test]
    fn submissions_reject_bad_buffer_lengths() {
        assert!(submit_header(0).is_none());
        assert!(submit_header(7).is_none());
        assert!(submit_header(2017).is_none());
        let q = submit_header(16).unwrap();
        assert_eq!(get32(&q, 0), CMD_SUBMIT_3D);
        assert_eq!(get32(&q, 16), CONTEXT_ID);
        assert_eq!(get32(&q, 24), 16);
    }
}
