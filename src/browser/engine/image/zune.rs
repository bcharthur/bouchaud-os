//! Replis de décodage image via les crates `zune-jpeg` / `zune-png` (pur Rust,
//! no_std+alloc, fuzz-testées).
//!
//! Les décodeurs maison restent le chemin PRIMAIRE (vérifiés en prod sur cette
//! cible) ; ces replis ne sont tentés que quand le maison renvoie `None`, et
//! couvrent exactement ses manques :
//!   - JPEG **progressif** (SOF2) — le maison ne gère que le baseline SOF0,
//!     or beaucoup de photos du web moderne sont progressives ;
//!   - PNG **entrelacé** (Adam7) — le maison le rejette explicitement.
//!
//! Sortie : pixels `0x00RRGGBB`, alpha composité sur fond blanc, comme tous
//! les décodeurs de `image/` (framebuffer opaque).

use alloc::vec::Vec;
use super::{composite_rgba, Image};

/// Convertit un tampon de canaux 8 bits (1 à 4 canaux) en pixels 0x00RRGGBB.
/// Gris et gris+alpha sont dupliqués sur RGB ; l'alpha est composité sur blanc.
fn channels_to_pix(buf: &[u8], w: usize, h: usize, nch: usize) -> Option<Vec<u32>> {
    if nch == 0 || nch > 4 || buf.len() < w.checked_mul(h)?.checked_mul(nch)? {
        return None;
    }
    let mut pix = Vec::with_capacity(w * h);
    for i in 0..w * h {
        let p = i * nch;
        let (r, g, b, a) = match nch {
            1 => (buf[p], buf[p], buf[p], 255),
            2 => (buf[p], buf[p], buf[p], buf[p + 1]),
            3 => (buf[p], buf[p + 1], buf[p + 2], 255),
            _ => (buf[p], buf[p + 1], buf[p + 2], buf[p + 3]),
        };
        pix.push(composite_rgba(r as u32, g as u32, b as u32, a as u32));
    }
    Some(pix)
}

/// Décode un JPEG via zune-jpeg (baseline ET progressif). `None` si invalide.
pub fn decode_jpeg(data: &[u8]) -> Option<Image> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    let mut dec = zune_jpeg::JpegDecoder::new(ZCursor::new(data));
    let buf = dec.decode().ok()?;
    let info = dec.info()?;
    let (w, h) = (info.width as usize, info.height as usize);
    if w == 0 || h == 0 || w * h > 40_000_000 { return None; }
    let nch = match dec.output_colorspace()? {
        ColorSpace::Luma => 1,
        ColorSpace::LumaA => 2,
        ColorSpace::RGB => 3,
        ColorSpace::RGBA => 4,
        _ => return None,
    };
    let pix = channels_to_pix(&buf, w, h, nch)?;
    Some(Image { w, h, pix })
}

/// Décode un PNG via zune-png (tous types, dont entrelacé Adam7).
/// Les sorties 16 bits sont réduites à 8 bits (octet de poids fort).
pub fn decode_png(data: &[u8]) -> Option<Image> {
    use zune_png::zune_core::colorspace::ColorSpace;
    use zune_png::zune_core::result::DecodingResult;
    let mut dec = zune_png::PngDecoder::new(data);
    let out = dec.decode().ok()?;
    let (w, h) = dec.get_dimensions()?;
    if w == 0 || h == 0 || w * h > 40_000_000 { return None; }
    let nch = match dec.get_colorspace()? {
        ColorSpace::Luma => 1,
        ColorSpace::LumaA => 2,
        ColorSpace::RGB => 3,
        ColorSpace::RGBA => 4,
        _ => return None,
    };
    let buf8: Vec<u8> = match out {
        DecodingResult::U8(v) => v,
        DecodingResult::U16(v) => v.iter().map(|&x| (x >> 8) as u8).collect(),
        _ => return None,
    };
    let pix = channels_to_pix(&buf8, w, h, nch)?;
    Some(Image { w, h, pix })
}
