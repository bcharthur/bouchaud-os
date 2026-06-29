//! WebP (conteneur RIFF/WEBP). Le décodage VP8 (lossy, DCT) et VP8L (lossless,
//! Huffman) n'est pas encore implémenté — on identifie le sous-format pour les
//! diagnostics et on renvoie None (repli propre : le moteur affiche l'`alt`).
//!
//! TODO (phase ultérieure) : VP8L lossless (le plus simple) puis VP8 lossy.

use super::Image;

pub fn decode(data: &[u8]) -> Option<Image> {
    if data.len() < 16 { return None; }
    let kind = &data[12..16];
    let _label = match kind {
        b"VP8 " => "VP8 (lossy)",
        b"VP8L" => "VP8L (lossless)",
        b"VP8X" => "VP8X (extended)",
        _ => "inconnu",
    };
    // Décodage non encore disponible -> repli propre.
    None
}
