//! P10+.4 (experimental, opt-in): verified virtio-gpu 3D control protocol.
//!
//! Does not draw pixels: this is host-side 3D context + render-target
//! resource management and a trusted-kernel-only command submission ABI.
//! No DRM uAPI, Mesa, Skia GPU or WebGL. The 2D default remains unchanged.
//!
//! Layouts match include/uapi/linux/virtio_gpu.h (little-endian).

pub const F_VIRGL: u64 = 1;
pub const F_VERSION_1: u64 = 1 << 32;
pub const CMD_GET_CAPSET_INFO: u32 = 0x0108;
pub const CMD_CTX_CREATE: u32 = 0x0200;
pub const CMD_CTX_DESTROY: u32 = 0x0201;
pub const CMD_CTX_ATTACH_RESOURCE: u32 = 0x0202;
pub const CMD_CTX_DETACH_RESOURCE: u32 = 0x0203;
pub const CMD_RESOURCE_CREATE_3D: u32 = 0x0204;
pub const CMD_SUBMIT_3D: u32 = 0x0207;
pub const RESP_OK_CAPSET_INFO: u32 = 0x1102;
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
    if info.id == 0 || info.version == 0 || info.size == 0 { return None; }
    Some(info)
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
    fn negotiation_never_enables_3d_by_default() {
        assert_eq!(negotiated_features(F_VERSION_1 | F_VIRGL, false), F_VERSION_1);
        assert_eq!(negotiated_features(F_VERSION_1, true), F_VERSION_1);
        assert_eq!(negotiated_features(F_VERSION_1 | F_VIRGL, true), F_VERSION_1 | F_VIRGL);
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
