//! BOUCHAUD_ATLAS_POLICE_V1 -- la police d'Atelier pour les applications ring 3.
//!
//! Le bureau dessine son texte avec `fontdue` sur DejaVu Sans. Une application
//! ring 3 (Notes, Calculatrice...) n'a pas de rasteriseur TrueType : elle
//! dessinait en 5x7, et ne ressemblait pas au reste d'Atelier. Le noyau publie
//! donc, une fois, les glyphes qu'il dessine LUI-MEME -- memes couvertures,
//! memes avances, meme ligne de base --, dans des fichiers que toute
//! application projette en lecture (`userland/libs/police/atelier.h`).
//!
//! Format (petit-boutiste), un fichier par taille, `glyphes-<px>.bin` :
//!   en-tete 16 o : "BOGA", version u16 = 1, px u16, ascendante i16,
//!                  nombre u16, reserve u32
//!   entrees 20 o : point de code u32, avance i16, gauche i16, haut i16,
//!                  largeur u16, hauteur u16, reserve u16, decalage u32
//!   couvertures  : largeur x hauteur octets par glyphe, au decalage donne
//!                  (depuis le debut du fichier).
//!
//! Le meme fichier sert le gras : comme `blend_span`, l'application double
//! chaque pixel vers la droite.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

pub const REPERTOIRE: &str = "/usr/share/atelier";
pub const TAILLES: [i32; 4] = [12, 13, 14, 16];

static INSTALLE: AtomicBool = AtomicBool::new(false);

/// Les points de code exportes : ASCII imprimable, Latin-1 (accents
/// francais), et quelques signes courants d'un texte francais.
fn points_de_code() -> impl Iterator<Item = char> {
    (0x20u32..=0x7e)
        .chain(0xa0..=0xff)
        .chain([0x152, 0x153, 0x2019, 0x201c, 0x201d, 0x2026, 0x20ac, 0x2013, 0x2014])
        .filter_map(char::from_u32)
}

/// Serialise l'atlas d'une taille. Pur, hors des polices : rend `None` si un
/// glyphe manque (rien n'est publie a moitie).
pub fn serialise(px: i32, ascendante: i32,
                 glyphe: impl Fn(char) -> Option<(usize, usize, i32, i32, i32, Vec<u8>)>) -> Option<Vec<u8>> {
    let codes: Vec<char> = points_de_code().collect();
    let mut entrees = Vec::with_capacity(codes.len() * 20);
    let mut couvertures = Vec::new();
    let base = 16 + codes.len() * 20;
    for &c in &codes {
        let (w, h, gauche, haut, avance, cov) = glyphe(c)?;
        if cov.len() != w * h || w > u16::MAX as usize || h > u16::MAX as usize {
            return None;
        }
        let decalage = (base + couvertures.len()) as u32;
        entrees.extend_from_slice(&(c as u32).to_le_bytes());
        entrees.extend_from_slice(&(avance as i16).to_le_bytes());
        entrees.extend_from_slice(&(gauche as i16).to_le_bytes());
        entrees.extend_from_slice(&(haut as i16).to_le_bytes());
        entrees.extend_from_slice(&(w as u16).to_le_bytes());
        entrees.extend_from_slice(&(h as u16).to_le_bytes());
        entrees.extend_from_slice(&0u16.to_le_bytes());
        entrees.extend_from_slice(&decalage.to_le_bytes());
        couvertures.extend_from_slice(&cov);
    }
    let mut out = Vec::with_capacity(base + couvertures.len());
    out.extend_from_slice(b"BOGA");
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(px as u16).to_le_bytes());
    out.extend_from_slice(&(ascendante as i16).to_le_bytes());
    out.extend_from_slice(&(codes.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&entrees);
    out.extend_from_slice(&couvertures);
    Some(out)
}

/// Publie les atlas, une fois. Appele avant de lancer une application ring 3
/// du bureau : la rasterisation (quelques centaines de glyphes) ne coute qu'a
/// ceux qui en ont besoin.
pub fn installe() {
    if INSTALLE.swap(true, Ordering::AcqRel) {
        return;
    }
    let dossier = crate::kernel::sysroot::mkdir_public(REPERTOIRE);
    if dossier == 0 {
        return;
    }
    let mut publies = 0;
    for px in TAILLES {
        let Some(asc) = crate::gui::font::ascendante(px) else { break };
        let octets = serialise(px, asc, |c| {
            crate::gui::font::glyphe_exporte(c, px)
                .map(|(w, h, l, t, a, cov)| (w, h, l, t, a, cov.to_vec()))
        });
        if let Some(octets) = octets {
            crate::kernel::sysroot::ecrit_binaire_public(
                dossier, &alloc::format!("glyphes-{}.bin", px), &octets, 0o444);
            publies += 1;
        }
    }
    crate::serial_println!("BOUCHAUD_ATLAS_POLICE tailles={} repertoire={}", publies, REPERTOIRE);
}
