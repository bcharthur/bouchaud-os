//! Mise a l'echelle d'images, pure (BOUCHAUD_ATELIER_ECHELLE_V1).
//!
//! Sans framebuffer ni noyau : les tests d'hote l'incluent tels quels
//! (`tools/gui/test_assets_atelier.rs`). Entier uniquement.

extern crate alloc;
use alloc::vec::Vec;

/// Recadrage « cover » centre, bilineaire, de `(sl, sh)` vers `(l, h)`.
/// Arithmetique en virgule fixe 16.16 : pas de flottant par pixel.
pub fn couvre(src: &[u32], sl: usize, sh: usize, l: usize, h: usize) -> Vec<u32> {
    let mut sortie = alloc::vec![0u32; l * h];
    if sl == 0 || sh == 0 || l == 0 || h == 0 || src.len() < sl * sh {
        return sortie;
    }
    // Pas source par pixel ecran (16.16) : le plus petit des deux rapports,
    // pour que l'image COUVRE l'ecran ; l'excedent est coupe au centre.
    let pas_x = ((sl as u64) << 16) / l as u64;
    let pas_y = ((sh as u64) << 16) / h as u64;
    let pas = pas_x.min(pas_y);
    let ox = (((sl as u64) << 16) - pas * l as u64) / 2;
    let oy = (((sh as u64) << 16) - pas * h as u64) / 2;
    for y in 0..h {
        let fy = oy + pas * y as u64;
        let y0 = ((fy >> 16) as usize).min(sh - 1);
        let y1 = (y0 + 1).min(sh - 1);
        let ty = ((fy >> 8) & 0xff) as u32;
        for x in 0..l {
            let fx = ox + pas * x as u64;
            let x0 = ((fx >> 16) as usize).min(sl - 1);
            let x1 = (x0 + 1).min(sl - 1);
            let tx = ((fx >> 8) & 0xff) as u32;
            let haut = mix(src[y0 * sl + x0], src[y0 * sl + x1], tx);
            let bas = mix(src[y1 * sl + x0], src[y1 * sl + x1], tx);
            sortie[y * l + x] = mix(haut, bas, ty) & 0x00ff_ffff;
        }
    }
    sortie
}

/// Melange entier : `t` en 256es (0 = `a`, 256 = `b`).
#[inline(always)]
pub fn mix(a: u32, b: u32, t: u32) -> u32 {
    let t = if t > 256 { 256 } else { t };
    let u = 256 - t;
    let r = (((a >> 16) & 0xff) * u + ((b >> 16) & 0xff) * t) >> 8;
    let g = (((a >> 8) & 0xff) * u + ((b >> 8) & 0xff) * t) >> 8;
    let bl = ((a & 0xff) * u + (b & 0xff) * t) >> 8;
    (r << 16) | (g << 8) | bl
}

