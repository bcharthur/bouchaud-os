//! GPU Core — contrat d'affichage et accounting graphique.
//!
//! Bouchaud OS ne pretend pas encore fournir un pilote 3D materiel universel.
//! Cette couche separe en revanche des maintenant le **GPU core** du backend
//! concret. Le backend actif est BGA/LFB (QEMU/Bochs) ; virtio-gpu pourra
//! implementer le meme contrat sans modifier le gestionnaire de fenetres.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    None,
    BgaLinearFramebuffer,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::None => "none",
            Backend::BgaLinearFramebuffer => "bga-lfb",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GpuStats {
    pub backend: Backend,
    pub active: bool,
    pub width: usize,
    pub height: usize,
    pub bpp: usize,
    pub scanout_phys: u64,
    pub scanout_bytes: u64,
    pub backbuffer_bytes: u64,
    pub presents: u64,
    pub bytes_presented: u64,
    pub userland_handoff: bool,
}

static mut BACKEND: Backend = Backend::None;
static mut ACTIVE: bool = false;
static mut WIDTH: usize = 0;
static mut HEIGHT: usize = 0;
static mut BPP: usize = 0;
static mut SCANOUT_PHYS: u64 = 0;
static mut PRESENTS: u64 = 0;
static mut BYTES_PRESENTED: u64 = 0;
static mut USERLAND_HANDOFF: bool = false;

pub fn activate_bga(width: usize, height: usize, bpp: usize, scanout_phys: u64) {
    unsafe {
        BACKEND = Backend::BgaLinearFramebuffer;
        ACTIVE = true;
        WIDTH = width;
        HEIGHT = height;
        BPP = bpp;
        SCANOUT_PHYS = scanout_phys;
        USERLAND_HANDOFF = false;
    }
}

pub fn deactivate() {
    unsafe {
        BACKEND = Backend::None;
        ACTIVE = false;
        WIDTH = 0;
        HEIGHT = 0;
        BPP = 0;
        SCANOUT_PHYS = 0;
        USERLAND_HANDOFF = false;
    }
}

pub fn note_present(bytes: usize) {
    unsafe {
        PRESENTS = PRESENTS.wrapping_add(1);
        BYTES_PRESENTED = BYTES_PRESENTED.wrapping_add(bytes as u64);
    }
}

pub fn note_handoff(active: bool) {
    unsafe { USERLAND_HANDOFF = active; }
}

pub fn stats() -> GpuStats {
    unsafe {
        let bytes_per_pixel = (BPP / 8) as u64;
        let scanout_bytes = WIDTH as u64 * HEIGHT as u64 * bytes_per_pixel;
        GpuStats {
            backend: BACKEND,
            active: ACTIVE,
            width: WIDTH,
            height: HEIGHT,
            bpp: BPP,
            scanout_phys: SCANOUT_PHYS,
            scanout_bytes,
            // Le backend BGA garde actuellement un double-buffer CPU de meme
            // taille que le scanout.
            backbuffer_bytes: if ACTIVE { scanout_bytes } else { 0 },
            presents: PRESENTS,
            bytes_presented: BYTES_PRESENTED,
            userland_handoff: USERLAND_HANDOFF,
        }
    }
}

pub fn print_info() {
    let s = stats();
    crate::println!("GPU Core v1");
    crate::println!("  backend : {}", s.backend.label());
    crate::println!("  active  : {}", s.active);
    crate::println!("  mode    : {}x{}x{}", s.width, s.height, s.bpp);
    crate::println!("  scanout : phys={:#x}, {} KiB", s.scanout_phys, s.scanout_bytes / 1024);
    crate::println!("  backbuf : {} KiB", s.backbuffer_bytes / 1024);
    crate::println!("  presents: {} ({} MiB copies)", s.presents, s.bytes_presented / (1024 * 1024));
    crate::println!("  handoff : {}", s.userland_handoff);
    crate::println!(
        "  acceleration: CPU raster + linear scanout; virtio-gpu/3D backend = prochaine implementation"
    );
}

pub fn self_test() -> bool {
    let s = stats();
    if !s.active {
        return s.backend == Backend::None
            && s.width == 0
            && s.height == 0
            && s.scanout_bytes == 0;
    }

    s.backend != Backend::None
        && s.width > 0
        && s.height > 0
        && s.bpp == 32
        && s.scanout_phys != 0
        && s.scanout_bytes == s.backbuffer_bytes
}
